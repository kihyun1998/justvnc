use justvnc_client::{
    ClientConfig, ClientSession, Error, Event, Framebuffer, Limits, MessageError, PixelFormat, Rect,
};

const RETRO_SERVER: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-retro-update.server.bin");
const RETRO_CLIENT: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-retro-update.client.bin");

fn drain_transmit(session: &mut ClientSession) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(bytes) = session.poll_transmit() {
        out.extend_from_slice(&bytes);
    }
    out
}

fn drain_events(session: &mut ClientSession) -> Vec<Event> {
    std::iter::from_fn(|| session.poll_event()).collect()
}

/// Xvnc's `-retro` root window: a 4-pixel diagonal stipple, white where this holds.
fn retro_is_white(x: usize, y: usize) -> bool {
    x % 4 == [3, 1, 2, 0][y % 4]
}

/// Xvnc draws its cursor (16x16) into the framebuffer at the screen centre when the client
/// has not asked for the Cursor pseudo-encoding.
fn under_cursor(x: usize, y: usize, width: usize, height: usize) -> bool {
    let (cx, cy) = (width / 2 - 7, height / 2 - 7);
    (cx..cx + 16).contains(&x) && (cy..cy + 16).contains(&y)
}

#[test]
fn rebuilds_tigervncs_screen_from_a_raw_update() {
    let mut session = ClientSession::new(ClientConfig::default());

    session.receive(RETRO_SERVER).unwrap();
    session.request_update(
        false,
        Rect {
            x: 0,
            y: 0,
            width: 48,
            height: 24,
        },
    );

    assert_eq!(drain_transmit(&mut session), RETRO_CLIENT);
    let events = drain_events(&mut session);
    let Some(Event::Connected(desktop)) = events.first() else {
        panic!("expected Connected first, got {events:?}");
    };
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    for event in &events[1..] {
        framebuffer.apply(event);
    }
    let rectangles = events
        .iter()
        .filter(|e| matches!(e, Event::Rectangle(_)))
        .count();
    assert_eq!(rectangles, 5);
    assert_eq!(events.last(), Some(&Event::UpdateComplete));

    let pixels = framebuffer.pixels();
    for y in 0..24 {
        for x in 0..48 {
            if under_cursor(x, y, 48, 24) {
                continue;
            }
            let pixel = &pixels[(y * 48 + x) * 4..][..4];
            let expected: &[u8] = if retro_is_white(x, y) {
                &[0xff, 0xff, 0xff, 0]
            } else {
                &[0, 0, 0, 0]
            };
            assert_eq!(pixel, expected, "pixel ({x}, {y})");
        }
    }
}

/// Server bytes up to the end of ServerInit in the retro recording.
const RETRO_HANDSHAKE_LEN: usize = 54;
/// Client bytes up to the end of ClientInit; SetEncodings follows in the recording.
const RETRO_CLIENT_HANDSHAKE_LEN: usize = 14;
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0];

fn connected(config: ClientConfig) -> ClientSession {
    let mut session = ClientSession::new(config);
    session
        .receive(&RETRO_SERVER[..RETRO_HANDSHAKE_LEN])
        .unwrap();
    drain_transmit(&mut session);
    drain_events(&mut session);
    session
}

fn rect_header(x: u16, y: u16, width: u16, height: u16, encoding: i32) -> Vec<u8> {
    [x, y, width, height]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .chain(encoding.to_be_bytes())
        .collect()
}

fn rgb888() -> PixelFormat {
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
}

#[test]
fn gives_the_same_screen_when_bytes_arrive_one_at_a_time() {
    let mut session = ClientSession::new(ClientConfig::default());

    for byte in RETRO_SERVER {
        session.receive(std::slice::from_ref(byte)).unwrap();
    }

    let events = drain_events(&mut session);
    let Some(Event::Connected(desktop)) = events.first() else {
        panic!("expected Connected first");
    };
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    events.iter().for_each(|e| framebuffer.apply(e));
    assert_eq!(events.last(), Some(&Event::UpdateComplete));
    // In the stipple, (3, 0) and (1, 1) are white and (0, 0) is black.
    assert_eq!(&framebuffer.pixels()[3 * 4..][..4], WHITE);
    assert_eq!(&framebuffer.pixels()[(48 + 1) * 4..][..4], WHITE);
    assert_eq!(&framebuffer.pixels()[..4], [0, 0, 0, 0]);
}

#[test]
fn resizes_on_desktop_size_and_keeps_the_top_left() {
    let mut session = ClientSession::new(ClientConfig::default());
    session.receive(RETRO_SERVER).unwrap();
    let events = drain_events(&mut session);
    let Some(Event::Connected(desktop)) = events.first() else {
        panic!("expected Connected first");
    };
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    events.iter().for_each(|e| framebuffer.apply(e));

    // RFC 6143 7.8.2: DesktopSize is a rectangle with no data; x and y are ignored.
    let update = [&[0, 0, 0, 1][..], &rect_header(7, 7, 16, 8, -223)].concat();
    session.receive(&update).unwrap();

    let events = drain_events(&mut session);
    assert_eq!(
        events,
        [
            Event::DesktopResized {
                width: 16,
                height: 8
            },
            Event::UpdateComplete
        ]
    );
    events.iter().for_each(|e| framebuffer.apply(e));
    assert_eq!((framebuffer.width(), framebuffer.height()), (16, 8));
    assert_eq!(framebuffer.pixels().len(), 16 * 8 * 4);
    // (3, 0) and (1, 1) keep their white pixels; row 1 now starts 16 pixels in.
    assert_eq!(&framebuffer.pixels()[3 * 4..][..4], WHITE);
    assert_eq!(&framebuffer.pixels()[(16 + 1) * 4..][..4], WHITE);
}

