//! The Proxy fed recorded bytes on both legs: a real Viewer's on the viewer leg, a real
//! server's on the target leg.

use justvnc_proto::client_message;
use justvnc_proto::server::SecurityType;
use justvnc_proto::vnc_auth::{self, CHALLENGE_LEN};
use justvnc_proto::{HandshakeError, Password, PixelFormat, ProtocolVersion, Rect};
use justvnc_proxy::{Error, Event, ProxyConfig, ProxySession, narrow_encodings};

macro_rules! client_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-client/tests/fixtures/", $name)).as_slice()
    };
}
macro_rules! server_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-server/tests/fixtures/", $name)).as_slice()
    };
}

/// TightVNC Viewer 2.8.85 against the `pattern` example, password "justvnc".
const VIEWER: &[u8] = server_fixture!("tightvnc-viewer-3.8-vncauth.viewer.bin");
const VIEWER_HANDSHAKE: &[u8] = server_fixture!("tightvnc-viewer-3.8-vncauth.server-handshake.bin");
/// TigerVNC's side of a session with updates holding CopyRect (3.8, None).
const TARGET: &[u8] = client_fixture!("tigervnc-3.8-none-copyrect.server.bin");
const TARGET_CLIENT: &[u8] = client_fixture!("tigervnc-3.8-none-copyrect.client.bin");

/// Where the server's handshake ends in a 3.8 recording with one security type: after
/// ServerInit and its name.
fn server_init_end(server: &[u8]) -> usize {
    let mut at = 12 + 1 + usize::from(server[12]);
    if server[13] == 2 {
        at += CHALLENGE_LEN;
    }
    at += 4;
    at + 24 + u32::from_be_bytes(server[at + 20..at + 24].try_into().unwrap()) as usize
}

/// A Proxy and the bytes it sent each way.
struct Run {
    proxy: ProxySession,
    to_viewer: Vec<u8>,
    to_target: Vec<u8>,
    events: Vec<Event>,
    error: Option<Error>,
}

impl Run {
    fn new(proxy: ProxySession) -> Self {
        Self {
            proxy,
            to_viewer: Vec::new(),
            to_target: Vec::new(),
            events: Vec::new(),
            error: None,
        }
    }

    fn drain(&mut self) {
        while let Some(bytes) = self.proxy.poll_viewer() {
            self.to_viewer.extend(bytes);
        }
        while let Some(bytes) = self.proxy.poll_target() {
            self.to_target.extend(bytes);
        }
        while let Some(event) = self.proxy.poll_event() {
            self.events.push(event);
        }
    }

    fn viewer(&mut self, bytes: &[u8]) {
        if let Err(e) = self.proxy.receive_viewer(bytes) {
            self.error.get_or_insert(e);
        }
        self.drain();
    }

    fn target(&mut self, bytes: &[u8]) {
        if let Err(e) = self.proxy.receive_target(bytes) {
            self.error.get_or_insert(e);
        }
        self.drain();
    }
}

fn vncauth_config() -> ProxyConfig {
    ProxyConfig {
        security: vec![SecurityType::VncAuth],
        ..ProxyConfig::default()
    }
}

/// The Viewer through VNC authentication with "justvnc", up to the Proxy asking for a Target.
fn authenticated(viewer: &[u8]) -> Run {
    let challenge = VIEWER_HANDSHAKE[14..30].try_into().unwrap();
    let mut run = Run::new(ProxySession::with_challenge(vncauth_config(), challenge));
    run.viewer(viewer);
    let Some(Event::Authenticate {
        challenge,
        response,
    }) = run.events.pop()
    else {
        panic!("{:?}", run.events);
    };
    assert!(vnc_auth::verify(b"justvnc", &challenge, &response));
    run.proxy.accept();
    run.drain();
    assert_eq!(run.events.pop(), Some(Event::TargetNeeded));
    run
}

