//! The server side of the handshake, checked against real servers: fed what the recorder sent,
//! configured as the recorded server was, it must send what that server sent.

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::server::{SecurityType, ServerHandshake, Step};
use justvnc_proto::vnc_auth::{self, CHALLENGE_LEN};
use justvnc_proto::{HandshakeError, Limits, Password, ProtocolVersion, ServerInit};

macro_rules! fixture {
    ($name:literal) => {
        (
            include_bytes!(concat!(
                "../../justvnc-client/tests/fixtures/",
                $name,
                ".server.bin"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "../../justvnc-client/tests/fixtures/",
                $name,
                ".client.bin"
            ))
            .as_slice(),
        )
    };
}

const PASSWORD: &[u8] = b"justvnc-secret";

/// What a recorded server sent: its version, security types, challenge and ServerInit, and how
/// many bytes its handshake took.
struct Recorded {
    version: ProtocolVersion,
    security: Vec<SecurityType>,
    challenge: [u8; CHALLENGE_LEN],
    init: Option<ServerInit>,
    len: usize,
}

fn security_type(value: u8) -> SecurityType {
    match value {
        1 => SecurityType::None,
        2 => SecurityType::VncAuth,
        _ => panic!("security type {value}"),
    }
}

fn recorded(server: &[u8]) -> Recorded {
    let version = ProtocolVersion::parse(server[..12].try_into().unwrap()).unwrap();
    let (security, mut at) = if version == ProtocolVersion::V3_3 {
        (vec![security_type(server[15])], 16)
    } else {
        let count = usize::from(server[12]);
        let types = server[13..13 + count]
            .iter()
            .map(|&t| security_type(t))
            .collect();
        (types, 13 + count)
    };
    let mut challenge = [0; CHALLENGE_LEN];
    if security.contains(&SecurityType::VncAuth) {
        challenge.copy_from_slice(&server[at..at + CHALLENGE_LEN]);
    }
    // The ServerInit, read by the client role, which is tested against the same recordings.
    let mut client = ClientHandshake::new(false, Some(Password::new(PASSWORD)), Limits::default());
    let init = client.receive(server).ok().flatten();
    at = server.len() - client.take_remaining().len();
    Recorded {
        version,
        security,
        challenge,
        init,
        len: at,
    }
}

/// Runs the server side on the recorder's bytes, `chunk` bytes at a time, deciding VNC
/// authentication with `PASSWORD`. Returns what it sent and how it ended.
fn serve(
    rec: &Recorded,
    client: &[u8],
    chunk: usize,
) -> (Vec<u8>, Result<Option<bool>, HandshakeError>) {
    let mut handshake = ServerHandshake::new(rec.version, rec.security.clone(), rec.challenge);
    let mut sent = handshake.take_transmit();
    let mut shared = None;
    for piece in client.chunks(chunk) {
        let mut step = handshake.receive(piece);
        loop {
            match step {
                Ok(Some(Step::Authenticate {
                    challenge,
                    response,
                })) => {
                    if vnc_auth::verify(PASSWORD, &challenge, &response) {
                        handshake.accept();
                    } else {
                        handshake.reject("Authentication failure");
                    }
                }
                Ok(Some(Step::ClientInit { shared: flag })) => {
                    handshake.server_init(rec.init.as_ref().unwrap());
                    shared = Some(flag);
                }
                Ok(None) => break,
                Err(e) => {
                    sent.extend(handshake.take_transmit());
                    return (sent, Err(e));
                }
            }
            step = handshake.receive(&[]);
        }
        sent.extend(handshake.take_transmit());
    }
    (sent, Ok(shared))
}

/// The server's bytes, with the true-colour flag of ServerInit written as 1: x11vnc sends 0xff,
/// and a flag is any non-zero value.
fn normalised(server: &[u8], rec: &Recorded) -> Vec<u8> {
    let mut expected = server[..rec.len].to_vec();
    if let Some(init) = &rec.init {
        let start = rec.len - 24 - init.name.as_bytes().len();
        expected[start + 4 + 3] = u8::from(expected[start + 4 + 3] != 0);
    }
    expected
}

fn check(server: &[u8], client: &[u8], ends: Result<Option<bool>, HandshakeError>) {
    let rec = recorded(server);
    for chunk in [client.len(), 1] {
        let (sent, result) = serve(&rec, client, chunk);
        assert_eq!(sent, normalised(server, &rec), "chunk {chunk}");
        assert_eq!(result, ends, "chunk {chunk}");
    }
}

#[test]
fn serves_3_8_with_none_as_tigervnc_does() {
    let (server, client) = fixture!("tigervnc-3.8-none-retro-update");
    // The recorder's handshake: version, security type, then ClientInit asking to share.
    check(server, &client[..14], Ok(Some(true)));
}

