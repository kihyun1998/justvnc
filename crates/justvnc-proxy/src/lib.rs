//! RFB Proxy between a Viewer and a Target.

mod session;
mod tap;

pub use justvnc_proto::CutText;
pub use justvnc_proto::client_message::ClientMessage;
pub use justvnc_proto::server::SecurityType;
pub use justvnc_proto::server_message::Framed;
pub use justvnc_proto::vnc_auth::{CHALLENGE_LEN, verify};
pub use justvnc_proto::{
    HandshakeError, Limits, MessageError, Password, PixelFormat, ProtocolVersion, ServerInit,
};
pub use session::{CloseReason, Error, Event, ProxyConfig, ProxySession, narrow_encodings};
pub use tap::{ForwardAll, Tap, TargetMessage, UpdateVerdict, Verdict};

#[cfg(feature = "tokio")]
pub mod tokio;
