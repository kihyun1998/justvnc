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
