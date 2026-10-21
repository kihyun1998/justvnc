//! The server role, with justvnc's client role on the other end. Both are sans-IO, so the test
//! moves their bytes between them.

mod common;

use justvnc_client::{ClientConfig, Event as ClientEvent};
use justvnc_proto::client_message;
use justvnc_proto::vnc_auth;
use justvnc_server::{
    CutText, Error, Event, Framebuffer, HandshakeError, Limits, MessageError, Password,
    PixelFormat, ProtocolVersion, Rect, SecurityType, ServerConfig, ServerSession,
};

const XRGB: PixelFormat = PixelFormat {
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
};

/// A test pattern: each pixel's red is its x, green its y, blue the frame number. It records
/// the input it is given.
#[derive(Default)]
struct Pattern {
    frame: u8,
    keys: Vec<(bool, u32)>,
    pointer: Vec<(u8, u16, u16)>,
    cut_text: Vec<CutText>,
}

const WIDTH: u16 = 64;
const HEIGHT: u16 = 32;

fn pattern_pixel(x: u16, y: u16, frame: u8) -> [u8; 4] {
    [frame, y as u8, x as u8, 0]
}

impl Framebuffer for Pattern {
    fn size(&self) -> (u16, u16) {
        (WIDTH, HEIGHT)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        let mut out = Vec::new();
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                out.extend_from_slice(&pattern_pixel(x, y, self.frame));
            }
        }
        out
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        self.keys.push((down, keysym));
    }

    fn pointer_event(&mut self, buttons: u8, x: u16, y: u16) {
        self.pointer.push((buttons, x, y));
    }

    fn cut_text(&mut self, text: CutText) {
        self.cut_text.push(text);
    }
}

type Pair = common::Pair<Pattern>;

fn whole() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    }
}

/// The client's picture after applying `events` to a black framebuffer.
fn picture(events: &[ClientEvent]) -> justvnc_client::Framebuffer {
    let mut fb = justvnc_client::Framebuffer::new(WIDTH, HEIGHT, XRGB);
    for event in events {
        fb.apply(event);
    }
    fb
}

fn pattern_picture(frame: u8) -> Vec<u8> {
    Pattern {
        frame,
        ..Pattern::default()
    }
    .read(whole())
}

