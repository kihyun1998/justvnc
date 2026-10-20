//! VNC authentication (security type 2): the DES response to a server's challenge.

use des::Des;
use des::cipher::{Array, BlockCipherEncrypt, KeyInit};

/// Size of the challenge and of the response.
pub const CHALLENGE_LEN: usize = 16;

/// The response a client sends for `challenge` when it knows `password`.
pub fn response(password: &[u8], challenge: &[u8; CHALLENGE_LEN]) -> [u8; CHALLENGE_LEN] {
    let mut key = [0u8; 8];
    for (k, p) in key.iter_mut().zip(password) {
        *k = p.reverse_bits();
    }
    let des = Des::new(&Array::from(key));
    let mut out = *challenge;
    let (halves, _) = out.as_chunks_mut::<8>();
    for half in halves {
        let mut block = Array::from(*half);
        des.encrypt_block(&mut block);
        half.copy_from_slice(&block);
    }
    out
}

/// Whether `received` is the response to `challenge` for `password`, compared in constant time.
pub fn verify(
    password: &[u8],
    challenge: &[u8; CHALLENGE_LEN],
    received: &[u8; CHALLENGE_LEN],
) -> bool {
    let expected = response(password, challenge);
    expected
        .iter()
        .zip(received)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
