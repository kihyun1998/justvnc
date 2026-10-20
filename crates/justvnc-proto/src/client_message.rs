//! Client-to-server messages, encoded for the wire.

use crate::{CutText, PixelFormat, Rect};

/// SetPixelFormat (message type 0).
pub fn set_pixel_format(format: &PixelFormat) -> [u8; 20] {
    let mut out = [0u8; 20];
    out[4..].copy_from_slice(&format.to_bytes());
    out
}

/// SetEncodings (message type 2), most preferred first.
///
/// # Panics
///
/// If there are more than 65535 encodings.
pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let count = u16::try_from(encodings.len()).expect("at most 65535 encodings");
    let mut out = vec![2, 0];
    out.extend_from_slice(&count.to_be_bytes());
    for encoding in encodings {
        out.extend_from_slice(&encoding.to_be_bytes());
    }
    out
}

/// FramebufferUpdateRequest (message type 3).
pub fn framebuffer_update_request(incremental: bool, area: Rect) -> [u8; 10] {
    let mut out = [3, u8::from(incremental), 0, 0, 0, 0, 0, 0, 0, 0];
    out[2..4].copy_from_slice(&area.x.to_be_bytes());
    out[4..6].copy_from_slice(&area.y.to_be_bytes());
    out[6..8].copy_from_slice(&area.width.to_be_bytes());
    out[8..10].copy_from_slice(&area.height.to_be_bytes());
    out
}

/// KeyEvent (message type 4): `keysym` pressed or released.
pub fn key_event(down: bool, keysym: u32) -> [u8; 8] {
    let [a, b, c, d] = keysym.to_be_bytes();
    [4, u8::from(down), 0, 0, a, b, c, d]
}

/// PointerEvent (message type 5): the pointer at (`x`, `y`) with buttons 1 to 8 in bits 0 to 7.
pub fn pointer_event(buttons: u8, x: u16, y: u16) -> [u8; 6] {
    let [xh, xl] = x.to_be_bytes();
    let [yh, yl] = y.to_be_bytes();
    [5, buttons, xh, xl, yh, yl]
}

/// ClientCutText (message type 6).
///
/// # Panics
///
/// If the text is longer than `u32::MAX` bytes.
pub fn client_cut_text(text: &CutText) -> Vec<u8> {
    let bytes = text.as_bytes();
    let length = u32::try_from(bytes.len()).expect("cut text fits a U32 length");
    let mut out = vec![6, 0, 0, 0];
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
    out
}
