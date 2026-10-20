use justvnc_client::{ClientConfig, ClientSession, Event, HandshakeError, Limits, PixelFormat};

const TIGERVNC_NONE: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none.server.bin");

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
fn answers_a_nonstandard_server_version_with_3_3() {
    // RFC 6143 7.1.1: versions other than 3.3, 3.7 and 3.8 are interpreted as 3.3.
    for version in [
        &b"RFB 003.889
"[..],
        b"RFB 004.001
",
        b"RFB 003.005
",
    ] {
        let mut session = ClientSession::new(ClientConfig::default());

        session.receive(version).unwrap();

        assert_eq!(
            drain_transmit(&mut session),
            b"RFB 003.003
"
        );
    }
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

const TIGHTVNC_REFUSED: &[u8] = include_bytes!("fixtures/tightvnc-3.8-loopback-refused.server.bin");

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
fn asks_for_a_password_when_the_server_offers_only_vnc_authentication() {
    let mut session = ClientSession::new(ClientConfig::default());

    let result = session.receive(&[SERVER_VERSION_3_8, &[1, 2]].concat());

    assert_eq!(result, Err(HandshakeError::PasswordRequired));
    assert_eq!(
        drain_transmit(&mut session),
        b"RFB 003.008
"
    );
}

#[test]
fn refuses_when_the_server_offers_no_security_type_it_accepts() {
    let mut session = ClientSession::new(with_password("justvnc-secret"));

    // Two security types offered: 16 (Tight) and 19 (VeNCrypt).
    let result = session.receive(&[SERVER_VERSION_3_8, &[2, 16, 19]].concat());

    assert_eq!(
        result,
        Err(HandshakeError::NoSupportedSecurityType {
            offered: vec![16, 19]
        })
    );
    assert_eq!(
        drain_transmit(&mut session),
        b"RFB 003.008
"
    );
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
            reason: Some("denied".into())
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

/// A recorded handshake: what the server sent and what a client the server accepted sent back.
struct Recording {
    server: &'static [u8],
    client: &'static [u8],
}

macro_rules! recording {
    ($name:literal) => {
        Recording {
            server: include_bytes!(concat!("fixtures/", $name, ".server.bin")),
            client: include_bytes!(concat!("fixtures/", $name, ".client.bin")),
        }
    };
}

fn with_password(password: &str) -> ClientConfig {
    ClientConfig {
        password: Some(password.into()),
        ..ClientConfig::default()
    }
}

#[test]
fn authenticates_to_tigervnc_with_vnc_authentication() {
    let rec = recording!("tigervnc-3.8-vncauth");
    let mut session = ClientSession::new(with_password("justvnc-secret"));

    session.receive(rec.server).unwrap();

    assert_eq!(drain_transmit(&mut session), rec.client);
    let Some(Event::Connected(desktop)) = session.poll_event() else {
        panic!("expected Connected");
    };
    assert_eq!(desktop.name.as_bytes(), b"justvnc-test");
}

#[test]
fn pads_a_short_password_with_zeroes() {
    let rec = recording!("x11vnc-3.8-vncauth-short-password");
    let mut session = ClientSession::new(with_password("abc"));

    session.receive(rec.server).unwrap();

    assert_eq!(drain_transmit(&mut session), rec.client);
    assert!(matches!(session.poll_event(), Some(Event::Connected(_))));
}

#[test]
fn reports_tigervnc_rejecting_a_wrong_password_with_its_reason() {
    let rec = recording!("tigervnc-3.8-vncauth-wrong-password");
    let mut session = ClientSession::new(with_password("wrong-pw"));

    let result = session.receive(rec.server);

    assert_eq!(
        result,
        Err(HandshakeError::SecurityFailed {
            reason: Some("Authentication failure".into())
        })
    );
    assert_eq!(drain_transmit(&mut session), rec.client);
}

fn assert_connects_to_x11vnc(rec: Recording, config: ClientConfig) {
    let mut session = ClientSession::new(config);

    session.receive(rec.server).unwrap();

    assert_eq!(drain_transmit(&mut session), rec.client);
    let Some(Event::Connected(desktop)) = session.poll_event() else {
        panic!("expected Connected");
    };
    assert_eq!((desktop.width, desktop.height), (800, 600));
    assert_eq!(desktop.name.as_bytes(), b"justvnc-x11vnc");
    // x11vnc sends 0xff for true-colour-flag; any non-zero value means true.
    assert!(desktop.pixel_format.true_colour);
}

#[test]
fn connects_to_a_3_3_server_without_authentication() {
    assert_connects_to_x11vnc(recording!("x11vnc-3.3-none"), ClientConfig::default());
}

#[test]
fn authenticates_to_a_3_3_server_that_chose_vnc_authentication() {
    assert_connects_to_x11vnc(
        recording!("x11vnc-3.3-vncauth"),
        with_password("justvnc-secret"),
    );
}

#[test]
fn connects_to_a_3_7_server_without_authentication() {
    assert_connects_to_x11vnc(recording!("x11vnc-3.7-none"), ClientConfig::default());
}

#[test]
fn reports_a_3_7_authentication_failure_without_a_reason() {
    let rec = recording!("x11vnc-3.7-vncauth-wrong-password");
    let mut session = ClientSession::new(with_password("wrong-pw"));

    let result = session.receive(rec.server);

    assert_eq!(result, Err(HandshakeError::SecurityFailed { reason: None }));
    assert_eq!(drain_transmit(&mut session), rec.client);
}

const SERVER_VERSION_3_3: &[u8] = b"RFB 003.003
";

#[test]
fn reports_the_reason_when_a_3_3_server_refuses_the_connection() {
    let mut session = ClientSession::new(ClientConfig::default());

    let mut bytes = [SERVER_VERSION_3_3, &[0, 0, 0, 0], &[0, 0, 0, 4]].concat();
    bytes.extend_from_slice(b"busy");
    let result = session.receive(&bytes);

    assert_eq!(
        result,
        Err(HandshakeError::ConnectionRefused {
            reason: "busy".into()
        })
    );
}

#[test]
fn rejects_a_security_type_a_3_3_server_may_not_choose() {
    let mut session = ClientSession::new(with_password("justvnc-secret"));

    let result = session.receive(&[SERVER_VERSION_3_3, &[0, 0, 0, 16]].concat());

    assert_eq!(
        result,
        Err(HandshakeError::UnexpectedSecurityType { value: 16 })
    );
}

#[test]
fn asks_for_a_password_when_a_3_3_server_chooses_vnc_authentication() {
    let mut session = ClientSession::new(ClientConfig::default());

    let result = session.receive(&[SERVER_VERSION_3_3, &[0, 0, 0, 2]].concat());

    assert_eq!(result, Err(HandshakeError::PasswordRequired));
}

#[test]
fn picks_the_first_usable_security_type_in_the_servers_order() {
    for (offered, expected) in [([2, 1], 2), ([1, 2], 1)] {
        let mut session = ClientSession::new(with_password("justvnc-secret"));

        session
            .receive(&[SERVER_VERSION_3_8, &[2], &offered].concat())
            .unwrap();

        assert_eq!(
            drain_transmit(&mut session),
            [b"RFB 003.008\n".as_slice(), &[expected]].concat()
        );
    }
}

#[test]
fn keeps_the_password_out_of_debug_output() {
    let config = with_password("justvnc-secret");
    let mut session = ClientSession::new(config.clone());
    let rec = recording!("tigervnc-3.8-vncauth");
    session.receive(&rec.server[..20]).unwrap();

    for debug in [format!("{config:?}"), format!("{session:?}")] {
        assert!(!debug.contains("justvnc-secret"), "{debug}");
        // The bytes of the password, as a derived Debug of a Vec<u8> would print them.
        assert!(!debug.contains("106, 117, 115, 116"), "{debug}");
    }
}
