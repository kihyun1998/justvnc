//! RFB server role.

mod framebuffer;
mod region;
mod session;

pub use framebuffer::Framebuffer;
pub use justvnc_proto::server::SecurityType;
pub use justvnc_proto::vnc_auth::{CHALLENGE_LEN, verify};
pub use justvnc_proto::{
    CutText, DesktopName, HandshakeError, Limits, MessageError, Password, PixelFormat,
    ProtocolVersion, Rect,
};
pub use session::{Error, Event, ServerConfig, ServerSession};

#[cfg(feature = "tokio")]
pub mod tokio;
