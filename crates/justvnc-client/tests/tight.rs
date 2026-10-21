mod common;

use justvnc_client::{ClientConfig, ClientSession, Event, Framebuffer, Rect};

/// Xtightvnc 1.3.10 (512x128): a full update of `make_zrle_image.py`'s image, then three
/// incremental ones as `make_tight_images.py`'s windows open beside it. Together they hold fill,
/// copy, gradient and both palette forms, on all four zlib streams, and LastRect.
const TIGHT_SERVER: &[u8] = include_bytes!("fixtures/xtightvnc-3.8-none-tight.server.bin");
const TIGHT_CLIENT: &[u8] = include_bytes!("fixtures/xtightvnc-3.8-none-tight.client.bin");
/// The final screen over a second connection that asked for Raw only.
const REFERENCE: &[u8] =
    include_bytes!("fixtures/xtightvnc-3.8-none-tight-reference-raw.server.bin");

const WHOLE: Rect = Rect {
    x: 0,
    y: 0,
    width: 512,
    height: 128,
};

/// Feeds a recording and returns the framebuffer after each FramebufferUpdate.
fn frames(server: &[u8], session: &mut ClientSession) -> Vec<Vec<u8>> {
    session.receive(server).unwrap();
    let mut events = std::iter::from_fn(|| session.poll_event());
    let Some(Event::Connected(desktop)) = events.next() else {
        panic!("expected Connected first");
    };
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    let mut frames = Vec::new();
    for event in events {
        framebuffer.apply(&event);
        if event == Event::UpdateComplete {
            frames.push(framebuffer.pixels().to_vec());
        }
    }
    frames
}

/// Each pixel's red, green and blue bits in the server's format: 32 bpp little-endian, 0x00RRGGBB.
fn colour_bits(frame: &[u8]) -> Vec<u32> {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_le_bytes(*p) & 0x00ff_ffff)
        .collect()
}

#[test]
fn decodes_xtightvncs_tight_as_its_raw_picture() {
    let mut reference_session = ClientSession::new(ClientConfig::default());
    let [reference] = frames(REFERENCE, &mut reference_session)
        .try_into()
        .unwrap();
    let mut session = ClientSession::new(ClientConfig::default());

    let frames = frames(TIGHT_SERVER, &mut session);
    session.request_update(false, WHOLE);
    for _ in 0..3 {
        session.request_update(true, WHOLE);
    }

    let transmit: Vec<u8> = std::iter::from_fn(|| session.poll_transmit())
        .flatten()
        .collect();
    // Version, security type and ClientInit take 14 bytes; SetEncodings follows.
    assert_eq!(
        common::without_set_encodings(&transmit, 14),
        common::without_set_encodings(TIGHT_CLIENT, 14)
    );
    assert_eq!(frames.len(), 4);
    // Tight's 3-byte TPIXELs carry the colour bits only, so only those are compared.
    assert!(
        colour_bits(&frames[3]) == colour_bits(&reference),
        "the screen rebuilt from Tight differs from the Raw picture"
    );
}

/// Where ServerInit starts and ends in the recording: after the version (12 bytes), the
/// security types (Xtightvnc offers two: None and Tight) and SecurityResult (4).
fn server_init_range() -> std::ops::Range<usize> {
    let start = 12 + 1 + usize::from(TIGHT_SERVER[12]) + 4;
    let name_length =
        u32::from_be_bytes(TIGHT_SERVER[start + 20..start + 24].try_into().unwrap()) as usize;
    start..start + 24 + name_length
}

/// A session that has read the recording's ServerInit (512x128) and nothing else.
fn connected(limits: justvnc_client::Limits) -> ClientSession {
    let mut session = ClientSession::new(ClientConfig {
        limits,
        ..ClientConfig::default()
    });
    session
        .receive(&TIGHT_SERVER[..server_init_range().end])
        .unwrap();
    session
}

fn rect_header(x: u16, y: u16, width: u16, height: u16, encoding: i32) -> Vec<u8> {
    [x, y, width, height]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .chain(encoding.to_be_bytes())
        .collect()
}

fn message_error(result: Result<(), justvnc_client::Error>) -> justvnc_client::MessageError {
    match result {
        Err(justvnc_client::Error::Message(e)) => e,
        other => panic!("expected a framing error, got {other:?}"),
    }
}

