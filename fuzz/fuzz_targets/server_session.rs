//! Runs a whole server session on arbitrary Viewer bytes, against a small test pattern.
//!
//! Input: a flags byte, a chunk size (0: all at once), then the Viewer's bytes. Flags: bits 0
//! and 1, the version offered (3.3, 3.7, 3.8); bit 2, VNC authentication instead of None;
//! bit 3, authentication accepted; bit 4, a change reported after each chunk.
#![no_main]

use justvnc_proto::server::SecurityType;
use justvnc_proto::{PixelFormat, ProtocolVersion, Rect};
use justvnc_server::{Event, Framebuffer, ServerConfig, ServerSession};
use libfuzzer_sys::fuzz_target;

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

struct Pattern;

impl Framebuffer for Pattern {
    fn size(&self) -> (u16, u16) {
        (64, 32)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        assert!(u32::from(rect.x) + u32::from(rect.width) <= 64, "{rect:?}");
        assert!(u32::from(rect.y) + u32::from(rect.height) <= 32, "{rect:?}");
        vec![0x80; usize::from(rect.width) * usize::from(rect.height) * 4]
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&[flags, chunk], viewer)) = data.split_first_chunk::<2>() else {
        return;
    };
    let version = match flags & 3 {
        0 => ProtocolVersion::V3_3,
        1 => ProtocolVersion::V3_7,
        _ => ProtocolVersion::V3_8,
    };
    let security = if flags & 4 != 0 {
        SecurityType::VncAuth
    } else {
        SecurityType::None
    };
    let mut session = ServerSession::with_challenge(
        ServerConfig {
            version,
            security: vec![security],
            ..ServerConfig::default()
        },
        [0x11; 16],
    );
    let mut pattern = Pattern;
    let size = if chunk == 0 {
        viewer.len().max(1)
    } else {
        usize::from(chunk)
    };
    for (i, piece) in viewer.chunks(size).enumerate() {
        let mut result = session.receive(piece, &mut pattern);
        while let Some(event) = session.poll_event() {
            if let Event::Authenticate { .. } = event {
                if flags & 8 != 0 {
                    result = result.and(session.accept(&mut pattern));
                } else {
                    session.reject("no");
                }
            }
        }
        if flags & 16 != 0 {
            let at = i as u16;
            let rect = Rect {
                x: at % 70,
                y: at % 40,
                width: at % 13 + 1,
                height: at % 7 + 1,
            };
            session.damage(rect, &pattern);
        }
        while session.poll_transmit().is_some() {}
        if result.is_err() {
            assert_eq!(
                session.receive(&[], &mut pattern),
                result,
                "the error did not stick"
            );
            break;
        }
    }
});