#[test]
fn serves_3_8_with_vnc_authentication_as_tigervnc_does() {
    let (server, client) = fixture!("tigervnc-3.8-vncauth");
    check(server, client, Ok(Some(true)));
}

#[test]
fn refuses_a_wrong_password_with_tigervncs_3_8_reason() {
    let (server, client) = fixture!("tigervnc-3.8-vncauth-wrong-password");
    check(
        server,
        client,
        Err(HandshakeError::SecurityFailed {
            reason: Some("Authentication failure".into()),
        }),
    );
}

#[test]
fn serves_3_8_with_vnc_authentication_as_x11vnc_does() {
    let (server, client) = fixture!("x11vnc-3.8-vncauth-short-password");
    let rec = recorded(server);
    // The recorder used a password x11vnc accepted; this test only knows PASSWORD, so it
    // checks the bytes up to the challenge and that the response is offered for a decision.
    let mut handshake = ServerHandshake::new(rec.version, rec.security.clone(), rec.challenge);
    let mut sent = handshake.take_transmit();
    let step = handshake.receive(client).unwrap();
    sent.extend(handshake.take_transmit());
    assert_eq!(sent, server[..12 + 2 + CHALLENGE_LEN]);
    let Some(Step::Authenticate {
        challenge,
        response,
    }) = step
    else {
        panic!("{step:?}");
    };
    assert_eq!(challenge, rec.challenge);
    assert_eq!(response[..], client[13..13 + CHALLENGE_LEN]);
}

#[test]
fn serves_3_7_with_none_as_x11vnc_does() {
    let (server, client) = fixture!("x11vnc-3.7-none");
    check(server, client, Ok(Some(true)));
}

#[test]
fn refuses_a_wrong_password_under_3_7_without_a_reason() {
    let (server, client) = fixture!("x11vnc-3.7-vncauth-wrong-password");
    check(
        server,
        client,
        Err(HandshakeError::SecurityFailed { reason: None }),
    );
}

#[test]
fn serves_3_3_with_none_as_x11vnc_does() {
    let (server, client) = fixture!("x11vnc-3.3-none");
    check(server, client, Ok(Some(true)));
}

#[test]
fn serves_3_3_with_vnc_authentication_as_x11vnc_does() {
    let (server, client) = fixture!("x11vnc-3.3-vncauth");
    check(server, client, Ok(Some(true)));
}

fn version_answer(server: ProtocolVersion, client: &[u8; 12]) -> Vec<u8> {
    let mut handshake = ServerHandshake::new(
        server,
        vec![SecurityType::VncAuth, SecurityType::None],
        [7; CHALLENGE_LEN],
    );
    handshake.take_transmit();
    handshake.receive(client).unwrap();
    handshake.take_transmit()
}

#[test]
fn speaks_3_3_to_a_client_that_reports_3_5() {
    // rfbproto: 3.5 was wrongly reported by some clients and is taken as 3.3, where the server
    // picks the security type: its first, here VNC authentication with its challenge.
    let answer = version_answer(ProtocolVersion::V3_8, b"RFB 003.005\n");

    assert_eq!(answer, [&[0, 0, 0, 2][..], &[7; CHALLENGE_LEN]].concat());
}

#[test]
fn offers_its_security_types_in_order_under_3_7_and_later() {
    for client in [b"RFB 003.007\n", b"RFB 003.008\n", b"RFB 003.889\n"] {
        assert_eq!(version_answer(ProtocolVersion::V3_8, client), [2, 2, 1]);
    }
}

#[test]
fn speaks_no_higher_than_the_version_it_offered() {
    // A client asking for 3.8 from a 3.3 server still gets the server-chosen security type.
    let answer = version_answer(ProtocolVersion::V3_3, b"RFB 003.008\n");

    assert_eq!(answer[..4], [0, 0, 0, 2]);
}

#[test]
fn refuses_a_client_version_that_is_not_rfb_3() {
    let mut handshake =
        ServerHandshake::new(ProtocolVersion::V3_8, vec![SecurityType::None], [0; 16]);

    assert_eq!(
        handshake.receive(b"RFB 004.000\n"),
        Err(HandshakeError::UnsupportedClientVersion {
            client: ProtocolVersion { major: 4, minor: 0 }
        })
    );
    let mut handshake =
        ServerHandshake::new(ProtocolVersion::V3_8, vec![SecurityType::None], [0; 16]);
    assert_eq!(
        handshake.receive(b"HTTP/1.1 200"),
        Err(HandshakeError::MalformedVersion)
    );
}

#[test]
fn refuses_a_security_type_it_did_not_offer() {
    let mut handshake =
        ServerHandshake::new(ProtocolVersion::V3_8, vec![SecurityType::VncAuth], [0; 16]);

    let result = handshake.receive(b"RFB 003.008\n\x01");

    assert_eq!(
        result,
        Err(HandshakeError::UnexpectedSecurityType { value: 1 })
    );
}
