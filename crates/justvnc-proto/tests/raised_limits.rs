//! Lengths near `u32::MAX`, accepted because the caller raised a limit that far. On a 32-bit
//! target they do not fit in a `usize` offset; the reader waits for bytes that cannot come.

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::server_message::ServerMessageReader;
use justvnc_proto::{Limits, PixelFormat};

const RAISED: Limits = Limits {
    max_name_length: u32::MAX,
    max_reason_length: u32::MAX,
    max_framebuffer_width: u32::MAX,
    max_framebuffer_height: u32::MAX,
    max_rectangles_per_update: u32::MAX,
    max_cut_text_length: u32::MAX,
    max_compressed_length: u32::MAX,
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

const MAX_LENGTH: [u8; 4] = [0xff; 4];

#[test]
fn waits_for_a_desktop_name_of_u32_max_bytes() {
    let mut handshake = ClientHandshake::new(false, None, RAISED);
    let server_init = [&[0, 64, 0, 64][..], &XRGB.to_bytes(), &MAX_LENGTH].concat();
    let bytes = [b"RFB 003.008\n", &[1, 1][..], &[0, 0, 0, 0], &server_init].concat();

    assert_eq!(handshake.receive(&bytes), Ok(None));
}

#[test]
fn waits_for_a_failure_reason_of_u32_max_bytes() {
    let mut handshake = ClientHandshake::new(false, None, RAISED);
    let bytes = [b"RFB 003.008\n", &[0][..], &MAX_LENGTH].concat();

    assert_eq!(handshake.receive(&bytes), Ok(None));
}

#[test]
fn waits_for_cut_text_of_u32_max_bytes() {
    let mut reader = ServerMessageReader::new(&XRGB, 64, 64, RAISED);

    assert_eq!(
        reader.next(&[&[3, 0, 0, 0][..], &MAX_LENGTH].concat()),
        Ok(None)
    );
}

#[test]
fn waits_for_zrle_data_of_u32_max_bytes() {
    let mut reader = ServerMessageReader::new(&XRGB, 64, 64, RAISED);
    let update = [
        &[0, 0, 0, 1][..],
        &[0, 0, 0, 0, 0, 64, 0, 64, 0, 0, 0, 16],
        &MAX_LENGTH,
    ]
    .concat();

    assert_eq!(reader.next(&update), Ok(None));
}

#[test]
fn waits_for_a_raw_rectangle_larger_than_the_address_space() {
    // 65535 x 65535 pixels of 4 bytes is 16 GiB.
    let mut reader = ServerMessageReader::new(&XRGB, 0xffff, 0xffff, RAISED);
    let update = [
        &[0, 0, 0, 1][..],
        &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0],
    ]
    .concat();

    assert_eq!(reader.next(&update), Ok(None));
}
