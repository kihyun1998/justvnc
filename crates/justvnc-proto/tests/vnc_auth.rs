use justvnc_proto::vnc_auth::{CHALLENGE_LEN, verify};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../justvnc-client/tests/fixtures/"
);

/// The challenge TigerVNC sent and the response the recording client sent back, in a 3.8
/// handshake where the client picked security type 2.
fn challenge_and_response(name: &str) -> ([u8; CHALLENGE_LEN], [u8; CHALLENGE_LEN]) {
    let server = std::fs::read(format!("{FIXTURES}{name}.server.bin")).unwrap();
    let client = std::fs::read(format!("{FIXTURES}{name}.client.bin")).unwrap();
    // Server: version (12), count (1), types (1), challenge. Client: version (12), type (1), response.
    (
        server[14..30].try_into().unwrap(),
        client[13..29].try_into().unwrap(),
    )
}

#[test]
fn accepts_the_response_tigervnc_accepted() {
    let (challenge, response) = challenge_and_response("tigervnc-3.8-vncauth");

    assert!(verify(b"justvnc-secret", &challenge, &response));
}

#[test]
fn rejects_the_response_tigervnc_rejected() {
    let (challenge, response) = challenge_and_response("tigervnc-3.8-vncauth-wrong-password");

    assert!(!verify(b"justvnc-secret", &challenge, &response));
}
