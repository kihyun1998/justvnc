//! Framing of server-to-client messages: where each message and rectangle ends.

use crate::{CutText, Limits, MessageError, PixelFormat, Rect, encoding, tight};

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
    tpixel: usize,
    width: u16,
    height: u16,
    limits: Limits,
    last_rect: bool,
    rectangles: Rectangles,
}

/// Where the reader is within a FramebufferUpdate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rectangles {
    /// Between messages.
    None,
    /// This many rectangles are still to come.
    Left(u16),
    /// The update said 0xFFFF and ends at a LastRect; this many have come so far.
    UntilLastRect(u32),
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
            tpixel: tight::tpixel_len(format),
            width,
            height,
            limits,
            last_rect: false,
            rectangles: Rectangles::None,
        }
    }

    /// Accepts the LastRect pseudo-encoding: for a client that offered it, a LastRect rectangle
    /// ends an update, and a count of 0xFFFF means "until the LastRect".
    pub fn with_last_rect(mut self) -> Self {
        self.last_rect = true;
        self
    }

    /// Frames the next piece at the start of `buf`. Returns it and how many bytes of `buf` it
    /// used, or `None` when `buf` does not hold all of it yet.
    pub fn next(&mut self, buf: &[u8]) -> Result<Option<(Framed, usize)>, MessageError> {
        match self.rectangles {
            Rectangles::None => self.read_message(buf),
            Rectangles::Left(0) => {
                self.rectangles = Rectangles::None;
                Ok(Some((Framed::UpdateEnd, 0)))
            }
            Rectangles::Left(_) | Rectangles::UntilLastRect(_) => {
                let Some((framed, used)) = self.read_rectangle(buf)? else {
                    return Ok(None);
                };
                self.rectangles = match (framed == Framed::UpdateEnd, self.rectangles) {
                    (true, _) => Rectangles::None,
                    (false, Rectangles::Left(left)) => Rectangles::Left(left - 1),
                    (false, Rectangles::UntilLastRect(seen)) => {
                        let max = self.limits.max_rectangles_per_update;
                        if seen + 1 > max {
                            return Err(MessageError::LimitExceeded {
                                field: "number-of-rectangles",
                                length: seen + 1,
                                max,
                            });
                        }
                        Rectangles::UntilLastRect(seen + 1)
                    }
                    (false, Rectangles::None) => unreachable!(),
                };
                Ok(Some((framed, used)))
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
        let end = CUT_TEXT_HEADER.saturating_add(length as usize);
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
        self.rectangles = if count == 0xFFFF && self.last_rect {
            Rectangles::UntilLastRect(0)
        } else {
            let max = self.limits.max_rectangles_per_update;
            if u32::from(count) > max {
                return Err(MessageError::LimitExceeded {
                    field: "number-of-rectangles",
                    length: u32::from(count),
                    max,
                });
            }
            Rectangles::Left(count)
        };
        match self.next(&buf[UPDATE_HEADER..]) {
            Ok(Some((framed, used))) => Ok(Some((framed, UPDATE_HEADER + used))),
            other => {
                // Nothing past the header was taken, so it is read again next time.
                self.rectangles = Rectangles::None;
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
                let end = usize::from(rect.width)
                    .saturating_mul(usize::from(rect.height))
                    .saturating_mul(self.bytes_per_pixel)
                    .saturating_add(RECT_HEADER);
                let Some(data) = buf.get(RECT_HEADER..end) else {
                    return Ok(None);
                };
                let framed = Framed::Rectangle {
                    rect,
                    encoding,
                    data: data.to_vec(),
                };
                Ok(Some((framed, end)))
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
                let end = start.saturating_add(length as usize);
                let Some(data) = buf.get(start..end) else {
                    return Ok(None);
                };
                let framed = Framed::Rectangle {
                    rect,
                    encoding,
                    data: data.to_vec(),
                };
                Ok(Some((framed, end)))
            }
            encoding::LAST_RECT if self.last_rect => Ok(Some((Framed::UpdateEnd, RECT_HEADER))),
            encoding::TIGHT => {
                if !rect.fits_in(self.width, self.height) {
                    return Err(MessageError::RectangleOutOfBounds { rect });
                }
                let data = &buf[RECT_HEADER..];
                let Some(len) = tight::frame(data, rect, self.tpixel, &self.limits)? else {
                    return Ok(None);
                };
                let framed = Framed::Rectangle {
                    rect,
                    encoding,
                    data: data[..len].to_vec(),
                };
                Ok(Some((framed, RECT_HEADER + len)))
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
