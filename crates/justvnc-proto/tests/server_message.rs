use justvnc_proto::server_message::{Framed, ServerMessageReader};
use justvnc_proto::{Limits, MessageError, PixelFormat};

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

/// LastRect (-224) as a rectangle header: position and size are ignored.
const LAST_RECT: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x20];

#[test]
fn reads_0xffff_as_a_count_for_a_client_that_did_not_offer_last_rect() {
    // A Proxy whose Viewer did not offer LastRect frames 0xFFFF as 65535 rectangles.
    let mut reader = ServerMessageReader::new(&XRGB, 64, 64, Limits::default());

    let result = reader.next(&[0, 0, 0xff, 0xff]);

    assert_eq!(
        result,
        Err(MessageError::LimitExceeded {
            field: "number-of-rectangles",
            length: 65535,
            max: 16384
        })
    );
}

#[test]
fn ends_an_update_at_last_rect_only_when_it_was_offered() {
    let update = [&[0, 0, 0, 3][..], &LAST_RECT].concat();
    let mut with = ServerMessageReader::new(&XRGB, 64, 64, Limits::default()).with_last_rect();
    let mut without = ServerMessageReader::new(&XRGB, 64, 64, Limits::default());

    // rfbproto: a client that offered LastRect stops at it, whatever the count said.
    assert_eq!(with.next(&update), Ok(Some((Framed::UpdateEnd, 16))));
    assert_eq!(
        without.next(&update),
        Err(MessageError::UnsupportedEncoding { encoding: -224 })
    );
}

const BGR233: PixelFormat = PixelFormat {
    bits_per_pixel: 8,
    depth: 8,
    red_max: 7,
    green_max: 7,
    blue_max: 3,
    red_shift: 0,
    green_shift: 3,
    blue_shift: 6,
    ..XRGB
};

/// A one-pixel Raw rectangle header at (0, 0).
const RAW_1X1: [u8; 12] = [0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0];

fn frame_all(reader: &mut ServerMessageReader, mut buf: &[u8]) -> Vec<(Framed, usize)> {
    let mut out = Vec::new();
    while let Some((framed, used)) = reader.next(buf).unwrap() {
        buf = &buf[used..];
        out.push((framed, used));
        if buf.is_empty() && out.last().map(|(f, _)| f) == Some(&Framed::UpdateEnd) {
            break;
        }
    }
    out
}

#[test]
fn takes_a_new_pixel_format_from_the_next_message_on() {
    let mut reader = ServerMessageReader::new(&XRGB, 64, 64, Limits::default());
    // An update of two rectangles is under way in 32-bit pixels when the format changes.
    let first = [&[0, 0, 0, 2][..], &RAW_1X1, &[1, 2, 3, 0]].concat();
    assert_eq!(frame_all(&mut reader, &first).len(), 1);

    reader.set_pixel_format(&BGR233);
    let rest = [&RAW_1X1[..], &[4, 5, 6, 0]].concat();
    let next_update = [&[0, 0, 0, 1][..], &RAW_1X1, &[7]].concat();

    let framed = frame_all(&mut reader, &[rest, next_update].concat());
    let pixels: Vec<Vec<u8>> = framed
        .into_iter()
        .filter_map(|(f, _)| match f {
            Framed::Rectangle { data, .. } => Some(data),
            _ => None,
        })
        .collect();
    assert_eq!(pixels, [vec![4, 5, 6, 0], vec![7]]);
}

#[test]
fn takes_last_rect_from_the_next_message_on() {
    let mut reader = ServerMessageReader::new(&XRGB, 64, 64, Limits::default());
    reader.set_last_rect(true);
    let update = [&[0, 0, 0xff, 0xff][..], &LAST_RECT].concat();

    assert_eq!(reader.next(&update), Ok(Some((Framed::UpdateEnd, 16))));

    reader.set_last_rect(false);
    assert_eq!(
        reader.next(&update),
        Err(MessageError::LimitExceeded {
            field: "number-of-rectangles",
            length: 65535,
            max: 16384
        })
    );
}
