//! Frames arbitrary bytes as the client's messages after ClientInit.
#![no_main]

use justvnc_proto::Limits;
use justvnc_proto::client_message::ClientMessageReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|stream: &[u8]| {
    // `whole` always sees the rest of the stream; `split` first sees half of it, which must
    // either frame the same message or leave the reader as if it had not been called.
    let mut whole = ClientMessageReader::new(Limits::default());
    let mut split = ClientMessageReader::new(Limits::default());
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
        let Ok(Some((_, used))) = expected else {
            break;
        };
        assert!(used > 0 && used <= rest.len());
        at += used;
    }
});
