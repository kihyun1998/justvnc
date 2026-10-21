//! Tight (encoding 7) without JPEG: fill, and basic compression with the copy, palette and
//! gradient filters, on four zlib streams.

use flate2::{Decompress, FlushDecompress, Status};
use justvnc_proto::tight::{self, FILTER_COPY, FILTER_GRADIENT, FILTER_PALETTE, Method};
use justvnc_proto::{PixelFormat, Rect};

use crate::DecodeError;

/// The connection's Tight state: four zlib streams, each continued by every rectangle that uses
/// it until a compression-control byte resets it.
pub(crate) struct TightDecoder {
    streams: [Decompress; 4],
}

impl std::fmt::Debug for TightDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TightDecoder").finish_non_exhaustive()
    }
}

impl TightDecoder {
    pub(crate) fn new() -> Self {
        Self {
            streams: std::array::from_fn(|_| Decompress::new(true)),
        }
    }

    /// Decodes one framed Tight rectangle (`data` starts at its compression-control byte).
    pub(crate) fn decode(
        &mut self,
        rect: Rect,
        data: &[u8],
        format: &PixelFormat,
    ) -> Result<Vec<u8>, DecodeError> {
        let mut reader = Reader { data, at: 0 };
        let control = reader.byte()?;
        for (bit, stream) in self.streams.iter_mut().enumerate() {
            if control & (1 << bit) != 0 {
                *stream = Decompress::new(true);
            }
        }
        let pixels = TPixels::for_format(format);
        let count = usize::from(rect.width) * usize::from(rect.height);
        let method = Method::from_control(control)
            .map_err(|_| DecodeError::Corrupt("Tight compression-control"))?;
        let (stream, filter) = match method {
            Method::Fill => {
                let pixel = pixels.read(reader.take(pixels.len)?);
                return Ok(pixel.repeat(count));
            }
            Method::Jpeg => return jpeg(&mut reader, rect, format),
            Method::Basic { stream, filter_id } => (
                stream,
                if filter_id {
                    reader.byte()?
                } else {
                    FILTER_COPY
                },
            ),
        };
        let width = usize::from(rect.width);
        match filter {
            FILTER_COPY => {
                let raw = self.filtered(&mut reader, stream, count * pixels.len)?;
                Ok(raw
                    .chunks_exact(pixels.len)
                    .flat_map(|p| pixels.read(p))
                    .collect())
            }
            FILTER_PALETTE => {
                let colours = usize::from(reader.byte()?) + 1;
                let palette: Vec<Vec<u8>> = (0..colours)
                    .map(|_| Ok(pixels.read(reader.take(pixels.len)?)))
                    .collect::<Result<_, DecodeError>>()?;
                let row_bytes = width.div_ceil(8);
                let size = if colours == 2 {
                    row_bytes * usize::from(rect.height)
                } else {
                    count
                };
                let indices = self.filtered(&mut reader, stream, size)?;
                let mut out = Vec::with_capacity(count * format.bytes_per_pixel());
                for i in 0..count {
                    let index = if colours == 2 {
                        let (row, col) = (i / width, i % width);
                        (indices[row * row_bytes + col / 8] >> (7 - col % 8)) & 1
                    } else {
                        indices[i]
                    };
                    let colour = palette
                        .get(usize::from(index))
                        .ok_or(DecodeError::Corrupt("Tight palette index out of range"))?;
                    out.extend_from_slice(colour);
                }
                Ok(out)
            }
            FILTER_GRADIENT => {
                if !matches!(format.bits_per_pixel, 16 | 32) {
                    return Err(DecodeError::Corrupt(
                        "Tight gradient filter needs 16 or 32 bits per pixel",
                    ));
                }
                let raw = self.filtered(&mut reader, stream, count * pixels.len)?;
                Ok(pixels.ungradient(&raw, width))
            }
            _ => Err(DecodeError::Corrupt("Tight filter-id")),
        }
    }

