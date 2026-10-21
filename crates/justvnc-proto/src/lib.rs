//! RFB protocol core: handshake state machines, messages and framing, without I/O.

pub mod client;
pub mod client_message;
mod cut_text;
pub mod encoding;
mod error;
mod limits;
mod password;
mod pixel_format;
mod rect;
mod server_init;
pub mod server_message;
pub mod tight;
mod version;
pub mod vnc_auth;

pub use cut_text::{CutText, NotLatin1};
pub use error::{HandshakeError, MessageError};
pub use limits::Limits;
pub use password::Password;
pub use pixel_format::PixelFormat;
pub use rect::Rect;
pub use server_init::{DesktopName, ServerInit};
pub use version::ProtocolVersion;
