//! Pixel format conversion, CopyRect and DesktopSize, with justvnc's client role on the other
//! end.

mod common;

use common::{XRGB, rectangles};
use justvnc_client::{ClientConfig, ClientSession, Event as ClientEvent};
use justvnc_proto::client_message;
use justvnc_server::{Error, Framebuffer, PixelFormat, Rect, ServerConfig, ServerSession};

/// A framebuffer held as XRGB bytes that a test can draw on and move parts of.
#[derive(Clone)]
struct Canvas {
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: u16, height: u16) -> Self {
        let mut canvas = Self {
            width,
            height,
            pixels: vec![0; usize::from(width) * usize::from(height) * 4],
        };
        for y in 0..height {
            for x in 0..width {
                canvas.set(x, y, [x as u8, y as u8, (x ^ y) as u8, 0]);
            }
        }
        canvas
    }

    fn at(&self, x: u16, y: u16) -> usize {
        (usize::from(y) * usize::from(self.width) + usize::from(x)) * 4
    }

    fn set(&mut self, x: u16, y: u16, pixel: [u8; 4]) {
        let i = self.at(x, y);
        self.pixels[i..i + 4].copy_from_slice(&pixel);
    }

    /// Moves the pixels of `rect` so its top left is at (`x`, `y`), as a window move does.
    fn move_to(&mut self, rect: Rect, x: u16, y: u16) {
        let before = self.clone();
        for dy in 0..rect.height {
            for dx in 0..rect.width {
                let i = before.at(rect.x + dx, rect.y + dy);
                let pixel = before.pixels[i..i + 4].try_into().unwrap();
                self.set(x + dx, y + dy, pixel);
            }
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new(64, 32)
    }
}

impl Framebuffer for Canvas {
    fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        let mut out = Vec::new();
        for y in rect.y..rect.y + rect.height {
            let start = self.at(rect.x, y);
            out.extend_from_slice(&self.pixels[start..start + usize::from(rect.width) * 4]);
        }
        out
    }
}

type Pair = common::Pair<Canvas>;

fn whole(canvas: &Canvas) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: canvas.width,
        height: canvas.height,
    }
}

/// The client's picture after `events`, from a black framebuffer the size of `canvas`.
fn picture(canvas: &Canvas, format: PixelFormat, events: &[ClientEvent]) -> Vec<u8> {
    let mut fb = justvnc_client::Framebuffer::new(canvas.width, canvas.height, format);
    for event in events {
        fb.apply(event);
    }
    fb.pixels().to_vec()
}

/// A connected pair whose client has the whole picture.
fn connected() -> (Pair, Vec<ClientEvent>) {
    let mut pair = Pair::new(ServerConfig::default(), ClientConfig::default());
    pair.pump();
    let whole = whole(&pair.pattern);
    pair.client.request_update(false, whole);
    pair.pump();
    let events = pair.client_events();
    (pair, events)
}

/// What TigerVNC sent for `make_ramp_image.py`'s 256x16 ramps in one pixel format: the bytes
/// of its one Raw rectangle.
fn tigervnc_ramp(server: &[u8], bytes_per_pixel: usize) -> &[u8] {
    let start = 12 + 1 + usize::from(server[12]) + 4;
    let name = u32::from_be_bytes(server[start + 20..start + 24].try_into().unwrap()) as usize;
    let pixels = start + 24 + name + 4 + 12;
    &server[pixels..pixels + 256 * 16 * bytes_per_pixel]
}

const fn format(bpp: u8, depth: u8, be: bool, max: [u16; 3], shift: [u8; 3]) -> PixelFormat {
    PixelFormat {
        bits_per_pixel: bpp,
        depth,
        big_endian: be,
        true_colour: true,
        red_max: max[0],
        green_max: max[1],
        blue_max: max[2],
        red_shift: shift[0],
        green_shift: shift[1],
        blue_shift: shift[2],
    }
}

