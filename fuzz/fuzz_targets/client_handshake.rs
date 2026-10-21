//! Runs the client side of the handshake on arbitrary server bytes.
//!
//! Input: a flags byte (bit 0: a password is set), a chunk size (0: all at once), then the
//! server's bytes.
#![no_main]

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::{HandshakeError, Limits, Password, ServerInit};
use libfuzzer_sys::fuzz_target;

/// Where the handshake ended up: what it returned, what it sent and what it left over.
type Outcome = (Result<Option<ServerInit>, HandshakeError>, Vec<u8>, Vec<u8>);

fn run(flags: u8, chunks: impl Iterator<Item = impl AsRef<[u8]>>) -> Outcome {
    let password = (flags & 1 != 0).then(|| Password::new("justvnc-secret"));
    let mut handshake = ClientHandshake::new(false, password, Limits::default());
    let mut transmit = Vec::new();
    let mut result = Ok(None);
    for chunk in chunks {
        result = handshake.receive(chunk.as_ref());
        transmit.extend(handshake.take_transmit());
        if !matches!(result, Ok(None)) {
            break;
        }
    }
    (result, transmit, handshake.take_remaining())
}

fuzz_target!(|data: &[u8]| {
    let Some((&[flags, chunk], server)) = data.split_first_chunk::<2>() else {
        return;
    };
    let at_once = run(flags, std::iter::once(server));
    if chunk != 0 {
        let (result, transmit, remaining) = run(flags, server.chunks(usize::from(chunk)));
        assert_eq!(
            (result, transmit),
            (at_once.0, at_once.1),
            "the handshake depends on how its bytes arrive"
        );
        // Chunks after the one that completed ServerInit were never fed.
        assert!(at_once.2.starts_with(&remaining));
    }
});
