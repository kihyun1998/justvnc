//! Client-to-server messages, encoded for the wire.

use crate::{PixelFormat, Rect};

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
