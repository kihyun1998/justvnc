//! Replaying a Recording through the client role's decoding.

use std::io::Read;
use std::time::Duration;

use justvnc_client::{Event, Rectangle};
use justvnc_decode::{DecodeError, Decoder};
use justvnc_proto::client_message::{ClientMessage, ClientMessageReader};
use justvnc_proto::server_message::{Framed, ServerMessageReader};
use justvnc_proto::{Limits, MessageError, PixelFormat, ServerInit, encoding};

use crate::{Direction, ReadError, RecordingReader};

/// Decodes a Recording from its start, entry by entry, following the Viewer's recorded
/// SetPixelFormat and SetEncodings where the Proxy applied them.
#[derive(Debug)]
pub struct Replay<R> {
    reader: RecordingReader<R>,
    target: ServerMessageReader,
    viewer: ClientMessageReader,
    decoder: Decoder,
    format: PixelFormat,
    failed: bool,
}

/// One recorded message, replayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The Viewer was shown this: the client role's events for one message from the Target.
    Shown { time: Duration, events: Vec<Event> },
    /// The Viewer sent this message to the Target.
    Sent {
        time: Duration,
        message: ClientMessage,
    },
}

/// Why a Recording could not be replayed.
#[derive(Debug)]
pub enum ReplayError {
    Read(ReadError),
    /// A recorded message that cannot be framed.
    Message(MessageError),
    Decode(DecodeError),
    /// A pixel format the client role cannot decode: not valid, or colour-mapped.
    UnsupportedPixelFormat(PixelFormat),
}

impl<R: Read> Replay<R> {
    /// Reads the Recording's header.
    pub fn new(input: R) -> Result<Self, ReadError> {
        let reader = RecordingReader::new(input)?;
        let desktop = reader.desktop();
        let format = desktop.pixel_format;
        let limits = Limits::default();
        Ok(Self {
            target: ServerMessageReader::new(&format, desktop.width, desktop.height, limits),
            viewer: ClientMessageReader::new(limits),
            decoder: Decoder::new(),
            format,
            failed: false,
            reader,
        })
    }

    /// The ServerInit the recorded session started with.
    pub fn desktop(&self) -> &ServerInit {
        self.reader.desktop()
    }

    /// The pixel format rectangles are decoded in, as the Viewer last asked.
    pub fn pixel_format(&self) -> PixelFormat {
        self.format
    }

    fn shown(&mut self, time: Duration, bytes: &[u8]) -> Result<Step, ReplayError> {
        let mut events = Vec::new();
        let mut at = 0;
        while let Some((framed, used)) = self
            .target
            .next(&bytes[at..])
            .map_err(ReplayError::Message)?
        {
            // Only the end of an update counted out by its rectangles, which takes no bytes,
            // may follow the message.
            if at > 0 && used > 0 {
                return Err(cut_short());
            }
            at += used;
            events.push(match framed {
                Framed::Rectangle {
                    rect,
                    encoding,
                    data,
                } => Event::Rectangle(Rectangle {
                    rect,
                    pixels: self
                        .decoder
                        .decode(rect, encoding, &data, &self.format)
                        .map_err(ReplayError::Decode)?,
                }),
                Framed::Copy { rect, src_x, src_y } => Event::Copy { rect, src_x, src_y },
                Framed::DesktopSize { width, height } => Event::DesktopResized { width, height },
                Framed::UpdateEnd => Event::UpdateComplete,
                Framed::Bell => Event::Bell,
                Framed::CutText(text) => Event::CutText(text),
            });
        }
        if at != bytes.len() {
            return Err(cut_short());
        }
        Ok(Step::Shown { time, events })
    }

    fn sent(&mut self, time: Duration, bytes: &[u8]) -> Result<Step, ReplayError> {
        let Some((message, used)) = self.viewer.next(bytes).map_err(ReplayError::Message)? else {
            return Err(cut_short());
        };
        if used != bytes.len() {
            return Err(cut_short());
        }
        match &message {
            ClientMessage::SetPixelFormat(format) => {
                if !format.true_colour || !format.is_valid() {
                    return Err(ReplayError::UnsupportedPixelFormat(*format));
                }
                self.target.set_pixel_format(format);
                self.format = *format;
            }
            ClientMessage::SetEncodings(encodings) => {
                self.target
                    .set_last_rect(encodings.contains(&encoding::LAST_RECT));
            }
            _ => {}
        }
        Ok(Step::Sent { time, message })
    }
}

/// The recorded messages, replayed in order; `None` at the end, or after an error.
impl<R: Read> Iterator for Replay<R> {
    type Item = Result<Step, ReplayError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let step = match self.reader.next()? {
            Ok(entry) => match entry.direction {
                Direction::ToViewer => self.shown(entry.time, &entry.bytes),
                Direction::ToTarget => self.sent(entry.time, &entry.bytes),
            },
            Err(e) => Err(ReplayError::Read(e)),
        };
        self.failed = step.is_err();
        Some(step)
    }
}

/// An entry holds exactly one message.
fn cut_short() -> ReplayError {
    ReplayError::Message(MessageError::Malformed(
        "a recorded entry does not hold exactly one message",
    ))
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => e.fmt(f),
            Self::Message(e) => e.fmt(f),
            Self::Decode(e) => e.fmt(f),
            Self::UnsupportedPixelFormat(format) => {
                write!(f, "pixel format not supported: {format}")
            }
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(e) => e.source(),
            Self::Message(e) => e.source(),
            Self::Decode(e) => e.source(),
            Self::UnsupportedPixelFormat(_) => None,
        }
    }
}