fn rectangles(events: &[ClientEvent]) -> Vec<Rect> {
    events
        .iter()
        .filter_map(|e| match e {
            ClientEvent::Rectangle(r) => Some(r.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn a_viewer_sees_the_test_pattern() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    pair.client.request_update(false, whole());
    pair.pump();

    let events = pair.client_events();
    let Some(ClientEvent::Connected(init)) = events.first() else {
        panic!("{events:?}");
    };
    assert_eq!(
        (init.width, init.height, init.pixel_format),
        (WIDTH, HEIGHT, XRGB)
    );
    assert_eq!(init.name.as_bytes(), b"justvnc");
    assert_eq!(picture(&events).pixels(), pattern_picture(0));
    assert_eq!(events.last(), Some(&ClientEvent::UpdateComplete));
    assert_eq!(pair.server_events, [Event::Connected { shared: true }]);
    assert_eq!((pair.server_error, pair.client_error), (None, None));
}

#[test]
fn completes_every_version_with_every_security_type() {
    for version in [
        ProtocolVersion::V3_3,
        ProtocolVersion::V3_7,
        ProtocolVersion::V3_8,
    ] {
        for security in [SecurityType::None, SecurityType::VncAuth] {
            let mut pair = Pair::new(
                ServerConfig {
                    version,
                    security: vec![security],
                    ..ServerConfig::default()
                },
                ClientConfig {
                    password: Some(Password::new("secret")),
                    ..ClientConfig::default()
                },
            );
            pair.password = Some(b"secret");
            pair.pump();

            let connected = pair
                .client_events()
                .into_iter()
                .any(|e| matches!(e, ClientEvent::Connected(_)));
            assert!(connected, "{version:?} {security:?}");
            assert_eq!(pair.server_error, None, "{version:?} {security:?}");
        }
    }
}

#[test]
fn hands_the_challenge_and_response_to_the_host() {
    let mut pair = Pair::new(
        ServerConfig {
            security: vec![SecurityType::VncAuth],
            ..ServerConfig::default()
        },
        ClientConfig {
            password: Some(Password::new("secret")),
            ..ClientConfig::default()
        },
    );
    pair.password = Some(b"secret");
    pair.pump();

    let Some(Event::Authenticate {
        challenge,
        response,
    }) = pair.server_events.first()
    else {
        panic!("{:?}", pair.server_events);
    };
    assert_eq!(*challenge, [0x5a; 16]);
    assert_eq!(*response, vnc_auth::response(b"secret", &[0x5a; 16]));
    assert_eq!(pair.server_events[1], Event::Connected { shared: true });
}

#[test]
fn a_rejected_viewer_is_told_why_and_the_session_ends() {
    let mut pair = Pair::new(
        ServerConfig {
            security: vec![SecurityType::VncAuth],
            ..ServerConfig::default()
        },
        ClientConfig {
            password: Some(Password::new("wrong")),
            ..ClientConfig::default()
        },
    );
    pair.password = Some(b"secret");
    pair.pump();

    let failed = HandshakeError::SecurityFailed {
        reason: Some("Authentication failure".into()),
    };
    assert_eq!(
        pair.client_error,
        Some(justvnc_client::Error::Handshake(failed.clone()))
    );
    assert_eq!(
        pair.server.receive(&[], &mut pair.pattern),
        Err(Error::Handshake(failed))
    );
}

#[test]
fn input_from_the_viewer_reaches_the_framebuffer() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    pair.client.key_event(true, 0x61);
    pair.client.key_event(false, 0x61);
    pair.client.pointer_event(0b1, 10, 20);
    pair.client
        .send_cut_text(&CutText::from_text("hello").unwrap());
    pair.pump();

    assert_eq!(pair.pattern.keys, [(true, 0x61), (false, 0x61)]);
    assert_eq!(pair.pattern.pointer, [(0b1, 10, 20)]);
    assert_eq!(
        pair.pattern.cut_text,
        [CutText::from_text("hello").unwrap()]
    );
}

#[test]
fn holds_an_incremental_request_until_the_host_reports_a_change() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    pair.client.request_update(false, whole());
    pair.pump();
    let mut events = pair.client_events();

    pair.client.request_update(true, whole());
    pair.pump();
    assert!(
        rectangles(&pair.client_events()).is_empty(),
        "nothing changed yet"
    );

    pair.pattern.frame = 9;
    let changed = Rect {
        x: 4,
        y: 2,
        width: 8,
        height: 3,
    };
    pair.server.damage(changed, &pair.pattern);
    pair.pump();
    let update = pair.client_events();
    assert_eq!(rectangles(&update), [changed]);

    events.extend(update);
    let seen = picture(&events);
    let (before, after) = (pattern_picture(0), pattern_picture(9));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = (4..12).contains(&x) && (2..5).contains(&y);
            let i = (usize::from(y) * usize::from(WIDTH) + usize::from(x)) * 4;
            let expected = if inside { &after } else { &before };
            assert_eq!(seen.pixels()[i..i + 4], expected[i..i + 4], "({x}, {y})");
        }
    }
}

#[test]
fn sends_a_change_reported_before_the_request_when_the_request_comes() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    let first = Rect {
        x: 1,
        y: 1,
        width: 2,
        height: 2,
    };
    let second = Rect {
        x: 10,
        y: 5,
        width: 1,
        height: 4,
    };
    pair.server.damage(first, &pair.pattern);
    pair.server.damage(second, &pair.pattern);
    pair.pump();
    assert!(
        rectangles(&pair.client_events()).is_empty(),
        "nothing was requested"
    );

    pair.client.request_update(true, whole());
    pair.pump();

    assert_eq!(rectangles(&pair.client_events()), [first, second]);
}

#[test]
fn sends_only_the_part_of_a_change_inside_the_requested_area() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    let left = Rect {
        x: 0,
        y: 0,
        width: 16,
        height: HEIGHT,
    };
    pair.client.request_update(true, left);
    pair.pump();

    pair.server.damage(
        Rect {
            x: 40,
            y: 0,
            width: 8,
            height: 8,
        },
        &pair.pattern,
    );
    pair.pump();
    assert!(
        rectangles(&pair.client_events()).is_empty(),
        "change outside the request"
    );

    pair.server.damage(
        Rect {
            x: 10,
            y: 4,
            width: 20,
            height: 2,
        },
        &pair.pattern,
    );
    pair.pump();
    assert_eq!(
        rectangles(&pair.client_events()),
        [Rect {
            x: 10,
            y: 4,
            width: 6,
            height: 2
        }]
    );
}