#[test]
fn relays_tightvnc_viewer_to_tigervnc() {
    let mut run = authenticated(VIEWER);
    let before_target = run.to_viewer.clone();
    run.proxy.connect_target(None);
    run.target(TARGET);

    let init_end = server_init_end(TARGET);
    // The Viewer: the Proxy's handshake, held until the Target answered, then the Target's own
    // ServerInit and every message after it, byte for byte.
    assert_eq!(before_target, VIEWER_HANDSHAKE[..12 + 2 + 16]);
    assert_eq!(
        run.to_viewer,
        [
            &VIEWER_HANDSHAKE[..12 + 2 + 16],
            &[0, 0, 0, 0],
            &TARGET[12 + 2 + 4..]
        ]
        .concat()
    );
    // The Target: the handshake the recorder gave TigerVNC, then the Viewer's messages with its
    // SetEncodings narrowed. The Viewer's handshake is 30 bytes; its SetEncodings follows.
    let set_encodings_end = 30 + 4 + 4 * usize::from(VIEWER[33]);
    assert_eq!(run.to_target[..14], TARGET_CLIENT[..14]);
    assert_eq!(
        run.to_target[14..],
        [
            &client_message::set_encodings(&[7, 1, 16, 0, -26, -223, -224])[..],
            &VIEWER[set_encodings_end..]
        ]
        .concat()
    );
    assert_eq!(run.error, None);
    let Some(Event::Connected { desktop }) = run.events.first() else {
        panic!("{:?}", run.events);
    };
    assert_eq!(desktop.to_bytes(), TARGET[12 + 2 + 4..init_end]);
}

#[test]
fn narrows_set_encodings_to_what_it_can_frame() {
    let tightvnc_viewer = [7, 1, 16, 5, 2, 0, -26, -223, -224, -232, -239];

    assert_eq!(
        narrow_encodings(&tightvnc_viewer),
        [7, 1, 16, 0, -26, -223, -224]
    );
    // Tight's quality (-32 to -23) and compression (-256 to -247) levels change only what is
    // inside a Tight rectangle, which still frames; the rest are dropped.
    let levels = [-32, -23, -256, -247, -22, -33, -246, -257, -308, -309];
    assert_eq!(narrow_encodings(&levels), [-32, -23, -256, -247]);
}

#[test]
fn offers_the_viewer_only_the_security_types_allowed() {
    for (security, offered) in [
        (vec![SecurityType::None], vec![1, 1]),
        (vec![SecurityType::VncAuth], vec![1, 2]),
        (
            vec![SecurityType::VncAuth, SecurityType::None],
            vec![2, 2, 1],
        ),
    ] {
        let config = ProxyConfig {
            security,
            ..ProxyConfig::default()
        };
        let mut run = Run::new(ProxySession::with_challenge(config, [0; 16]));
        run.viewer(b"RFB 003.008\n");

        assert_eq!(run.to_viewer[12..], offered);
    }
}

#[test]
fn asks_for_the_target_only_after_the_viewer_authenticated() {
    let challenge = VIEWER_HANDSHAKE[14..30].try_into().unwrap();
    let mut run = Run::new(ProxySession::with_challenge(vncauth_config(), challenge));
    run.viewer(VIEWER);

    assert!(matches!(run.events[..], [Event::Authenticate { .. }]));
    run.proxy.reject("Authentication failure");
    run.drain();

    assert!(!run.events.contains(&Event::TargetNeeded));
    assert_eq!(
        run.to_viewer[12 + 2 + 16..],
        [&[0, 0, 0, 1, 0, 0, 0, 22][..], b"Authentication failure"].concat()
    );
}

