//! Framing of Tight (encoding 7) rectangles: their size from explicit and compact-length fields,
//! never from inflating zlib data.

use crate::{Limits, MessageError, PixelFormat, Rect};

/// Widest rectangle Tight allows.
pub const MAX_WIDTH: u16 = 2048;
/// Filtered data shorter than this is sent as is, without zlib.
pub const MIN_TO_COMPRESS: usize = 12;

/// The compression method in a compression-control byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// BasicCompression on zlib stream `stream`, with the filter-id byte present or not.
    Basic {
        stream: usize,
        filter_id: bool,
    },
    Fill,
    Jpeg,
}

impl Method {
    /// Reads the method from a compression-control byte. The low 4 bits (stream resets) are not
    /// looked at.
    pub fn from_control(control: u8) -> Result<Self, MessageError> {
        match control >> 4 {
            0x8 => Ok(Self::Fill),
            0x9 => Ok(Self::Jpeg),
            high if high & 0x8 == 0 => Ok(Self::Basic {
                stream: usize::from(high & 0x3),
                filter_id: high & 0x4 != 0,
            }),
            // 0xA and 0xE (BasicCompression without zlib) need a pseudo-encoding never offered.
            _ => Err(MessageError::Malformed(
                "Tight compression-control is not valid",
            )),
        }
    }
}

pub const FILTER_COPY: u8 = 0;
pub const FILTER_PALETTE: u8 = 1;
pub const FILTER_GRADIENT: u8 = 2;

/// Bytes in a TPIXEL: 3 (red, green, blue) for true colour, 32 bpp, depth 24 and 8-bit
/// components; otherwise a whole pixel.
pub fn tpixel_len(format: &PixelFormat) -> usize {
    let rgb888 = format.true_colour
        && format.bits_per_pixel == 32
        && format.depth == 24
        && [format.red_max, format.green_max, format.blue_max] == [255; 3];
    if rgb888 { 3 } else { format.bytes_per_pixel() }
}

/// Reads a compact length (1 to 3 bytes) at the start of `buf`: the value and the bytes it took.
pub fn compact_length(buf: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0;
    for (n, shift) in [0, 7, 14].into_iter().enumerate() {
        let byte = *buf.get(n)?;
        let bits = if n == 2 { byte } else { byte & 0x7f };
        value |= usize::from(bits) << shift;
        if n == 2 || byte & 0x80 == 0 {
            return Some((value, n + 1));
        }
    }
    unreachable!()
}

/// How many bytes the Tight data at the start of `buf` takes, or `None` when `buf` does not hold
/// all of it yet.
pub fn frame(
    buf: &[u8],
    rect: Rect,
    tpixel: usize,
    limits: &Limits,
) -> Result<Option<usize>, MessageError> {
    if rect.width > MAX_WIDTH {
        return Err(MessageError::Malformed(
            "Tight rectangle is wider than 2048",
        ));
    }
    let Some(&control) = buf.first() else {
        return Ok(None);
    };
    let mut at = 1;
    let filtered = match Method::from_control(control)? {
        Method::Fill => return Ok((buf.len() > tpixel).then_some(1 + tpixel)),
        Method::Jpeg => None,
        Method::Basic { filter_id, .. } => {
            let filter = if filter_id {
                let Some(&id) = buf.get(at) else {
                    return Ok(None);
                };
                at += 1;
                id
            } else {
                FILTER_COPY
            };
            let (w, h) = (usize::from(rect.width), usize::from(rect.height));
            Some(match filter {
                FILTER_COPY | FILTER_GRADIENT => w * h * tpixel,
                FILTER_PALETTE => {
                    let Some(&count) = buf.get(at) else {
                        return Ok(None);
                    };
                    let colours = usize::from(count) + 1;
                    at += 1 + colours * tpixel;
                    if colours == 2 {
                        w.div_ceil(8) * h
                    } else {
                        w * h
                    }
                }
                _ => return Err(MessageError::Malformed("Tight filter-id is not valid")),
            })
        }
    };
    let length = match filtered {
        Some(size) if size < MIN_TO_COMPRESS => size,
        _ => {
            let Some((length, used)) = buf.get(at..).and_then(compact_length) else {
                return Ok(None);
            };
            at += used;
            let max = limits.max_compressed_length;
            if length > max as usize {
                return Err(MessageError::LimitExceeded {
                    field: "tight-length",
                    length: length as u32,
                    max,
                });
            }
            length
        }
    };
    Ok((buf.len() >= at + length).then_some(at + length))
}
