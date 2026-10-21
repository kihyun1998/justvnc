//! ZRLE (encoding 16): zlib-compressed 64x64 tiles, decoded with one zlib stream per connection.

use flate2::{Decompress, FlushDecompress, Status};
use justvnc_proto::{PixelFormat, Rect};

use crate::DecodeError;

const TILE: usize = 64;

/// The connection's ZRLE state: the zlib stream every ZRLE rectangle continues.
pub(crate) struct ZrleDecoder {
    inflate: Decompress,
}

impl std::fmt::Debug for ZrleDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZrleDecoder")
            .field("total_in", &self.inflate.total_in())
            .finish()
    }
}

impl ZrleDecoder {
    pub(crate) fn new() -> Self {
        Self {
            inflate: Decompress::new(true),
        }
    }

    pub(crate) fn decode(
        &mut self,
        rect: Rect,
        data: &[u8],
        format: &PixelFormat,
    ) -> Result<Vec<u8>, DecodeError> {
        let cpixel = CPixel::for_format(format);
        let (width, height) = (usize::from(rect.width), usize::from(rect.height));
        let tiles = width.div_ceil(TILE) * height.div_ceil(TILE);
        // The most a valid rectangle can inflate to: every tile a 127-colour palette, then every
        // pixel either a raw CPIXEL or a one-pixel run of one CPIXEL and one length byte.
        let max = tiles * (1 + 127 * cpixel.len) + width * height * (cpixel.len + 1);
        let tiles_data = self.inflate(data, max)?;
        let mut out = vec![0u8; width * height * format.bytes_per_pixel()];
        let mut reader = Reader {
            data: &tiles_data,
            at: 0,
        };
        for ty in (0..height).step_by(TILE) {
            for tx in (0..width).step_by(TILE) {
                let tile = Tile {
                    x: tx,
                    y: ty,
                    width: TILE.min(width - tx),
                    height: TILE.min(height - ty),
                };
                decode_tile(&mut reader, tile, &cpixel, &mut out, width)?;
            }
        }
        if reader.at != tiles_data.len() {
            return Err(DecodeError::Corrupt(
                "ZRLE data continues after the last tile",
            ));
        }
        Ok(out)
    }

    /// Inflates one rectangle's zlib data, continuing the connection's stream, refusing to
    /// produce more than `max` bytes.
    fn inflate(&mut self, data: &[u8], max: usize) -> Result<Vec<u8>, DecodeError> {
        const TOO_LARGE: DecodeError =
            DecodeError::Corrupt("ZRLE data inflates past its rectangle");
        let start = self.inflate.total_in();
        let consumed = |inflate: &Decompress| (inflate.total_in() - start) as usize;
        let mut out = Vec::new();
        loop {
            if out.len() == out.capacity() {
                // Room for one byte past `max` at most, so going over it is seen.
                let grow = out.capacity().max(4096).min(max + 1 - out.len());
                out.reserve_exact(grow);
            }
            let input = &data[consumed(&self.inflate)..];
            let status = self
                .inflate
                .decompress_vec(input, &mut out, FlushDecompress::Sync)
                .map_err(|_| DecodeError::Corrupt("ZRLE zlib stream is invalid"))?;
            if out.len() > max {
                return Err(TOO_LARGE);
            }
            let all_in = consumed(&self.inflate) == data.len();
            match status {
                Status::StreamEnd => return Ok(out),
                // Every input byte used and room left over: nothing more is pending.
                _ if all_in && out.len() < out.capacity() => return Ok(out),
                Status::BufError if out.len() < out.capacity() => {
                    return Err(DecodeError::Corrupt("ZRLE zlib stream ends early"));
                }
                _ => {}
            }
        }
    }
}

/// How a CPIXEL maps to the bytes of a pixel in the session's format.
struct CPixel {
    len: usize,
    /// Where the CPIXEL's bytes go within the pixel's bytes.
    offset: usize,
    pixel_len: usize,
}