#[test]
fn injects_the_targets_password_and_never_shows_it_to_the_viewer() {
    let target = client_fixture!("tigervnc-3.8-vncauth.server.bin");
    let recorder = client_fixture!("tigervnc-3.8-vncauth.client.bin");
    let mut run = authenticated(VIEWER);
    run.proxy
        .connect_target(Some(Password::new("justvnc-secret")));
    run.target(target);

    // What the recorder, keying DES with pycryptodome, sent TigerVNC, which accepted it.
    assert_eq!(run.to_target[..recorder.len()], *recorder);
    let response = &recorder[13..13 + CHALLENGE_LEN];
    let target_challenge = &target[14..14 + CHALLENGE_LEN];
    for secret in [response, target_challenge, b"justvnc-secret"] {
        assert!(
            !run.to_viewer.windows(secret.len()).any(|w| w == secret),
            "the Viewer saw {secret:?}"
        );
    }
    assert_eq!(run.error, None);
}

#[test]
fn tells_the_viewer_when_the_target_refuses_the_password() {
    let target = client_fixture!("tigervnc-3.8-vncauth-wrong-password.server.bin");
    let mut run = authenticated(VIEWER);
    run.proxy.connect_target(Some(Password::new("wrong")));
    run.target(target);

    assert_eq!(
        run.to_viewer[12 + 2 + 16..],
        [
            &[0, 0, 0, 1, 0, 0, 0, 31][..],
            b"Connection to the Target failed"
        ]
        .concat()
    );
    assert_eq!(
        run.error,
        Some(Error::TargetHandshake(HandshakeError::SecurityFailed {
            reason: Some("Authentication failure".into())
        }))
    );
}

#[test]
fn tells_the_viewer_when_the_host_cannot_reach_the_target() {
    let mut run = authenticated(VIEWER);

    run.proxy.refuse("Target unreachable");
    run.drain();

    assert_eq!(
        run.to_viewer[12 + 2 + 16..],
        [&[0, 0, 0, 1, 0, 0, 0, 18][..], b"Target unreachable"].concat()
    );
    assert_eq!(
        run.proxy.receive_viewer(&[]),
        Err(Error::ViewerHandshake(HandshakeError::SecurityFailed {
            reason: Some("Target unreachable".into())
        }))
    );
}

#[test]
fn closes_on_a_viewer_before_3_8_without_telling_it_why() {
    let config = ProxyConfig {
        version: ProtocolVersion::V3_7,
        ..ProxyConfig::default()
    };
    let mut run = Run::new(ProxySession::with_challenge(config, [0; 16]));
    run.viewer(b"RFB 003.007\n\x01");
    assert_eq!(run.events, [Event::TargetNeeded]);
    let sent = run.to_viewer.len();

    run.proxy.refuse("Target unreachable");
    run.drain();

    assert_eq!(
        run.to_viewer.len(),
        sent,
        "3.7 has no SecurityResult after None"
    );
}

const BGR233: PixelFormat = PixelFormat {
    bits_per_pixel: 8,
    depth: 8,
    big_endian: false,
    true_colour: true,
    red_max: 7,
    green_max: 7,
    blue_max: 3,
    red_shift: 0,
    green_shift: 3,
    blue_shift: 6,
};

/// A Viewer on 3.8 with None that asked for `format`, then a full update of the 256x16 ramp.
fn viewer_asking_for(format: &PixelFormat) -> Vec<u8> {
    let whole = Rect {
        x: 0,
        y: 0,
        width: 256,
        height: 16,
    };
    [
        &b"RFB 003.008\n\x01\x01"[..],
        &client_message::set_pixel_format(format),
        &client_message::set_encodings(&[0]),
        &client_message::framebuffer_update_request(false, whole),
    ]
    .concat()
}

#[test]
fn frames_the_target_in_the_pixel_format_the_viewer_asked_for() {
    // TigerVNC sending the ramp in BGR233, one byte a pixel: framed as the Target's own 32-bit
    // format it would not end where the recording does.
    let target = server_fixture!("tigervnc-ramp-bgr233.server.bin");
    let mut run = Run::new(ProxySession::with_challenge(
        ProxyConfig::default(),
        [0; 16],
    ));
    run.viewer(&viewer_asking_for(&BGR233));
    run.proxy.connect_target(None);
    run.target(target);

    assert_eq!(run.error, None);
    assert!(run.to_viewer.ends_with(&target[server_init_end(target)..]));
    // The Viewer's messages reach the Target unchanged; its SetEncodings needed no narrowing.
    assert!(run.to_target.ends_with(&viewer_asking_for(&BGR233)[14..]));
}