#[test]
fn converts_to_the_viewers_pixel_format_as_tigervnc_does() {
    let ramp = Canvas {
        width: 256,
        height: 16,
        pixels: tigervnc_ramp(include_bytes!("fixtures/tigervnc-ramp-xrgb.server.bin"), 4).to_vec(),
    };
    let cases: [(PixelFormat, &[u8]); 4] = [
        (
            format(16, 16, false, [31, 63, 31], [11, 5, 0]),
            include_bytes!("fixtures/tigervnc-ramp-rgb565.server.bin"),
        ),
        (
            format(16, 15, false, [31, 31, 31], [10, 5, 0]),
            include_bytes!("fixtures/tigervnc-ramp-rgb555.server.bin"),
        ),
        (
            format(8, 8, false, [7, 7, 3], [0, 3, 6]),
            include_bytes!("fixtures/tigervnc-ramp-bgr233.server.bin"),
        ),
        (
            format(32, 24, true, [255, 255, 255], [0, 8, 16]),
            include_bytes!("fixtures/tigervnc-ramp-be32.server.bin"),
        ),
    ];
    for (format, tigervnc) in cases {
        let config = ClientConfig {
            pixel_format: Some(format),
            ..ClientConfig::default()
        };
        let mut pair = Pair::with(ServerConfig::default(), config, ramp.clone());
        pair.pump();
        pair.client.request_update(false, whole(&ramp));
        pair.pump();

        let events = pair.client_events();
        let expected = tigervnc_ramp(tigervnc, format.bytes_per_pixel());
        assert_eq!(picture(&ramp, format, &events), expected, "{format:?}");
        assert_eq!(pair.server_error, None);
    }
}

#[test]
fn converts_from_a_framebuffer_that_is_not_8_bits_a_colour() {
    // A 16-bit framebuffer's 5-bit red 31 is full red, 16 just over half.
    struct Rgb565;
    impl Framebuffer for Rgb565 {
        fn size(&self) -> (u16, u16) {
            (2, 1)
        }
        fn pixel_format(&self) -> PixelFormat {
            format(16, 16, false, [31, 63, 31], [11, 5, 0])
        }
        fn read(&self, _: Rect) -> Vec<u8> {
            [
                (31u16 << 11).to_le_bytes(),
                (16u16 << 11 | 63 << 5).to_le_bytes(),
            ]
            .concat()
        }
    }
    let mut server = ServerSession::with_challenge(ServerConfig::default(), [0; 16]);
    let mut client = ClientSession::new(ClientConfig {
        pixel_format: Some(XRGB),
        ..ClientConfig::default()
    });
    let mut fb = Rgb565;
    let mut moved = true;
    let mut requested = false;
    while moved {
        moved = false;
        while let Some(bytes) = server.poll_transmit() {
            moved = true;
            client.receive(&bytes).unwrap();
        }
        if !requested && client.pixel_format().is_some() {
            client.request_update(
                false,
                Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            );
            requested = true;
        }
        while let Some(bytes) = client.poll_transmit() {
            moved = true;
            server.receive(&bytes, &mut fb).unwrap();
        }
    }

    let pixels = std::iter::from_fn(|| client.poll_event())
        .find_map(|e| match e {
            ClientEvent::Rectangle(r) => Some(r.pixels),
            _ => None,
        })
        .unwrap();
    // (16 * 255 + 16) / 31 = 132; 63 green is 255.
    assert_eq!(pixels, [0, 0, 255, 0, 0, 255, 132, 0]);
}

#[test]
fn refuses_a_colour_mapped_pixel_format() {
    let mut server = ServerSession::with_challenge(ServerConfig::default(), [0; 16]);
    let mut canvas = Canvas::default();
    server
        .receive(b"RFB 003.008\n\x01\x01", &mut canvas)
        .unwrap();
    let mapped = PixelFormat {
        true_colour: false,
        ..format(8, 8, false, [7, 7, 3], [0, 3, 6])
    };

    let result = server.receive(&client_message::set_pixel_format(&mapped), &mut canvas);

    assert_eq!(result, Err(Error::UnsupportedPixelFormat(mapped)));
}

#[test]
fn sends_a_move_as_copyrect() {
    let (mut pair, mut events) = connected();
    let window = Rect {
        x: 4,
        y: 4,
        width: 10,
        height: 6,
    };
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    pair.pattern.move_to(window, 30, 20);
    pair.server.copy(
        Rect {
            x: 30,
            y: 20,
            ..window
        },
        window.x,
        window.y,
        &pair.pattern,
    );
    pair.pump();

    let update = pair.client_events();
    assert!(
        update.contains(&ClientEvent::Copy {
            rect: Rect {
                x: 30,
                y: 20,
                ..window
            },
            src_x: 4,
            src_y: 4
        }),
        "{update:?}"
    );
    assert!(rectangles(&update).is_empty(), "only the copy: {update:?}");
    events.extend(update);
    assert_eq!(
        picture(&pair.pattern, XRGB, &events),
        pair.pattern.read(whole(&pair.pattern))
    );
}

