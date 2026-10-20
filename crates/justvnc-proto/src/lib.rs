//! RFB protocol core: handshake state machines, messages and framing, without I/O.

pub mod client;
mod error;
mod limits;
mod pixel_format;
mod server_init;
mod version;

pub use error::HandshakeError;
pub use limits::Limits;
pub use pixel_format::PixelFormat;
pub use server_init::{DesktopName, ServerInit};
pub use version::ProtocolVersion;