#[test]
fn refuses_a_pixel_format_it_cannot_frame_without_forwarding_it() {
    let target = server_fixture!("tigervnc-ramp-xrgb.server.bin");
    for format in [
        PixelFormat {
            true_colour: false,
            ..BGR233
        },
        PixelFormat {
            blue_shift: 7,
            ..BGR233
        },
    ] {
        let mut run = Run::new(ProxySession::with_challenge(
            ProxyConfig::default(),
            [0; 16],
        ));
        run.viewer(&viewer_asking_for(&format));
        run.proxy.connect_target(None);
        run.target(&target[..server_init_end(target)]);

        assert_eq!(run.error, Some(Error::UnsupportedPixelFormat(format)));
        let set_pixel_format = client_message::set_pixel_format(&format);
        assert!(
            !run.to_target
                .windows(set_pixel_format.len())
                .any(|w| w == set_pixel_format),
            "forwarded {format:?}"
        );
    }
}

#[test]
fn keeps_target_bytes_that_arrive_before_the_target_leg_starts() {
    let mut run = authenticated(VIEWER);

    run.target(TARGET);
    run.proxy.connect_target(None);
    run.drain();

    assert_eq!(run.error, None);
    assert!(run.to_viewer.ends_with(&TARGET[12 + 2 + 4..]));
}

/// A Viewer on 3.8 with None that offered `encodings` and asked for a full 256x128 update.
fn viewer_offering(encodings: &[i32]) -> Vec<u8> {
    let whole = Rect {
        x: 0,
        y: 0,
        width: 256,
        height: 128,
    };
    [
        &b"RFB 003.008\n\x01\x01"[..],
        &client_message::set_encodings(encodings),
        &client_message::framebuffer_update_request(false, whole),
    ]
    .concat()
}

#[test]
fn frames_updates_that_end_at_last_rect_for_a_viewer_that_offered_it() {
    // TigerVNC's Tight updates say 0xFFFF rectangles and end at a LastRect.
    let target = client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin");
    let mut run = Run::new(ProxySession::with_challenge(
        ProxyConfig::default(),
        [0; 16],
    ));
    run.viewer(&viewer_offering(&[7, 16, 0, -223, -224, -24]));
    run.proxy.connect_target(None);
    run.target(target);

    assert_eq!(run.error, None);
    assert!(run.to_viewer.ends_with(&target[server_init_end(target)..]));
}

#[test]
fn stops_at_last_rect_from_a_target_when_the_viewer_did_not_offer_it() {
    // rfbproto: without LastRect, 0xFFFF is a count, here above the limit.
    let target = client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin");
    let mut run = Run::new(ProxySession::with_challenge(
        ProxyConfig::default(),
        [0; 16],
    ));
    run.viewer(&viewer_offering(&[7, 16, 0, -223, -24]));
    run.proxy.connect_target(None);
    run.target(target);

    assert!(
        matches!(run.error, Some(Error::Target(_))),
        "{:?}",
        run.error
    );
}

#[test]
fn never_tells_the_viewer_it_passed_before_the_host_decided() {
    // A host that starts the Target leg before deciding on the Viewer's response.
    let challenge = VIEWER_HANDSHAKE[14..30].try_into().unwrap();
    let mut run = Run::new(ProxySession::with_challenge(vncauth_config(), challenge));
    run.viewer(VIEWER);
    run.proxy.connect_target(None);
    run.target(TARGET);

    assert_eq!(
        run.to_viewer,
        VIEWER_HANDSHAKE[..12 + 2 + 16],
        "no SecurityResult"
    );
    assert!(
        !run.events
            .iter()
            .any(|e| matches!(e, Event::Connected { .. }))
    );
}