#[test]
fn sends_a_copy_and_a_change_together_with_the_copy_first() {
    let (mut pair, mut events) = connected();
    let window = Rect {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    };
    // The window moves right by 2, overlapping where it was; its old left edge is redrawn.
    pair.pattern.move_to(window, 2, 0);
    for y in 0..8 {
        for x in 0..2 {
            pair.pattern.set(x, y, [0xff, 0, 0, 0]);
        }
    }
    pair.server
        .copy(Rect { x: 2, ..window }, 0, 0, &pair.pattern);
    pair.server
        .damage(Rect { width: 2, ..window }, &pair.pattern);
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    let update = pair.client_events();
    let copy = update
        .iter()
        .position(|e| matches!(e, ClientEvent::Copy { .. }))
        .expect("a copy");
    let raw = update
        .iter()
        .position(|e| matches!(e, ClientEvent::Rectangle(_)))
        .expect("a rectangle");
    assert!(copy < raw, "{update:?}");
    events.extend(update);
    assert_eq!(
        picture(&pair.pattern, XRGB, &events),
        pair.pattern.read(whole(&pair.pattern))
    );
}

#[test]
fn sends_a_move_whose_source_has_unsent_changes_as_pixels() {
    let (mut pair, mut events) = connected();
    let window = Rect {
        x: 4,
        y: 4,
        width: 6,
        height: 6,
    };
    // The Viewer has not seen this change yet, so it cannot copy from there.
    pair.pattern.set(5, 5, [1, 2, 3, 0]);
    pair.server.damage(
        Rect {
            x: 5,
            y: 5,
            width: 1,
            height: 1,
        },
        &pair.pattern,
    );
    pair.pattern.move_to(window, 40, 10);
    pair.server.copy(
        Rect {
            x: 40,
            y: 10,
            ..window
        },
        4,
        4,
        &pair.pattern,
    );
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    let update = pair.client_events();
    assert!(
        !update.iter().any(|e| matches!(e, ClientEvent::Copy { .. })),
        "{update:?}"
    );
    events.extend(update);
    assert_eq!(
        picture(&pair.pattern, XRGB, &events),
        pair.pattern.read(whole(&pair.pattern))
    );
}

