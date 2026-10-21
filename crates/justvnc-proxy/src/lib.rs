//! RFB Proxy between a Viewer and a Target.

mod session;

pub use justvnc_proto::server::SecurityType;
pub use justvnc_proto::vnc_auth::{CHALLENGE_LEN, verify};
pub use justvnc_proto::{
    HandshakeError, Limits, MessageError, Password, PixelFormat, ProtocolVersion, ServerInit,
};
pub use session::{Error, Event, ProxyConfig, ProxySession, narrow_encodings};

#[cfg(feature = "tokio")]
pub mod tokio;