impl CPixel {
    /// The CPIXEL layout for `format` (RFC 6143 7.7.5): 3 of the pixel's 4 wire bytes, first or
    /// last, or the whole pixel.
    fn for_format(format: &PixelFormat) -> Self {
        let pixel_len = format.bytes_per_pixel();
        let full = Self {
            len: pixel_len,
            offset: 0,
            pixel_len,
        };
        if !format.true_colour || format.bits_per_pixel != 32 || format.depth > 24 {
            return full;
        }
        let fits_low = [
            (format.red_max, format.red_shift),
            (format.green_max, format.green_shift),
            (format.blue_max, format.blue_shift),
        ]
        .iter()
        .all(|&(max, shift)| u64::from(max) << shift < 1 << 24);
        let fits_high = format.red_shift > 7 && format.green_shift > 7 && format.blue_shift > 7;
        let first_three = (fits_low && !format.big_endian) || (fits_high && format.big_endian);
        let last_three = (fits_low && format.big_endian) || (fits_high && !format.big_endian);
        match (first_three, last_three) {
            (true, _) => Self {
                len: 3,
                offset: 0,
                pixel_len,
            },
            (false, true) => Self {
                len: 3,
                offset: 1,
                pixel_len,
            },
            _ => full,
        }
    }

    /// The pixel bytes for the CPIXEL at the start of `bytes`.
    fn expand(&self, bytes: &[u8]) -> [u8; 4] {
        let mut pixel = [0u8; 4];
        pixel[self.offset..self.offset + self.len].copy_from_slice(&bytes[..self.len]);
        pixel
    }
}

#[derive(Clone, Copy)]
struct Tile {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        let bytes = self
            .data
            .get(self.at..self.at + n)
            .ok_or(DecodeError::Corrupt("ZRLE tile data ends early"))?;
        self.at += n;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// A run length: one more than the sum of its bytes, ending at the first byte that is not 255.
    fn run_length(&mut self) -> Result<usize, DecodeError> {
        let mut length = 1;
        loop {
            let byte = self.byte()?;
            length += usize::from(byte);
            if byte != 255 {
                return Ok(length);
            }
        }
    }

    fn palette(&mut self, size: usize, cpixel: &CPixel) -> Result<Vec<[u8; 4]>, DecodeError> {
        (0..size)
            .map(|_| Ok(cpixel.expand(self.take(cpixel.len)?)))
            .collect()
    }
}

fn decode_tile(
    reader: &mut Reader,
    tile: Tile,
    cpixel: &CPixel,
    out: &mut [u8],
    rect_width: usize,
) -> Result<(), DecodeError> {
    let mut put = |i: usize, pixel: &[u8; 4]| {
        let (x, y) = (tile.x + i % tile.width, tile.y + i / tile.width);
        let at = (y * rect_width + x) * cpixel.pixel_len;
        out[at..at + cpixel.pixel_len].copy_from_slice(&pixel[..cpixel.pixel_len]);
    };
    let count = tile.width * tile.height;
    match reader.byte()? {
        0 => {
            for i in 0..count {
                put(i, &cpixel.expand(reader.take(cpixel.len)?));
            }
        }
        1 => {
            let pixel = cpixel.expand(reader.take(cpixel.len)?);
            (0..count).for_each(|i| put(i, &pixel));
        }
        size @ 2..=16 => {
            let palette = reader.palette(usize::from(size), cpixel)?;
            let bits = match size {
                2 => 1,
                3 | 4 => 2,
                _ => 4,
            };
            let row_bytes = (tile.width * bits).div_ceil(8);
            for row in 0..tile.height {
                let packed = reader.take(row_bytes)?;
                for col in 0..tile.width {
                    let bit = col * bits;
                    let index = (packed[bit / 8] >> (8 - bits - bit % 8)) & ((1 << bits) - 1);
                    let pixel = palette
                        .get(usize::from(index))
                        .ok_or(DecodeError::Corrupt("ZRLE palette index out of range"))?;
                    put(row * tile.width + col, pixel);
                }
            }
        }
        128 => {
            let mut i = 0;
            while i < count {
                let pixel = cpixel.expand(reader.take(cpixel.len)?);
                let end = i + reader.run_length()?;
                if end > count {
                    return Err(DecodeError::Corrupt("ZRLE run goes past its tile"));
                }
                (i..end).for_each(|j| put(j, &pixel));
                i = end;
            }
        }
        sub @ 130..=255 => {
            let palette = reader.palette(usize::from(sub - 128), cpixel)?;
            let mut i = 0;
            while i < count {
                let index = reader.byte()?;
                let pixel = palette
                    .get(usize::from(index & 0x7f))
                    .ok_or(DecodeError::Corrupt("ZRLE palette index out of range"))?;
                let end = i + if index & 0x80 != 0 {
                    reader.run_length()?
                } else {
                    1
                };
                if end > count {
                    return Err(DecodeError::Corrupt("ZRLE run goes past its tile"));
                }
                (i..end).for_each(|j| put(j, pixel));
                i = end;
            }
        }
        _ => {
            return Err(DecodeError::Corrupt(
                "ZRLE subencoding is unused or not allowed",
            ));
        }
    }
    Ok(())
}