#[test]
fn sends_a_second_move_before_the_first_is_sent_as_pixels() {
    let (mut pair, mut events) = connected();
    let first = Rect {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    let second = Rect {
        x: 10,
        y: 10,
        width: 4,
        height: 4,
    };
    pair.pattern.move_to(first, 20, 0);
    pair.server
        .copy(Rect { x: 20, ..first }, 0, 0, &pair.pattern);
    pair.pattern.move_to(second, 30, 10);
    pair.server
        .copy(Rect { x: 30, ..second }, 10, 10, &pair.pattern);
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    let update = pair.client_events();
    let copies = update
        .iter()
        .filter(|e| matches!(e, ClientEvent::Copy { .. }))
        .count();
    assert_eq!(copies, 1, "{update:?}");
    events.extend(update);
    assert_eq!(
        picture(&pair.pattern, XRGB, &events),
        pair.pattern.read(whole(&pair.pattern))
    );
}

/// The server's bytes after a 3.8 None handshake in which the Viewer offered only `encodings`,
/// read by a client session.
fn raw_viewer(encodings: &[i32]) -> (ServerSession, Canvas, ClientSession) {
    let mut server = ServerSession::with_challenge(ServerConfig::default(), [0; 16]);
    let mut canvas = Canvas::default();
    server
        .receive(b"RFB 003.008\n\x01\x01", &mut canvas)
        .unwrap();
    server
        .receive(&client_message::set_encodings(encodings), &mut canvas)
        .unwrap();
    let mut client = ClientSession::new(ClientConfig::default());
    while let Some(bytes) = server.poll_transmit() {
        client.receive(&bytes).unwrap();
    }
    (server, canvas, client)
}

fn feed(server: &mut ServerSession, client: &mut ClientSession) -> Vec<ClientEvent> {
    while let Some(bytes) = server.poll_transmit() {
        client.receive(&bytes).unwrap();
    }
    std::iter::from_fn(|| client.poll_event()).collect()
}

#[test]
fn sends_a_move_as_pixels_to_a_viewer_that_did_not_offer_copyrect() {
    let (mut server, mut canvas, mut client) = raw_viewer(&[0]);
    let window = Rect {
        x: 4,
        y: 4,
        width: 6,
        height: 6,
    };
    canvas.move_to(window, 40, 10);
    server.copy(
        Rect {
            x: 40,
            y: 10,
            ..window
        },
        4,
        4,
        &canvas,
    );
    let request = client_message::framebuffer_update_request(true, whole(&canvas));
    server.receive(&request, &mut canvas).unwrap();

    let events = feed(&mut server, &mut client);
    assert!(
        !events.iter().any(|e| matches!(e, ClientEvent::Copy { .. })),
        "{events:?}"
    );
    assert_eq!(
        rectangles(&events),
        [Rect {
            x: 40,
            y: 10,
            ..window
        }]
    );
}

#[test]
fn announces_a_new_size_then_sends_the_whole_framebuffer() {
    let (mut pair, _) = connected();
    pair.pattern = Canvas::new(80, 20);
    pair.server.resize(&pair.pattern).unwrap();
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    let resized = pair.client_events();
    assert_eq!(
        resized
            .iter()
            .filter(|e| !matches!(e, ClientEvent::UpdateComplete))
            .collect::<Vec<_>>(),
        [&ClientEvent::DesktopResized {
            width: 80,
            height: 20
        }],
        "the DesktopSize update holds nothing else"
    );

    // The Viewer asks again, incrementally, and gets everything: the server considers the whole
    // framebuffer changed.
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();
    let update = pair.client_events();
    assert_eq!(rectangles(&update), [whole(&pair.pattern)]);
    let mut events = resized;
    events.extend(update);
    assert_eq!(
        picture(&pair.pattern, XRGB, &events),
        pair.pattern.read(whole(&pair.pattern))
    );
}

#[test]
fn announces_a_new_size_at_once_when_a_request_is_waiting() {
    let (mut pair, _) = connected();
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    pair.pattern = Canvas::new(32, 32);
    pair.server.resize(&pair.pattern).unwrap();
    pair.pump();

    assert!(pair.client_events().contains(&ClientEvent::DesktopResized {
        width: 32,
        height: 32
    }));
}

#[test]
fn announces_nothing_when_the_size_did_not_change() {
    let (mut pair, _) = connected();
    pair.client.request_update(true, whole(&pair.pattern));
    pair.pump();

    pair.server.resize(&pair.pattern).unwrap();
    pair.pump();

    assert_eq!(pair.client_events(), []);
}

#[test]
fn ends_the_session_when_a_viewer_that_cannot_resize_would_have_to() {
    let (mut server, _, _) = raw_viewer(&[0, 1]);
    let bigger = Canvas::new(100, 100);

    let result = server.resize(&bigger);

    assert_eq!(result, Err(Error::ViewerCannotResize));
    assert_eq!(
        server.receive(&[], &mut Canvas::default()),
        result,
        "the error sticks"
    );
}

#[test]
fn converts_from_a_big_endian_framebuffer() {
    // One pixel of red 0x11, green 0x22, blue 0x33 as 0x00RRGGBB, most significant byte first.
    struct BigEndian;
    impl Framebuffer for BigEndian {
        fn size(&self) -> (u16, u16) {
            (1, 1)
        }
        fn pixel_format(&self) -> PixelFormat {
            PixelFormat {
                big_endian: true,
                ..XRGB
            }
        }
        fn read(&self, _: Rect) -> Vec<u8> {
            vec![0, 0x11, 0x22, 0x33]
        }
    }
    let mut server = ServerSession::with_challenge(ServerConfig::default(), [0; 16]);
    let mut fb = BigEndian;
    server.receive(b"RFB 003.008\n\x01\x01", &mut fb).unwrap();
    let mut client = ClientSession::new(ClientConfig::default());
    while let Some(bytes) = server.poll_transmit() {
        client.receive(&bytes).unwrap();
    }
    let request = [
        &client_message::set_pixel_format(&XRGB)[..],
        &client_message::framebuffer_update_request(
            false,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
        ),
    ]
    .concat();
    server.receive(&request, &mut fb).unwrap();

    // The client session decodes in the server's own format, so read the Raw bytes directly.
    let mut sent = Vec::new();
    while let Some(bytes) = server.poll_transmit() {
        sent.extend(bytes);
    }
    assert_eq!(sent[sent.len() - 4..], [0x33, 0x22, 0x11, 0]);
}