#[test]
fn refuses_a_tight_rectangle_wider_than_2048() {
    let limits = justvnc_client::Limits {
        max_framebuffer_width: 4096,
        ..justvnc_client::Limits::default()
    };
    let mut session = ClientSession::new(ClientConfig {
        limits,
        ..ClientConfig::default()
    });
    // A ServerInit for a 4000x128 screen, built from the recording's.
    let init = server_init_range();
    let mut handshake = TIGHT_SERVER[..init.end].to_vec();
    handshake[init.start..init.start + 2].copy_from_slice(&4000u16.to_be_bytes());
    session.receive(&handshake).unwrap();

    // rfbproto: a Tight rectangle may not be wider than 2048 pixels. Fill, so no data is missing.
    let update = [
        &[0, 0, 0, 1][..],
        &rect_header(0, 0, 2049, 1, 7),
        &[0x80, 1, 2, 3],
    ]
    .concat();

    assert_eq!(
        message_error(session.receive(&update)),
        justvnc_client::MessageError::Malformed("Tight rectangle is wider than 2048")
    );
}

#[test]
fn counts_rectangles_towards_the_limit_as_they_arrive_until_last_rect() {
    let mut session = connected(justvnc_client::Limits {
        max_rectangles_per_update: 2,
        ..justvnc_client::Limits::default()
    });
    let fill = |x| [rect_header(x, 0, 1, 1, 7), vec![0x80, 1, 2, 3]].concat();

    // 0xFFFF rectangles: two fills, then a LastRect, is within the limit.
    let ok = [
        vec![0, 0, 0xff, 0xff],
        fill(0),
        fill(1),
        rect_header(0, 0, 0, 0, -224),
    ]
    .concat();
    session.receive(&ok).unwrap();
    let events: Vec<Event> = std::iter::from_fn(|| session.poll_event()).collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Rectangle(_)))
            .count(),
        2
    );
    assert_eq!(events.last(), Some(&Event::UpdateComplete));

    // A third rectangle before the LastRect is one too many.
    let too_many = [vec![0, 0, 0xff, 0xff], fill(0), fill(1), fill(2)].concat();
    assert_eq!(
        message_error(session.receive(&too_many)),
        justvnc_client::MessageError::LimitExceeded {
            field: "number-of-rectangles",
            length: 3,
            max: 2
        }
    );
}

#[test]
fn refuses_tight_data_longer_than_the_limit_before_it_arrives() {
    let mut session = connected(justvnc_client::Limits {
        max_compressed_length: 100,
        ..justvnc_client::Limits::default()
    });

    // Copy filter on a 10x10 rectangle (300 bytes filtered), compact length 101 (one byte).
    let update = [
        &[0, 0, 0, 1][..],
        &rect_header(0, 0, 10, 10, 7),
        &[0x00, 101],
    ]
    .concat();

    assert_eq!(
        message_error(session.receive(&update)),
        justvnc_client::MessageError::LimitExceeded {
            field: "tight-length",
            length: 101,
            max: 100
        }
    );
}

#[test]
fn frames_filtered_data_under_12_bytes_as_sent_without_zlib() {
    let mut session = connected(justvnc_client::Limits::default());

    // Copy filter (control 0x00) on a 3x1 rectangle: 9 bytes of TPIXELs, so no compact length
    // and no zlib (rfbproto). Then a 1x1 fill, to show the stream stays in step.
    let update = [
        &[0, 0, 0, 2][..],
        &rect_header(0, 0, 3, 1, 7),
        &[0x00, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        &rect_header(3, 0, 1, 1, 7),
        &[0x80, 0xaa, 0xbb, 0xcc],
    ]
    .concat();
    session.receive(&update).unwrap();

    let pixels: Vec<Vec<u8>> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|e| match e {
            Event::Rectangle(r) => Some(r.pixels),
            _ => None,
        })
        .collect();
    // TPIXELs are red, green, blue; the server's pixels are 0x00RRGGBB little-endian.
    assert_eq!(
        pixels,
        [
            vec![3, 2, 1, 0, 6, 5, 4, 0, 9, 8, 7, 0],
            vec![0xcc, 0xbb, 0xaa, 0]
        ]
    );
}

#[test]
fn frames_a_two_colour_palette_by_its_one_bit_rows() {
    let mut session = connected(justvnc_client::Limits::default());

    // Palette filter with 2 colours on a 9x1 rectangle: 2 bytes of bits (9 pixels, padded), raw.
    // A 1-byte-per-pixel reading would wait for 9 bytes and swallow the fill that follows.
    let update = [
        &[0, 0, 0, 2][..],
        &rect_header(0, 0, 9, 1, 7),
        &[0x40, 1, 1, 0, 0, 0, 255, 255, 255, 0b1000_0000, 0b1000_0000],
        &rect_header(9, 0, 1, 1, 7),
        &[0x80, 0xaa, 0xbb, 0xcc],
    ]
    .concat();
    session.receive(&update).unwrap();

    let events: Vec<Event> = std::iter::from_fn(|| session.poll_event()).collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Rectangle(_)))
            .count(),
        2
    );
    assert_eq!(events.last(), Some(&Event::UpdateComplete));
}
