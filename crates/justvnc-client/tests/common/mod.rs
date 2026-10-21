//! Helpers shared by the session tests.

/// `client` (bytes a client sent) with the SetEncodings message that starts at `at` cut out.
///
/// Recordings keep the SetEncodings the session sent when they were made. The list grows with
/// each encoding, and the server's answers stay valid, so tests compare everything else and
/// `handshake.rs` alone pins the list.
pub fn without_set_encodings(client: &[u8], at: usize) -> Vec<u8> {
    assert_eq!(client[at], 2, "no SetEncodings at byte {at}");
    let count = usize::from(u16::from_be_bytes([client[at + 2], client[at + 3]]));
    let end = at + 4 + 4 * count;
    [&client[..at], &client[end..]].concat()
}
