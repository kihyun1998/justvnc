//! Frames arbitrary bytes as the server's messages after ServerInit.
//!
//! Input: PIXEL_FORMAT (16 bytes), framebuffer width and height (U16 each), a flags byte (bit 0:
//! LastRect offered), then the message stream.
#![no_main]

use justvnc_proto::server_message::{Framed, ServerMessageReader};
use justvnc_proto::{Limits, PixelFormat};
use libfuzzer_sys::fuzz_target;

const HEAD: usize = PixelFormat::LEN + 2 + 2 + 1;

fn reader(head: &[u8; HEAD]) -> ServerMessageReader {
    let format = PixelFormat::from_bytes(head[..PixelFormat::LEN].try_into().unwrap());
    let width = u16::from_be_bytes([head[16], head[17]]);
    let height = u16::from_be_bytes([head[18], head[19]]);
    let reader = ServerMessageReader::new(&format, width, height, Limits::default());
    if head[20] & 1 != 0 {
        reader.with_last_rect()
    } else {
        reader
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((head, stream)) = data.split_first_chunk::<HEAD>() else {
        return;
    };
    // `whole` always sees the rest of the stream; `split` first sees half of it, which must
    // either frame the same piece or leave the reader as if it had not been called.
    let mut whole = reader(head);
    let mut split = reader(head);
    let mut at = 0;
    loop {
        let rest = &stream[at..];
        let expected = whole.next(rest);
        let early = split.next(&rest[..rest.len() / 2]);
        if early != Ok(None) {
            assert_eq!(early, expected, "half of the bytes framed differently");
        } else {
            assert_eq!(
                split.next(rest),
                expected,
                "waiting for bytes changed the reader"
            );
        }
        let Ok(Some((framed, used))) = expected else {
            break;
        };
        assert!(used <= rest.len());
        assert!(
            used > 0 || framed == Framed::UpdateEnd,
            "{framed:?} used no bytes"
        );
        at += used;
    }
});
