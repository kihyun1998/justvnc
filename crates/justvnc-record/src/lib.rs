//! Recordings of RFB sessions: the framed messages that crossed after the handshake, each with
//! its direction and a time, written and read back (ADR-0005). With the `replay` feature,
//! [`Replay`] decodes a Recording into the client role's events; with `frames`, [`Frames`] turns it
//! into keyframes and changed regions, and [`export_frames`] writes them as PNG files.

use std::io::{self, Read, Write};
use std::time::Duration;

use justvnc_proto::{DesktopName, Limits, PixelFormat, ServerInit};

#[cfg(feature = "frames")]
mod frames;
#[cfg(feature = "replay")]
mod replay;
#[cfg(feature = "frames")]
pub use frames::{ExportError, Frame, FrameKind, Frames, FramesConfig, export_frames};
#[cfg(feature = "replay")]
pub use replay::{Replay, ReplayError, Step};
#[cfg(feature = "replay")]
pub use {justvnc_client::Event, justvnc_proto::client_message::ClientMessage};

const MAGIC: &[u8; 17] = b"JUSTVNC-RECORDING";
const VERSION: u16 = 1;
/// ServerInit without its name.
const SERVER_INIT_HEADER: usize = 24;
/// The longest message a session with default limits can frame: a Raw rectangle filling an
/// 8192x8192 framebuffer at 32 bits per pixel, with its headers.
const MAX_ENTRY: u32 = 8192 * 8192 * 4 + 16;

/// Which way a message crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the Target (the server) to the Viewer.
    ToViewer,
    /// From the Viewer to the Target.
    ToTarget,
}

/// One message of a Recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub direction: Direction,
    /// Since the start of the session, as the host measured it.
    pub time: Duration,
    /// The bytes of one framed message, as they crossed.
    pub bytes: Vec<u8>,
}

/// Writes a Recording: the desktop first, then one entry per message.
#[derive(Debug)]
pub struct RecordingWriter<W> {
    out: W,
}

impl<W: Write> RecordingWriter<W> {
    /// Starts a Recording of a session whose ServerInit is `desktop`.
    pub fn new(mut out: W, desktop: &ServerInit) -> io::Result<Self> {
        let init = desktop.to_bytes();
        out.write_all(MAGIC)?;
        out.write_all(&VERSION.to_be_bytes())?;
        out.write_all(&length(init.len())?.to_be_bytes())?;
        out.write_all(&init)?;
        Ok(Self { out })
    }

    /// Adds the message `bytes`, which crossed in `direction` at `time`.
    pub fn write(&mut self, direction: Direction, time: Duration, bytes: &[u8]) -> io::Result<()> {
        let micros = u64::try_from(time.as_micros()).unwrap_or(u64::MAX);
        self.out.write_all(&[direction as u8])?;
        self.out.write_all(&micros.to_be_bytes())?;
        self.out.write_all(&length(bytes.len())?.to_be_bytes())?;
        self.out.write_all(bytes)
    }

    /// Flushes what was written to the underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    /// The underlying writer.
    pub fn into_inner(self) -> W {
        self.out
    }
}

fn length(len: usize) -> io::Result<u32> {
    u32::try_from(len)
        .ok()
        .filter(|&n| n <= MAX_ENTRY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "message too long to record"))
}

/// Reads a Recording, which may come from anywhere: every length is checked before the bytes
/// it announces are read.
#[derive(Debug)]
pub struct RecordingReader<R> {
    input: R,
    desktop: ServerInit,
}

/// Why a Recording could not be read.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    /// The input does not start as a Recording does.
    NotARecording,
    /// A Recording in a format version this reader does not know.
    UnsupportedVersion(u16),
    /// The input ends inside the header or an entry.
    Truncated,
    /// The Recording breaks its format, as described.
    Malformed(&'static str),
}

impl<R: Read> RecordingReader<R> {
    /// Reads the header: the format, then the desktop.
    pub fn new(mut input: R) -> Result<Self, ReadError> {
        let mut magic = [0; MAGIC.len()];
        read_exact(&mut input, &mut magic)?;
        if &magic != MAGIC {
            return Err(ReadError::NotARecording);
        }
        let version = u16::from_be_bytes(read_array(&mut input)?);
        if version != VERSION {
            return Err(ReadError::UnsupportedVersion(version));
        }
        let len = u32::from_be_bytes(read_array(&mut input)?);
        let longest = SERVER_INIT_HEADER as u32 + Limits::default().max_name_length;
        if !(SERVER_INIT_HEADER as u32..=longest).contains(&len) {
            return Err(ReadError::Malformed(
                "ServerInit length is not one a server may send",
            ));
        }
        let init = read_bytes(&mut input, len)?;
        let name_len = u32::from_be_bytes(init[20..24].try_into().unwrap()) as usize;
        if SERVER_INIT_HEADER + name_len != init.len() {
            return Err(ReadError::Malformed(
                "ServerInit name length does not match",
            ));
        }
        let desktop = ServerInit {
            width: u16::from_be_bytes([init[0], init[1]]),
            height: u16::from_be_bytes([init[2], init[3]]),
            pixel_format: PixelFormat::from_bytes(init[4..20].try_into().unwrap()),
            name: DesktopName::new(init[SERVER_INIT_HEADER..].to_vec()),
        };
        Ok(Self { input, desktop })
    }

    /// The ServerInit the recorded session started with.
    pub fn desktop(&self) -> &ServerInit {
        &self.desktop
    }

    fn entry(&mut self, direction: u8) -> Result<Entry, ReadError> {
        let direction = match direction {
            0 => Direction::ToViewer,
            1 => Direction::ToTarget,
            _ => return Err(ReadError::Malformed("unknown direction")),
        };
        let time = Duration::from_micros(u64::from_be_bytes(read_array(&mut self.input)?));
        let len = u32::from_be_bytes(read_array(&mut self.input)?);
        if len > MAX_ENTRY {
            return Err(ReadError::Malformed("entry is longer than any message"));
        }
        let bytes = read_bytes(&mut self.input, len)?;
        Ok(Entry {
            direction,
            time,
            bytes,
        })
    }
}

/// The entries in order; `None` at the end of the Recording.
impl<R: Read> Iterator for RecordingReader<R> {
    type Item = Result<Entry, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut direction = [0];
        match self.input.read(&mut direction) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(e) => return Some(Err(ReadError::Io(e))),
        }
        Some(self.entry(direction[0]))
    }
}

fn read_exact(input: &mut impl Read, buf: &mut [u8]) -> Result<(), ReadError> {
    input.read_exact(buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => ReadError::Truncated,
        _ => ReadError::Io(e),
    })
}

fn read_array<const N: usize>(input: &mut impl Read) -> Result<[u8; N], ReadError> {
    let mut buf = [0; N];
    read_exact(input, &mut buf)?;
    Ok(buf)
}

/// `len` bytes, grown as they arrive rather than allocated up front.
fn read_bytes(input: &mut impl Read, len: u32) -> Result<Vec<u8>, ReadError> {
    let mut bytes = Vec::new();
    input
        .take(u64::from(len))
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() != len as usize {
        return Err(ReadError::Truncated);
    }
    Ok(bytes)
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::NotARecording => f.write_str("not a justvnc Recording"),
            Self::UnsupportedVersion(v) => {
                write!(f, "Recording format version {v} is not supported")
            }
            Self::Truncated => f.write_str("the Recording ends inside an entry"),
            Self::Malformed(what) => write!(f, "malformed Recording: {what}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => e.source(),
            _ => None,
        }
    }
}