#[test]
fn rejects_a_server_framebuffer_larger_than_the_limit() {
    let config = ClientConfig {
        limits: Limits {
            max_framebuffer_width: 47,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = ClientSession::new(config);

    let result = session.receive(&RETRO_SERVER[..RETRO_HANDSHAKE_LEN]);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::LimitExceeded {
            field: "framebuffer-width",
            length: 48,
            max: 47
        }))
    );
}

#[test]
fn rejects_more_rectangles_than_the_limit_before_reading_them() {
    let config = ClientConfig {
        limits: Limits {
            max_rectangles_per_update: 4,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = connected(config);

    // The retro update announces 5 rectangles; only its 4-byte header is fed.
    let result = session.receive(&RETRO_SERVER[RETRO_HANDSHAKE_LEN..][..4]);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::LimitExceeded {
            field: "number-of-rectangles",
            length: 5,
            max: 4
        }))
    );
}

#[test]
fn rejects_a_rectangle_outside_the_framebuffer_before_its_pixels_arrive() {
    let mut session = connected(ClientConfig::default());

    let update = [&[0, 0, 0, 1][..], &rect_header(0, 0, 0xffff, 0xffff, 0)].concat();
    let result = session.receive(&update);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::RectangleOutOfBounds {
            rect: Rect {
                x: 0,
                y: 0,
                width: 0xffff,
                height: 0xffff
            }
        }))
    );
}

#[test]
fn rejects_a_desktop_size_larger_than_the_limit() {
    let config = ClientConfig {
        limits: Limits {
            max_framebuffer_height: 100,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = connected(config);

    let update = [&[0, 0, 0, 1][..], &rect_header(0, 0, 64, 101, -223)].concat();
    let result = session.receive(&update);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::LimitExceeded {
            field: "framebuffer-height",
            length: 101,
            max: 100
        }))
    );
}

#[test]
fn stops_at_a_server_message_it_does_not_handle_and_stays_stopped() {
    let mut session = connected(ClientConfig::default());

    // Bell (message type 2), then a FramebufferUpdate with no rectangles.
    let first = session.receive(&[2, 0, 0, 0, 0]);
    let later = session.receive(&[0, 0, 0, 0]);

    let expected = Err(Error::Message(MessageError::UnsupportedMessage {
        message_type: 2,
    }));
    assert_eq!(first, expected);
    assert_eq!(later, expected);
    assert_eq!(session.poll_event(), None);
}

#[test]
fn asks_for_the_configured_pixel_format_before_the_encodings() {
    let rgb565 = PixelFormat {
        bits_per_pixel: 16,
        depth: 16,
        big_endian: false,
        true_colour: true,
        red_max: 31,
        green_max: 63,
        blue_max: 31,
        red_shift: 11,
        green_shift: 5,
        blue_shift: 0,
    };
    let mut session = ClientSession::new(ClientConfig {
        pixel_format: Some(rgb565),
        ..ClientConfig::default()
    });

    session
        .receive(&RETRO_SERVER[..RETRO_HANDSHAKE_LEN])
        .unwrap();

    // RFC 6143 7.5.1: type 0, 3 bytes padding, then PIXEL_FORMAT (maxes big-endian, 3 bytes padding).
    let set_pixel_format = [
        0, 0, 0, 0, 16, 16, 0, 1, 0, 31, 0, 63, 0, 31, 11, 5, 0, 0, 0, 0,
    ];
    let transmit = drain_transmit(&mut session);
    let (handshake, rest) = transmit.split_at(RETRO_CLIENT_HANDSHAKE_LEN);
    assert_eq!(handshake, &RETRO_CLIENT[..RETRO_CLIENT_HANDSHAKE_LEN]);
    assert_eq!(rest[..20], set_pixel_format);
    assert_eq!(rest[20..], RETRO_CLIENT[RETRO_CLIENT_HANDSHAKE_LEN..][..12]);
    assert_eq!(session.pixel_format(), Some(rgb565));
}

#[test]
fn refuses_a_colour_mapped_pixel_format() {
    let colour_mapped = PixelFormat {
        bits_per_pixel: 8,
        true_colour: false,
        ..rgb888()
    };
    let mut session = ClientSession::new(ClientConfig {
        pixel_format: Some(colour_mapped),
        ..ClientConfig::default()
    });

    let result = session.receive(&RETRO_SERVER[..RETRO_HANDSHAKE_LEN]);
    // The handshake itself completed, so only the session can remember that it stopped.
    let later = session.receive(&RETRO_SERVER[RETRO_HANDSHAKE_LEN..]);

    let expected = Err(Error::UnsupportedPixelFormat(colour_mapped));
    assert_eq!(result, expected);
    assert_eq!(later, expected);
}

#[test]
fn applies_an_empty_raw_rectangle_without_changing_anything() {
    let mut session = connected(ClientConfig::default());
    let mut framebuffer = Framebuffer::new(48, 24, rgb888());

    let update = [
        &[0, 0, 0, 2][..],
        &rect_header(5, 5, 0, 3, 0),
        &rect_header(5, 5, 3, 0, 0),
    ]
    .concat();
    session.receive(&update).unwrap();

    for event in drain_events(&mut session) {
        framebuffer.apply(&event);
    }
    assert!(framebuffer.pixels().iter().all(|&b| b == 0));
}

#[test]
fn asks_for_incremental_updates() {
    let mut session = connected(ClientConfig::default());

    session.request_update(
        true,
        Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        },
    );

    // RFC 6143 7.5.3: type 3, incremental, then x, y, width, height as U16.
    assert_eq!(drain_transmit(&mut session), [3, 1, 0, 1, 0, 2, 0, 3, 0, 4]);
}
