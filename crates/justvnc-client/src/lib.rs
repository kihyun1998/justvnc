//! RFB client role.

mod session;

pub use justvnc_proto::{
    DesktopName, HandshakeError, Limits, Password, PixelFormat, ProtocolVersion, ServerInit,
};
pub use session::{ClientConfig, ClientSession, Event};

#[cfg(feature = "tokio")]
pub mod tokio;