#[test]
fn answers_a_request_reaching_past_the_framebuffer_with_the_part_inside_it() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    pair.client.request_update(
        false,
        Rect {
            x: 60,
            y: 30,
            width: 100,
            height: 100,
        },
    );
    pair.pump();

    let events = pair.client_events();
    assert_eq!(
        rectangles(&events),
        [Rect {
            x: 60,
            y: 30,
            width: 4,
            height: 2
        }]
    );
    assert_eq!(pair.client_error, None);
}

/// The session after the handshake, fed raw client bytes.
fn connected_server() -> (ServerSession, Pattern) {
    let mut server = ServerSession::with_challenge(
        ServerConfig {
            limits: Limits {
                max_cut_text_length: 8,
                ..Limits::default()
            },
            ..ServerConfig::default()
        },
        [0; 16],
    );
    let mut pattern = Pattern::default();
    server
        .receive(b"RFB 003.008\n\x01\x01", &mut pattern)
        .unwrap();
    while server.poll_transmit().is_some() {}
    (server, pattern)
}

#[test]
fn refuses_viewer_cut_text_longer_than_the_limit_before_it_arrives() {
    let (mut server, mut pattern) = connected_server();

    let result = server.receive(&[6, 0, 0, 0, 0, 0, 0, 9], &mut pattern);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::LimitExceeded {
            field: "cut-text-length",
            length: 9,
            max: 8
        }))
    );
    assert_eq!(
        server.receive(&[], &mut pattern),
        result,
        "the error sticks"
    );
}

#[test]
fn refuses_a_pixel_format_that_is_not_valid() {
    let (mut server, mut pattern) = connected_server();
    let invalid = PixelFormat {
        blue_shift: 38,
        ..XRGB
    };

    let result = server.receive(&client_message::set_pixel_format(&invalid), &mut pattern);

    assert_eq!(result, Err(Error::UnsupportedPixelFormat(invalid)));
}

#[test]
fn keeps_a_change_outside_the_requested_area_for_a_later_request() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    let left = Rect {
        x: 0,
        y: 0,
        width: 16,
        height: HEIGHT,
    };
    let across = Rect {
        x: 10,
        y: 4,
        width: 20,
        height: 2,
    };
    pair.server.damage(across, &pair.pattern);
    pair.client.request_update(true, left);
    pair.pump();
    pair.client_events();

    pair.client.request_update(true, whole());
    pair.pump();

    assert_eq!(
        rectangles(&pair.client_events()),
        [Rect {
            x: 16,
            y: 4,
            width: 14,
            height: 2
        }]
    );
}

#[test]
fn sends_many_small_changes_as_the_rectangle_that_holds_them() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    for i in 0..100 {
        let dot = Rect {
            x: 2 + (i % 50) as u16,
            y: 3 + (i / 50) as u16 * 10,
            width: 1,
            height: 1,
        };
        pair.server.damage(dot, &pair.pattern);
    }

    pair.client.request_update(true, whole());
    pair.pump();

    let sent = rectangles(&pair.client_events());
    assert!(sent.len() <= 64, "{} rectangles", sent.len());
    let covers = |x: u16, y: u16| {
        sent.iter()
            .any(|r| (r.x..r.x + r.width).contains(&x) && (r.y..r.y + r.height).contains(&y))
    };
    assert!((0..100).all(|i| covers(2 + i % 50, 3 + i / 50 * 10)));
}

#[test]
fn a_full_request_takes_the_changes_it_covers() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    pair.server.damage(
        Rect {
            x: 3,
            y: 3,
            width: 3,
            height: 3,
        },
        &pair.pattern,
    );
    pair.client.request_update(false, whole());
    pair.client.request_update(true, whole());
    pair.pump();

    assert_eq!(
        rectangles(&pair.client_events()),
        [whole()],
        "nothing left to send"
    );
}

#[test]
fn sends_overlapping_changes_once() {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    let square = Rect {
        x: 4,
        y: 4,
        width: 4,
        height: 4,
    };
    pair.server.damage(square, &pair.pattern);
    pair.server.damage(square, &pair.pattern);
    pair.server.damage(
        Rect {
            x: 6,
            y: 4,
            width: 4,
            height: 4,
        },
        &pair.pattern,
    );

    pair.client.request_update(true, whole());
    pair.pump();

    let sent = rectangles(&pair.client_events());
    let area: u32 = sent
        .iter()
        .map(|r| u32::from(r.width) * u32::from(r.height))
        .sum();
    assert_eq!(area, 6 * 4, "{sent:?}");
}