    /// The `size` bytes of filtered data: as is when shorter than 12 bytes, otherwise a compact
    /// length and zlib data on `stream` that must inflate to exactly `size` bytes.
    fn filtered(
        &mut self,
        reader: &mut Reader,
        stream: usize,
        size: usize,
    ) -> Result<Vec<u8>, DecodeError> {
        if size < tight::MIN_TO_COMPRESS {
            return Ok(reader.take(size)?.to_vec());
        }
        let (length, used) = tight::compact_length(&reader.data[reader.at..])
            .ok_or(DecodeError::Corrupt("Tight data ends early"))?;
        reader.at += used;
        let zlib = reader.take(length)?;
        let inflate = &mut self.streams[stream];
        let mut out = Vec::with_capacity(size + 1);
        let start = inflate.total_in();
        loop {
            let (consumed, produced) = ((inflate.total_in() - start) as usize, out.len());
            let status = inflate
                .decompress_vec(&zlib[consumed..], &mut out, FlushDecompress::Sync)
                .map_err(|_| DecodeError::Corrupt("Tight zlib stream is invalid"))?;
            if out.len() > size {
                return Err(DecodeError::Corrupt(
                    "Tight data inflates past its rectangle",
                ));
            }
            if (inflate.total_in() - start) as usize == zlib.len() && out.len() < out.capacity() {
                break;
            }
            let stuck = consumed == (inflate.total_in() - start) as usize && produced == out.len();
            if status == Status::StreamEnd || stuck {
                break;
            }
        }
        if out.len() != size {
            return Err(DecodeError::Corrupt(
                "Tight data inflates to the wrong size",
            ));
        }
        Ok(out)
    }
}

#[cfg(not(feature = "jpeg"))]
fn jpeg(_: &mut Reader, _: Rect, _: &PixelFormat) -> Result<Vec<u8>, DecodeError> {
    Err(DecodeError::Unsupported("Tight JPEG"))
}

/// A JpegCompression rectangle: a compact length and a JFIF image exactly `rect`'s size,
/// decoded to red, green, blue and put into `format` (16 or 32 bpp true colour only).
#[cfg(feature = "jpeg")]
fn jpeg(reader: &mut Reader, rect: Rect, format: &PixelFormat) -> Result<Vec<u8>, DecodeError> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    if !format.true_colour || !matches!(format.bits_per_pixel, 16 | 32) {
        return Err(DecodeError::Corrupt(
            "Tight JPEG needs 16 or 32 bits per pixel",
        ));
    }
    let (length, used) = tight::compact_length(&reader.data[reader.at..])
        .ok_or(DecodeError::Corrupt("Tight data ends early"))?;
    reader.at += used;
    let jfif = reader.take(length)?;
    let (width, height) = (usize::from(rect.width), usize::from(rect.height));
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(width.max(1))
        .set_max_height(height.max(1));
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(jfif), options);
    decoder
        .decode_headers()
        .map_err(|_| DecodeError::Corrupt("Tight JPEG is not a valid image"))?;
    let info = decoder
        .info()
        .ok_or(DecodeError::Corrupt("Tight JPEG is not a valid image"))?;
    if (usize::from(info.width), usize::from(info.height)) != (width, height) {
        return Err(DecodeError::Corrupt(
            "Tight JPEG is not the rectangle's size",
        ));
    }
    let rgb = decoder
        .decode()
        .map_err(|_| DecodeError::Corrupt("Tight JPEG is not a valid image"))?;
    let pixels = TPixels {
        len: 3,
        format: *format,
    };
    Ok(rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| pixels.rgb_to_pixel(p))
        .collect())
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
            .ok_or(DecodeError::Corrupt("Tight data ends early"))?;
        self.at += n;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
}

/// How TPIXELs map to pixels in the session's format.
struct TPixels {
    /// Bytes per TPIXEL: 3 (red, green, blue) or a whole pixel.
    len: usize,
    format: PixelFormat,
}

