//! Pixel decoding for RFB encodings.

use justvnc_proto::{PixelFormat, Rect, encoding};

/// Why a framed rectangle could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// No decoder exists for this encoding.
    UnsupportedEncoding { encoding: i32 },
    /// The framed data does not have the size the encoding requires.
    WrongLength { expected: usize, actual: usize },
}

/// Turns framed rectangles into pixels. Holds the state some encodings keep across a connection.
#[derive(Debug, Default)]
pub struct Decoder {}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes one framed rectangle into `rect.width * rect.height` pixels in `format`, row by
    /// row from the top left.
    pub fn decode(
        &mut self,
        rect: Rect,
        encoding: i32,
        data: &[u8],
        format: &PixelFormat,
    ) -> Result<Vec<u8>, DecodeError> {
        match encoding {
            encoding::RAW => {
                let expected =
                    usize::from(rect.width) * usize::from(rect.height) * format.bytes_per_pixel();
                if data.len() != expected {
                    return Err(DecodeError::WrongLength {
                        expected,
                        actual: data.len(),
                    });
                }
                Ok(data.to_vec())
            }
            _ => Err(DecodeError::UnsupportedEncoding { encoding }),
        }
    }
}
