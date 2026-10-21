//! Pixel decoding for RFB encodings.

mod tight;
mod zrle;

use justvnc_proto::{PixelFormat, Rect, encoding};

use crate::tight::TightDecoder;
use crate::zrle::ZrleDecoder;

/// Why a framed rectangle could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// No decoder exists for this encoding.
    UnsupportedEncoding { encoding: i32 },
    /// The framed data does not have the size the encoding requires.
    WrongLength { expected: usize, actual: usize },
    /// The data breaks the encoding's rules, as described.
    Corrupt(&'static str),
    /// The data uses a part of the encoding not supported yet, as named.
    Unsupported(&'static str),
}

/// Turns framed rectangles into pixels. Holds the state some encodings keep across a connection.
#[derive(Debug)]
pub struct Decoder {
    zrle: ZrleDecoder,
    tight: TightDecoder,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            zrle: ZrleDecoder::new(),
            tight: TightDecoder::new(),
        }
    }

    /// Decodes one framed rectangle into `rect.width * rect.height` pixels in `format`, row by
    /// row from the top left. Rectangles must arrive in the order the server sent them.
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
            encoding::ZRLE => self.zrle.decode(rect, data, format),
            encoding::TIGHT => self.tight.decode(rect, data, format),
            _ => Err(DecodeError::UnsupportedEncoding { encoding }),
        }
    }
}
