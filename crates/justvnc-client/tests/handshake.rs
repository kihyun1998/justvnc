use justvnc_client::{
    ClientConfig, ClientSession, Event, HandshakeError, Limits, PixelFormat, ProtocolVersion,
};

const TIGERVNC_NONE: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none.bin");

fn drain_transmit(session: &mut ClientSession) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(bytes) = session.poll_transmit() {
        out.extend_from_slice(&bytes);
    }
    out
}

#[test]
fn completes_a_3_8_none_handshake_with_tigervnc() {
    let mut session = ClientSession::new(ClientConfig::default());

    session.receive(TIGERVNC_NONE).unwrap();

    // Version, security type None, ClientInit with the shared flag set.
    assert_eq!(drain_transmit(&mut session), b"RFB 003.008\n\x01\x01");
    let Some(Event::Connected(desktop)) = session.poll_event() else {
        panic!("expected Connected");
    };
    assert_eq!((desktop.width, desktop.height), (1024, 768));
    assert_eq!(desktop.name.as_bytes(), b"justvnc-test");
    assert_eq!(
        desktop.pixel_format,
        PixelFormat {
            bits_per_pixel: 32,
            depth: 24,
            big_endian: false,
            true_colour: true,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
        }
    );
    assert!(session.poll_event().is_none());
}

#[test]
fn treats_a_nonstandard_server_version_as_3_3_and_refuses_it() {
    // RFC 6143 7.1.1: versions other than 3.3, 3.7 and 3.8 are interpreted as 3.3.
    let mut session = ClientSession::new(ClientConfig::default());

    let result = session.receive(b"RFB 003.889\n");

    assert_eq!(
        result,
        Err(HandshakeError::UnsupportedVersion {
            server: ProtocolVersion {
                major: 3,
                minor: 889
            }
        })
    );
    assert_eq!(session.poll_transmit(), None);
}

#[test]
fn gives_the_same_result_when_bytes_arrive_one_at_a_time() {
    let mut session = ClientSession::new(ClientConfig::default());

    for byte in TIGERVNC_NONE {
        session.receive(std::slice::from_ref(byte)).unwrap();
    }

    assert_eq!(drain_transmit(&mut session), b"RFB 003.008\n\x01\x01");
    let Some(Event::Connected(desktop)) = session.poll_event() else {
        panic!("expected Connected");
    };
    assert_eq!((desktop.width, desktop.height), (1024, 768));
    assert_eq!(desktop.name.as_bytes(), b"justvnc-test");
}

const TIGHTVNC_REFUSED: &[u8] = include_bytes!("fixtures/tightvnc-3.8-loopback-refused.bin");

#[test]
fn reports_the_reason_when_tightvnc_refuses_the_connection() {
    let mut session = ClientSession::new(ClientConfig::default());

    let result = session.receive(TIGHTVNC_REFUSED);

    assert_eq!(
        result,
        Err(HandshakeError::ConnectionRefused {
            reason: "Sorry, loopback connections are not enabled".into()
        })
    );
    assert!(session.poll_event().is_none());
}

const SERVER_VERSION_3_8: &[u8] = b"RFB 003.008\n";

#[test]
fn refuses_when_the_server_offers_no_security_type_it_accepts() {
    let mut session = ClientSession::new(ClientConfig::default());

    // One security type offered: 2, VNC authentication.
    let result = session.receive(&[SERVER_VERSION_3_8, &[1, 2]].concat());

    assert_eq!(
        result,
        Err(HandshakeError::NoSupportedSecurityType { offered: vec![2] })
    );
    assert_eq!(drain_transmit(&mut session), b"RFB 003.008\n");
}

#[test]
fn reports_the_reason_when_security_fails() {
    let mut session = ClientSession::new(ClientConfig::default());

    let mut bytes = [SERVER_VERSION_3_8, &[1, 1], &[0, 0, 0, 1], &[0, 0, 0, 6]].concat();
    bytes.extend_from_slice(b"denied");
    let result = session.receive(&bytes);

    assert_eq!(
        result,
        Err(HandshakeError::SecurityFailed {
            reason: "denied".into()
        })
    );
    assert_eq!(drain_transmit(&mut session), b"RFB 003.008\n\x01");
}

#[test]
fn rejects_a_desktop_name_longer_than_the_limit_before_it_arrives() {
    let config = ClientConfig {
        limits: Limits {
            max_name_length: 8,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = ClientSession::new(config);

    // ServerInit header announcing a 9-byte name; the name itself never arrives.
    let header = &TIGERVNC_NONE[..TIGERVNC_NONE.len() - 12 - 4];
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&9u32.to_be_bytes());
    let result = session.receive(&bytes);

    assert_eq!(
        result,
        Err(HandshakeError::LimitExceeded {
            field: "name-length",
            length: 9,
            max: 8
        })
    );
}

#[test]
fn rejects_a_failure_reason_longer_than_the_limit_before_it_arrives() {
    let config = ClientConfig {
        limits: Limits {
            max_reason_length: 8,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = ClientSession::new(config);

    let result = session.receive(&[SERVER_VERSION_3_8, &[0], &u32::MAX.to_be_bytes()].concat());

    assert_eq!(
        result,
        Err(HandshakeError::LimitExceeded {
            field: "reason-length",
            length: u32::MAX,
            max: 8
        })
    );
}

#[test]
fn asks_for_exclusive_access_when_not_shared() {
    let mut session = ClientSession::new(ClientConfig {
        shared: false,
        ..ClientConfig::default()
    });

    session.receive(TIGERVNC_NONE).unwrap();

    assert_eq!(drain_transmit(&mut session), b"RFB 003.008\n\x01\x00");
}
