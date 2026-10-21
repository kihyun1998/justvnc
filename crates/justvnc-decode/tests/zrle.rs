use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use justvnc_decode::{DecodeError, Decoder};
use justvnc_proto::{PixelFormat, Rect, encoding};

/// 32 bits per pixel, depth 24, true colour, with the given byte order and shifts.
fn rgb(big_endian: bool, depth: u8, shifts: [u8; 3]) -> PixelFormat {
    PixelFormat {
        bits_per_pixel: 32,
        depth,
        big_endian,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: shifts[0],
        green_shift: shifts[1],
        blue_shift: shifts[2],
    }
}

/// One ZRLE rectangle's data: `tiles` compressed as a whole zlib stream with a sync flush, as a
/// server sends it.
fn zrle(tiles: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(tiles).unwrap();
    z.flush().unwrap();
    z.get_ref().clone()
}

fn decode(
    format: &PixelFormat,
    width: u16,
    height: u16,
    tiles: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    Decoder::new().decode(rect, encoding::ZRLE, &zrle(tiles), format)
}

#[test]
fn takes_the_first_three_bytes_when_colour_sits_low_in_a_little_endian_pixel() {
    // A 1x1 solid tile (subencoding 1) with CPIXEL 11 22 33.
    let pixels = decode(&rgb(false, 24, [16, 8, 0]), 1, 1, &[1, 0x11, 0x22, 0x33]).unwrap();

    assert_eq!(pixels, [0x11, 0x22, 0x33, 0]);
}

#[test]
fn takes_the_last_three_bytes_when_colour_sits_low_in_a_big_endian_pixel() {
    let pixels = decode(&rgb(true, 24, [16, 8, 0]), 1, 1, &[1, 0x11, 0x22, 0x33]).unwrap();

    assert_eq!(pixels, [0, 0x11, 0x22, 0x33]);
}

#[test]
fn takes_the_last_three_bytes_when_colour_sits_high_in_a_little_endian_pixel() {
    let pixels = decode(&rgb(false, 24, [24, 16, 8]), 1, 1, &[1, 0x11, 0x22, 0x33]).unwrap();

    assert_eq!(pixels, [0, 0x11, 0x22, 0x33]);
}

#[test]
fn takes_the_first_three_bytes_when_colour_sits_high_in_a_big_endian_pixel() {
    let pixels = decode(&rgb(true, 24, [24, 16, 8]), 1, 1, &[1, 0x11, 0x22, 0x33]).unwrap();

    assert_eq!(pixels, [0x11, 0x22, 0x33, 0]);
}

#[test]
fn keeps_four_byte_cpixels_when_depth_is_above_24() {
    let pixels = decode(
        &rgb(false, 32, [16, 8, 0]),
        1,
        1,
        &[1, 0x11, 0x22, 0x33, 0x44],
    )
    .unwrap();

    assert_eq!(pixels, [0x11, 0x22, 0x33, 0x44]);
}

#[test]
fn refuses_data_that_inflates_past_its_rectangle() {
    // A 1x1 rectangle can never need a megabyte of tile data.
    let result = decode(&rgb(false, 24, [16, 8, 0]), 1, 1, &vec![0; 1 << 20]);

    assert_eq!(
        result,
        Err(DecodeError::Corrupt(
            "ZRLE data inflates past its rectangle"
        ))
    );
}

#[test]
fn refuses_palette_reuse_which_zrle_forbids() {
    // RFC 6143 7.7.6: subencodings 127 and 129 may not appear in ZRLE.
    for sub in [127, 129] {
        let result = decode(&rgb(false, 24, [16, 8, 0]), 1, 1, &[sub, 0]);

        assert_eq!(
            result,
            Err(DecodeError::Corrupt(
                "ZRLE subencoding is unused or not allowed"
            ))
        );
    }
}

#[test]
fn refuses_a_palette_index_outside_the_palette() {
    // Palette RLE with 2 colours (130), then index 5.
    let tiles = [130, 1, 1, 1, 2, 2, 2, 5];

    let result = decode(&rgb(false, 24, [16, 8, 0]), 1, 1, &tiles);

    assert_eq!(
        result,
        Err(DecodeError::Corrupt("ZRLE palette index out of range"))
    );
}

#[test]
fn refuses_a_run_longer_than_its_tile() {
    // Plain RLE (128) on a 2x1 tile: one pixel, then a run of 3 (length byte 2).
    let tiles = [128, 9, 9, 9, 2];

    let result = decode(&rgb(false, 24, [16, 8, 0]), 2, 1, &tiles);

    assert_eq!(
        result,
        Err(DecodeError::Corrupt("ZRLE run goes past its tile"))
    );
}

#[test]
fn reads_packed_pixels_most_significant_bit_first_with_padded_rows() {
    // 3x2 tile, 2-colour palette (subencoding 2): A = 0a0a0a, B = 0b0b0b. One bit per pixel,
    // rows padded to a byte: row 0 = A B B -> 0b011_00000, row 1 = B A A -> 0b100_00000.
    let tiles = [
        2,
        0x0a,
        0x0a,
        0x0a,
        0x0b,
        0x0b,
        0x0b,
        0b0110_0000,
        0b1000_0000,
    ];

    let pixels = decode(&rgb(false, 24, [16, 8, 0]), 3, 2, &tiles).unwrap();

    let (a, b) = ([0x0a, 0x0a, 0x0a, 0], [0x0b, 0x0b, 0x0b, 0]);
    assert_eq!(pixels, [a, b, b, b, a, a].concat());
}

#[test]
fn reads_palette_indices_up_to_127() {
    // Palette RLE with 100 colours (228): colour n is n n n. Then index 99 alone, and index 70
    // with the run flag (0x80 | 70) and a run of 2 (length byte 1).
    let mut tiles = vec![228];
    for n in 0..100u8 {
        tiles.extend_from_slice(&[n, n, n]);
    }
    tiles.extend_from_slice(&[99, 0x80 | 70, 1]);

    let pixels = decode(&rgb(false, 24, [16, 8, 0]), 3, 1, &tiles).unwrap();

    assert_eq!(pixels, [99, 99, 99, 0, 70, 70, 70, 0, 70, 70, 70, 0]);
}

#[test]
fn refuses_a_pixel_format_whose_colours_do_not_fit_in_the_pixel() {
    // A solid tile: subencoding 1, then one CPIXEL.
    let result = decode(&rgb(false, 24, [16, 8, 70]), 1, 1, &[1, 1, 2, 3]);

    assert_eq!(
        result,
        Err(DecodeError::Unsupported("pixel format is not valid"))
    );
}