impl TPixels {
    fn for_format(format: &PixelFormat) -> Self {
        Self {
            len: tight::tpixel_len(format),
            format: *format,
        }
    }

    fn rgb(&self) -> bool {
        self.len == 3
    }

    /// The pixel, in the session's format, for the TPIXEL at the start of `bytes`.
    fn read(&self, bytes: &[u8]) -> Vec<u8> {
        if !self.rgb() {
            return bytes[..self.len].to_vec();
        }
        let f = &self.format;
        let value = u32::from(bytes[0]) << f.red_shift
            | u32::from(bytes[1]) << f.green_shift
            | u32::from(bytes[2]) << f.blue_shift;
        encode(value, f)
    }

    /// The pixel, in the session's format, for 8-bit red, green and blue, each scaled to the
    /// format's range.
    #[cfg(feature = "jpeg")]
    fn rgb_to_pixel(&self, rgb: &[u8; 3]) -> Vec<u8> {
        let f = &self.format;
        let scale = |v: u8, max: u16| (u32::from(v) * u32::from(max) + 127) / 255;
        let value = scale(rgb[0], f.red_max) << f.red_shift
            | scale(rgb[1], f.green_max) << f.green_shift
            | scale(rgb[2], f.blue_max) << f.blue_shift;
        encode(value, f)
    }

    /// Undoes the gradient filter on `raw` (one TPIXEL per pixel, `width` pixels per row): each
    /// colour component is its difference from left + above - above-left, clamped to the
    /// component's range, with pixels outside the rectangle taken as zero.
    fn ungradient(&self, raw: &[u8], width: usize) -> Vec<u8> {
        let f = &self.format;
        let maxes = [f.red_max, f.green_max, f.blue_max].map(u32::from);
        let shifts = [f.red_shift, f.green_shift, f.blue_shift];
        let components = |unit: &[u8]| -> [u32; 3] {
            if self.rgb() {
                [unit[0], unit[1], unit[2]].map(u32::from)
            } else {
                let value = decode(unit, f);
                std::array::from_fn(|c| (value >> shifts[c]) & maxes[c])
            }
        };
        let rows = raw.len() / self.len / width.max(1);
        let mut values = vec![[0u32; 3]; width * rows];
        for y in 0..rows {
            for x in 0..width {
                let diff = components(&raw[(y * width + x) * self.len..][..self.len]);
                let at = |dx: usize, dy: usize| {
                    if x < dx || y < dy {
                        [0; 3]
                    } else {
                        values[(y - dy) * width + (x - dx)]
                    }
                };
                let (left, up, up_left) = (at(1, 0), at(0, 1), at(1, 1));
                values[y * width + x] = std::array::from_fn(|c| {
                    let predicted = (i64::from(left[c]) + i64::from(up[c]) - i64::from(up_left[c]))
                        .clamp(0, i64::from(maxes[c])) as u32;
                    (predicted + diff[c]) & maxes[c]
                });
            }
        }
        values
            .iter()
            .flat_map(|v| {
                let value = (0..3).fold(0, |acc, c| acc | v[c] << shifts[c]);
                encode(value, f)
            })
            .collect()
    }
}

/// A pixel value as its wire bytes in `format`.
fn encode(value: u32, format: &PixelFormat) -> Vec<u8> {
    let bytes = format.bytes_per_pixel();
    let all = if format.big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    if format.big_endian {
        all[4 - bytes..].to_vec()
    } else {
        all[..bytes].to_vec()
    }
}

/// The pixel value in the wire bytes `unit` of `format`.
fn decode(unit: &[u8], format: &PixelFormat) -> u32 {
    let bytes = format.bytes_per_pixel();
    unit[..bytes].iter().enumerate().fold(0, |acc, (i, &b)| {
        let shift = if format.big_endian {
            8 * (bytes - 1 - i)
        } else {
            8 * i
        };
        acc | u32::from(b) << shift
    })
}
