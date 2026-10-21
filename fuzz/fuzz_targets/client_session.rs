//! Runs a whole client session, decoding included, on arbitrary server bytes.
//!
//! Input: a flags byte, a chunk size (0: all at once), then the server's bytes. Flags: bit 0, a
//! password is set; bit 1, no JPEG quality is asked for; bits 2 and 3, the pixel format asked for
//! (the server's own, 16-bit, 8-bit, or 32-bit big-endian).
#![no_main]

use justvnc_client::{ClientConfig, ClientSession};
use justvnc_proto::{Password, PixelFormat};
use libfuzzer_sys::fuzz_target;

fn true_colour(bits_per_pixel: u8, big_endian: bool, max: [u16; 3], shift: [u8; 3]) -> PixelFormat {
    PixelFormat {
        bits_per_pixel,
        depth: bits_per_pixel.min(24),
        big_endian,
        true_colour: true,
        red_max: max[0],
        green_max: max[1],
        blue_max: max[2],
        red_shift: shift[0],
        green_shift: shift[1],
        blue_shift: shift[2],
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&[flags, chunk], server)) = data.split_first_chunk::<2>() else {
        return;
    };
    let pixel_format = match (flags >> 2) & 3 {
        0 => None,
        1 => Some(true_colour(16, false, [31, 63, 31], [11, 5, 0])),
        2 => Some(true_colour(8, false, [7, 7, 3], [0, 3, 6])),
        _ => Some(true_colour(32, true, [255, 255, 255], [16, 8, 0])),
    };
    let mut session = ClientSession::new(ClientConfig {
        password: (flags & 1 != 0).then(|| Password::new("justvnc-secret")),
        jpeg_quality: (flags & 2 == 0).then_some(8),
        pixel_format,
        ..ClientConfig::default()
    });
    let size = if chunk == 0 {
        server.len().max(1)
    } else {
        usize::from(chunk)
    };
    for piece in server.chunks(size) {
        let result = session.receive(piece);
        while session.poll_event().is_some() {}
        while session.poll_transmit().is_some() {}
        if result.is_err() {
            assert_eq!(session.receive(&[]), result, "the error did not stick");
            break;
        }
    }
});
