//! Framing of server-to-client messages: where each message and rectangle ends.

use crate::{CutText, Limits, MessageError, PixelFormat, Rect, encoding};

/// One framed piece of the server's message stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Framed {
    /// A rectangle of pixel data, still in its encoding.
    Rectangle {
        rect: Rect,
        encoding: i32,
        data: Vec<u8>,
    },
    /// Copy the pixels at (`src_x`, `src_y`), as big as `rect`, to `rect` (CopyRect).
    Copy { rect: Rect, src_x: u16, src_y: u16 },
    /// The framebuffer is now `width` x `height` (DesktopSize pseudo-encoding).
    DesktopSize { width: u16, height: u16 },
    /// The last rectangle of a FramebufferUpdate has been framed.
    UpdateEnd,
    /// Bell.
    Bell,
    /// ServerCutText.
    CutText(CutText),
}

/// Frames the server's messages after ServerInit, one piece at a time.
#[derive(Debug)]
pub struct ServerMessageReader {
    bytes_per_pixel: usize,
    width: u16,
    height: u16,
    limits: Limits,
    rectangles_left: Option<u16>,
}

const FRAMEBUFFER_UPDATE: u8 = 0;
const BELL: u8 = 2;
const SERVER_CUT_TEXT: u8 = 3;

const UPDATE_HEADER: usize = 4;
const RECT_HEADER: usize = 12;
const CUT_TEXT_HEADER: usize = 8;
const COPY_RECT_DATA: usize = 4;

impl ServerMessageReader {
    /// A reader for a `width` x `height` framebuffer whose pixels use `format`.
    pub fn new(format: &PixelFormat, width: u16, height: u16, limits: Limits) -> Self {
        Self {
            bytes_per_pixel: format.bytes_per_pixel(),
            width,
            height,
            limits,
            rectangles_left: None,
        }
    }

    /// Frames the next piece at the start of `buf`. Returns it and how many bytes of `buf` it
    /// used, or `None` when `buf` does not hold all of it yet.
    pub fn next(&mut self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        match self.rectangles_left {
            None => self.read_message(buf),
            Some(0) => {
                self.rectangles_left = None;
                Ok(Some((Framed::UpdateEnd, 0)))
            }
            Some(left) => {
                let framed = self.read_rectangle(buf)?;
                if framed.is_some() {
                    self.rectangles_left = Some(left - 1);
                }
                Ok(framed)
            }
        }
    }

    fn read_message(&mut self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        let Some(&message_type) = buf.first() else {
            return Ok(None);
        };
        match message_type {
            FRAMEBUFFER_UPDATE => self.read_update_header(buf),
            BELL => Ok(Some((Framed::Bell, 1))),
            SERVER_CUT_TEXT => self.read_cut_text(buf),
            _ => Err(MessageError::UnsupportedMessage { message_type }),
        }
    }

    fn read_cut_text(&self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        let Some(header) = buf.first_chunk::<CUT_TEXT_HEADER>() else {
            return Ok(None);
        };
        let length = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let max = self.limits.max_cut_text_length;
        if length > max {
            return Err(MessageError::LimitExceeded {
                field: "cut-text-length",
                length,
                max,
            });
        }
        let end = CUT_TEXT_HEADER + length as usize;
        let Some(text) = buf.get(CUT_TEXT_HEADER..end) else {
            return Ok(None);
        };
        Ok(Some((
            Framed::CutText(CutText::from_latin1(text.to_vec())),
            end,
        )))
    }

    fn read_update_header(&mut self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        let Some(header) = buf.first_chunk::<UPDATE_HEADER>() else {
            return Ok(None);
        };
        let count = u16::from_be_bytes([header[2], header[3]]);
        let max = self.limits.max_rectangles_per_update;
        if u32::from(count) > max {
            return Err(MessageError::LimitExceeded {
                field: "number-of-rectangles",
                length: u32::from(count),
                max,
            });
        }
        self.rectangles_left = Some(count);
        match self.next(&buf[UPDATE_HEADER..]) {
            Ok(Some((framed, used))) => Ok(Some((framed, UPDATE_HEADER + used))),
            other => {
                // Nothing past the header was taken, so it is read again next time.
                self.rectangles_left = None;
                other
            }
        }
    }

    fn read_rectangle(&mut self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        let Some(header) = buf.first_chunk::<RECT_HEADER>() else {
            return Ok(None);
        };
        let field = |i: usize| u16::from_be_bytes([header[i], header[i + 1]]);
        let rect = Rect {
            x: field(0),
            y: field(2),
            width: field(4),
            height: field(6),
        };
        let encoding = i32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        match encoding {
            encoding::DESKTOP_SIZE => {
                self.limits.check_framebuffer(rect.width, rect.height)?;
                self.width = rect.width;
                self.height = rect.height;
                let framed = Framed::DesktopSize {
                    width: rect.width,
                    height: rect.height,
                };
                Ok(Some((framed, RECT_HEADER)))
            }
            encoding::RAW => {
                if !rect.fits_in(self.width, self.height) {
                    return Err(MessageError::RectangleOutOfBounds { rect });
                }
                let len = usize::from(rect.width) * usize::from(rect.height) * self.bytes_per_pixel;
                let Some(data) = buf.get(RECT_HEADER..RECT_HEADER + len) else {
                    return Ok(None);
                };
                let framed = Framed::Rectangle {
                    rect,
                    encoding,
                    data: data.to_vec(),
                };
                Ok(Some((framed, RECT_HEADER + len)))
            }
            encoding::ZRLE => {
                if !rect.fits_in(self.width, self.height) {
                    return Err(MessageError::RectangleOutOfBounds { rect });
                }
                let Some(length) = buf.get(RECT_HEADER..RECT_HEADER + 4) else {
                    return Ok(None);
                };
                let length = u32::from_be_bytes([length[0], length[1], length[2], length[3]]);
                let max = self.limits.max_compressed_length;
                if length > max {
                    return Err(MessageError::LimitExceeded {
                        field: "zrle-length",
                        length,
                        max,
                    });
                }
                let start = RECT_HEADER + 4;
                let Some(data) = buf.get(start..start + length as usize) else {
                    return Ok(None);
                };
                let framed = Framed::Rectangle {
                    rect,
                    encoding,
                    data: data.to_vec(),
                };
                Ok(Some((framed, start + length as usize)))
            }
            encoding::COPY_RECT => {
                let Some(src) = buf.get(RECT_HEADER..RECT_HEADER + COPY_RECT_DATA) else {
                    return Ok(None);
                };
                let source = Rect {
                    x: u16::from_be_bytes([src[0], src[1]]),
                    y: u16::from_be_bytes([src[2], src[3]]),
                    ..rect
                };
                for r in [rect, source] {
                    if !r.fits_in(self.width, self.height) {
                        return Err(MessageError::RectangleOutOfBounds { rect: r });
                    }
                }
                let framed = Framed::Copy {
                    rect,
                    src_x: source.x,
                    src_y: source.y,
                };
                Ok(Some((framed, RECT_HEADER + COPY_RECT_DATA)))
            }
            _ => Err(MessageError::UnsupportedEncoding { encoding }),
        }
    }
}
