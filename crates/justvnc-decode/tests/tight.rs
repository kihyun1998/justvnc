use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use justvnc_decode::{DecodeError, Decoder};
use justvnc_proto::{PixelFormat, Rect, encoding};

/// 32 bpp, depth 24, 8-bit components at shifts 16/8/0: Tight's 3-byte TPIXEL case.
fn xrgb(big_endian: bool) -> PixelFormat {
    PixelFormat {
        bits_per_pixel: 32,
        depth: 24,
        big_endian,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 16,
        green_shift: 8,
        blue_shift: 0,
    }
}

/// 16 bpp little-endian 5-6-5: TPIXELs are whole 2-byte pixels.
const RGB565: PixelFormat = PixelFormat {
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

fn rect(width: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width,
        height,
    }
}

fn decode(d: &mut Decoder, r: Rect, data: &[u8], f: &PixelFormat) -> Result<Vec<u8>, DecodeError> {
    d.decode(r, encoding::TIGHT, data, f)
}

/// A zlib stream fed rectangle by rectangle, sync-flushed after each as a server does.
struct Stream(ZlibEncoder<Vec<u8>>);

impl Stream {
    fn new() -> Self {
        Self(ZlibEncoder::new(Vec::new(), Compression::default()))
    }

    /// The compact length and zlib data for `filtered`, continuing this stream.
    fn next(&mut self, filtered: &[u8]) -> Vec<u8> {
        self.0.write_all(filtered).unwrap();
        self.0.flush().unwrap();
        let zlib = std::mem::take(self.0.get_mut());
        assert!(
            zlib.len() < 128,
            "keep test data to a one-byte compact length"
        );
        [&[zlib.len() as u8][..], &zlib].concat()
    }
}

/// Basic compression, copy filter, stream 0, with `reset` in the low bits: a 4x1 rectangle of
/// TPIXELs (12 bytes, so it is compressed).
fn copy_rect(reset: u8, stream: &mut Stream, colours: [[u8; 3]; 4]) -> Vec<u8> {
    let filtered = colours.concat();
    [&[reset][..], &stream.next(&filtered)].concat()
}

#[test]
fn fills_with_a_tpixel_read_as_red_green_blue() {
    // FillCompression (0x80), TPIXEL red 0x11, green 0x22, blue 0x33.
    let data = [0x80, 0x11, 0x22, 0x33];

    let little = decode(&mut Decoder::new(), rect(2, 1), &data, &xrgb(false)).unwrap();
    let big = decode(&mut Decoder::new(), rect(2, 1), &data, &xrgb(true)).unwrap();

    // 0x00112233 as a little-endian and as a big-endian 32-bit pixel.
    assert_eq!(little, [0x33, 0x22, 0x11, 0, 0x33, 0x22, 0x11, 0]);
    assert_eq!(big, [0, 0x11, 0x22, 0x33, 0, 0x11, 0x22, 0x33]);
}

#[test]
fn continues_a_zlib_stream_until_a_reset_bit_starts_it_again() {
    let mut decoder = Decoder::new();
    let mut server_stream = Stream::new();
    let a = [[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]];
    let b = [[5, 5, 5], [6, 6, 6], [7, 7, 7], [8, 8, 8]];

    let first = decode(
        &mut decoder,
        rect(4, 1),
        &copy_rect(0, &mut server_stream, a),
        &xrgb(false),
    );
    let second = decode(
        &mut decoder,
        rect(4, 1),
        &copy_rect(0, &mut server_stream, b),
        &xrgb(false),
    );
    // The server starts stream 0 again and says so with bit 0.
    let mut new_stream = Stream::new();
    let third = decode(
        &mut decoder,
        rect(4, 1),
        &copy_rect(0x01, &mut new_stream, a),
        &xrgb(false),
    );

    let pixels = |c: [[u8; 3]; 4]| {
        c.iter()
            .flat_map(|p| [p[2], p[1], p[0], 0])
            .collect::<Vec<u8>>()
    };
    assert_eq!(first.unwrap(), pixels(a));
    assert_eq!(second.unwrap(), pixels(b));
    assert_eq!(third.unwrap(), pixels(a));
}

#[test]
fn fails_when_a_new_stream_arrives_without_its_reset_bit() {
    let mut decoder = Decoder::new();
    let a = [[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]];
    decode(
        &mut decoder,
        rect(4, 1),
        &copy_rect(0, &mut Stream::new(), a),
        &xrgb(false),
    )
    .unwrap();

    // A second zlib header in the middle of stream 0.
    let result = decode(
        &mut decoder,
        rect(4, 1),
        &copy_rect(0, &mut Stream::new(), a),
        &xrgb(false),
    );

    assert!(matches!(result, Err(DecodeError::Corrupt(_))), "{result:?}");
}

#[test]
fn undoes_the_gradient_filter_on_16_bit_pixels() {
    // A 2x1 rectangle, rgb565. V0 = (r 10, g 20, b 5), V1 = (r 8, g 25, b 7).
    // Predictions: P0 = 0 (nothing left or above), P1 = V0. Differences, modulo each range:
    // D0 = V0 = 0x5285; D1 = (8-10 mod 32, 25-20, 7-5) = (30, 5, 2) = 0xf0a2.
    // Basic with filter-id (0x40, stream 0), gradient (2); 4 bytes, so not compressed.
    let data = [0x40, 2, 0x85, 0x52, 0xa2, 0xf0];

    let pixels = decode(&mut Decoder::new(), rect(2, 1), &data, &RGB565).unwrap();

    // V0 = 0x5285, V1 = 8 << 11 | 25 << 5 | 7 = 0x4327, little-endian.
    assert_eq!(pixels, [0x85, 0x52, 0x27, 0x43]);
}

