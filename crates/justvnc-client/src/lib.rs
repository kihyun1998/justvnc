//! RFB client role.

mod framebuffer;
mod session;

pub use framebuffer::Framebuffer;
pub use justvnc_decode::DecodeError;
pub use justvnc_proto::{
    CutText, DesktopName, HandshakeError, Limits, MessageError, NotLatin1, Password, PixelFormat,
    ProtocolVersion, Rect, ServerInit,
};
pub use session::{ClientConfig, ClientSession, Error, Event, Rectangle};

#[cfg(feature = "tokio")]
pub mod tokio;
