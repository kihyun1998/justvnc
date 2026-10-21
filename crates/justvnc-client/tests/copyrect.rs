mod common;

use justvnc_client::{
    ClientConfig, ClientSession, Error, Event, Framebuffer, MessageError, PixelFormat, Rect,
    Rectangle,
};

/// TigerVNC (120x80, -retro) answering a full update, then an incremental one after an xlogo
/// window moved from (8, 8) to (50, 30), then another full update.
const SERVER: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-copyrect.server.bin");
const CLIENT: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-copyrect.client.bin");

/// The events of each FramebufferUpdate, after Connected.
fn updates(session: &mut ClientSession) -> (PixelFormat, Vec<Vec<Event>>) {
    let mut events = std::iter::from_fn(|| session.poll_event());
    let Some(Event::Connected(desktop)) = events.next() else {
        panic!("expected Connected first");
    };
    let mut updates = vec![Vec::new()];
    for event in events {
        let done = event == Event::UpdateComplete;
        updates.last_mut().unwrap().push(event);
        if done {
            updates.push(Vec::new());
        }
    }
    updates.pop();
    (desktop.pixel_format, updates)
}

fn apply(framebuffer: &mut Framebuffer, events: &[Event]) {
    events.iter().for_each(|e| framebuffer.apply(e));
}

#[test]
fn rebuilds_a_moved_window_from_copyrect_as_a_full_update_shows_it() {
    let mut session = ClientSession::new(ClientConfig::default());
    let whole = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 80,
    };

    session.receive(SERVER).unwrap();
    session.request_update(false, whole);
    session.request_update(true, whole);
    session.request_update(false, whole);

    let transmit: Vec<u8> = std::iter::from_fn(|| session.poll_transmit())
        .flatten()
        .collect();
    // Version, security type and ClientInit take 14 bytes; SetEncodings follows.
    assert_eq!(
        common::without_set_encodings(&transmit, 14),
        common::without_set_encodings(CLIENT, 14)
    );
    let (format, updates) = updates(&mut session);
    assert_eq!(updates.len(), 3);
    let copies = updates[1]
        .iter()
        .filter(|e| matches!(e, Event::Copy { .. }))
        .count();
    assert_eq!(copies, 4);

    let mut rebuilt = Framebuffer::new(120, 80, format);
    apply(&mut rebuilt, &updates[0]);
    apply(&mut rebuilt, &updates[1]);
    // RFC 6143 7.5.3: a full update never uses CopyRect, so it is the server's own picture.
    let mut reference = Framebuffer::new(120, 80, format);
    apply(&mut reference, &updates[2]);
    assert!(
        rebuilt.pixels() == reference.pixels(),
        "the screen rebuilt with CopyRect differs from the full update"
    );
}

const GREY8: PixelFormat = PixelFormat {
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

/// A 10x10 one-byte-per-pixel framebuffer where every pixel holds a different value.
fn numbered() -> Framebuffer {
    let mut fb = Framebuffer::new(10, 10, GREY8);
    fb.apply(&Event::Rectangle(Rectangle {
        rect: Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        },
        pixels: (0..100).collect(),
    }));
    fb
}

/// What `rect` should hold after a copy from (`sx`, `sy`): the source as it was before.
fn expected_after_copy(before: &[u8], rect: Rect, sx: u16, sy: u16) -> Vec<u8> {
    let mut after = before.to_vec();
    for dy in 0..rect.height {
        for dx in 0..rect.width {
            let from = usize::from(sy + dy) * 10 + usize::from(sx + dx);
            let to = usize::from(rect.y + dy) * 10 + usize::from(rect.x + dx);
            after[to] = before[from];
        }
    }
    after
}

#[test]
fn copies_overlapping_areas_from_the_source_as_it_was() {
    let cases = [
        (
            "down and right",
            Rect {
                x: 3,
                y: 3,
                width: 6,
                height: 6,
            },
            1,
            1,
        ),
        (
            "up and left",
            Rect {
                x: 1,
                y: 1,
                width: 6,
                height: 6,
            },
            3,
            3,
        ),
        (
            "right within the same rows",
            Rect {
                x: 2,
                y: 4,
                width: 7,
                height: 2,
            },
            0,
            4,
        ),
        (
            "left within the same rows",
            Rect {
                x: 0,
                y: 4,
                width: 7,
                height: 2,
            },
            2,
            4,
        ),
    ];
    for (name, rect, src_x, src_y) in cases {
        let mut fb = numbered();
        let before = fb.pixels().to_vec();

        fb.apply(&Event::Copy { rect, src_x, src_y });

        assert_eq!(
            fb.pixels(),
            expected_after_copy(&before, rect, src_x, src_y),
            "{name}"
        );
    }
}

#[test]
fn rejects_a_copy_whose_source_lies_outside_the_framebuffer() {
    let mut session = ClientSession::new(ClientConfig::default());
    // ServerInit only: the recording's first 54 bytes.
    session.receive(&SERVER[..54]).unwrap();

    // One CopyRect rectangle: 10x10 at (0, 0), from (115, 0) on a 120-pixel-wide screen.
    let update = [
        0, 0, 0, 1, 0, 0, 0, 0, 0, 10, 0, 10, 0, 0, 0, 1, 0, 115, 0, 0,
    ];
    let result = session.receive(&update);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::RectangleOutOfBounds {
            rect: Rect {
                x: 115,
                y: 0,
                width: 10,
                height: 10
            }
        }))
    );
}