#[test]
fn reads_a_two_colour_palette_one_bit_per_pixel_with_padded_rows() {
    // Basic with filter-id (0x40), palette (1), 2 colours (1), A = 0a0a0a, B = 0b0b0b, then
    // 3x2 pixels as bits: row 0 = A B B -> 011_00000, row 1 = B A A -> 100_00000 (2 bytes, raw).
    let data = [
        0x40,
        1,
        1,
        0x0a,
        0x0a,
        0x0a,
        0x0b,
        0x0b,
        0x0b,
        0b0110_0000,
        0b1000_0000,
    ];

    let pixels = decode(&mut Decoder::new(), rect(3, 2), &data, &xrgb(false)).unwrap();

    let (a, b) = ([0x0a, 0x0a, 0x0a, 0], [0x0b, 0x0b, 0x0b, 0]);
    assert_eq!(pixels, [a, b, b, b, a, a].concat());
}

#[test]
fn refuses_a_palette_index_outside_the_palette() {
    // Palette of 3 colours (count byte 2), then 2x1 indices 0 and 7: 2 bytes, raw.
    let data = [0x40, 1, 2, 1, 1, 1, 2, 2, 2, 3, 3, 3, 0, 7];

    let result = decode(&mut Decoder::new(), rect(2, 1), &data, &xrgb(false));

    assert_eq!(
        result,
        Err(DecodeError::Corrupt("Tight palette index out of range"))
    );
}

#[test]
fn refuses_data_that_inflates_past_its_rectangle() {
    // A 4x1 copy rectangle needs 12 bytes; this stream inflates to 4096.
    let mut stream = Stream::new();
    let data = [&[0u8][..], &stream.next(&[0; 4096])].concat();

    let result = decode(&mut Decoder::new(), rect(4, 1), &data, &xrgb(false));

    assert_eq!(
        result,
        Err(DecodeError::Corrupt(
            "Tight data inflates past its rectangle"
        ))
    );
}

#[test]
#[cfg(not(feature = "jpeg"))]
fn reports_jpeg_as_not_supported_yet() {
    // JpegCompression (0x90), compact length 2, two bytes.
    let result = decode(
        &mut Decoder::new(),
        rect(1, 1),
        &[0x90, 2, 0xff, 0xd8],
        &xrgb(false),
    );

    assert_eq!(result, Err(DecodeError::Unsupported("Tight JPEG")));
}

/// The first JFIF image in TigerVNC's JPEG recording (256x64), from its SOI to its EOI marker.
#[cfg(feature = "jpeg")]
fn recorded_jpeg() -> Vec<u8> {
    let recording = include_bytes!(
        "../../justvnc-client/tests/fixtures/tigervnc-3.8-none-tight-jpeg.server.bin"
    );
    let start = recording
        .windows(3)
        .position(|w| w == [0xff, 0xd8, 0xff])
        .unwrap();
    let end = start
        + recording[start..]
            .windows(2)
            .position(|w| w == [0xff, 0xd9])
            .unwrap()
        + 2;
    recording[start..end].to_vec()
}

/// JpegCompression with a compact length, for data shorter than 4194304 bytes.
#[cfg(feature = "jpeg")]
fn jpeg_rect(jfif: &[u8]) -> Vec<u8> {
    let n = jfif.len();
    let mut out = vec![0x90];
    match n {
        0..=127 => out.push(n as u8),
        128..=16383 => out.extend([(n & 0x7f) as u8 | 0x80, (n >> 7) as u8]),
        _ => out.extend([
            (n & 0x7f) as u8 | 0x80,
            ((n >> 7) & 0x7f) as u8 | 0x80,
            (n >> 14) as u8,
        ]),
    }
    [out, jfif.to_vec()].concat()
}

#[test]
#[cfg(feature = "jpeg")]
fn decodes_a_recorded_jpeg_the_size_of_its_rectangle() {
    let pixels = decode(
        &mut Decoder::new(),
        rect(256, 64),
        &jpeg_rect(&recorded_jpeg()),
        &xrgb(false),
    );

    assert_eq!(pixels.map(|p| p.len()), Ok(256 * 64 * 4));
}

#[test]
#[cfg(feature = "jpeg")]
fn refuses_a_jpeg_that_is_not_the_size_of_its_rectangle() {
    // The image is 256x64: a narrower rectangle and a wider one.
    for width in [255, 257] {
        let result = decode(
            &mut Decoder::new(),
            rect(width, 64),
            &jpeg_rect(&recorded_jpeg()),
            &xrgb(false),
        );

        assert!(
            matches!(result, Err(DecodeError::Corrupt(_))),
            "{width}: {result:?}"
        );
    }
}

#[test]
#[cfg(feature = "jpeg")]
fn refuses_jpeg_below_16_bits_per_pixel() {
    // rfbproto: JpegCompression may only be used with 16 or 32 bits per pixel.
    let grey8 = PixelFormat {
        bits_per_pixel: 8,
        depth: 8,
        red_max: 7,
        green_max: 7,
        blue_max: 3,
        red_shift: 0,
        green_shift: 3,
        blue_shift: 6,
        ..xrgb(false)
    };

    let result = decode(
        &mut Decoder::new(),
        rect(256, 64),
        &jpeg_rect(&recorded_jpeg()),
        &grey8,
    );

    assert_eq!(
        result,
        Err(DecodeError::Corrupt(
            "Tight JPEG needs 16 or 32 bits per pixel"
        ))
    );
}
