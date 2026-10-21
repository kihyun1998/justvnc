use crate::{ProtocolVersion, Rect};

/// Why a handshake could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// The peer's ProtocolVersion message is not "RFB xxx.yyy\n".
    MalformedVersion,
    /// The server offers a version this session does not speak.
    UnsupportedVersion { server: ProtocolVersion },
    /// The client asked for a version other than RFB 3.x (server role).
    UnsupportedClientVersion { client: ProtocolVersion },
    /// The server refused the connection before authentication, with its reason.
    ConnectionRefused { reason: String },
    /// The server requires VNC authentication and no password was configured.
    PasswordRequired,
    /// A 3.3 server chose a security type other than 0, 1 or 2, or a client chose one the
    /// server did not offer.
    UnexpectedSecurityType { value: u32 },
    /// None of the security types the server offered is one this session accepts.
    NoSupportedSecurityType { offered: Vec<u8> },
    /// Authentication failed: the server reported it, or (server role) the host rejected the
    /// response. Only 3.8 carries a reason.
    SecurityFailed { reason: Option<String> },
    /// The peer announced a length above the configured limit.
    LimitExceeded {
        field: &'static str,
        length: u32,
        max: u32,
    },
}

/// Why a message after ServerInit could not be framed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// A server message type this session does not handle yet.
    UnsupportedMessage { message_type: u8 },
    /// A rectangle in an encoding that cannot be framed.
    UnsupportedEncoding { encoding: i32 },
    /// A rectangle reaching outside the framebuffer.
    RectangleOutOfBounds { rect: Rect },
    /// A message or rectangle that breaks its encoding's rules, as described.
    Malformed(&'static str),
    /// The peer announced a value above the configured limit.
    LimitExceeded {
        field: &'static str,
        length: u32,
        max: u32,
    },
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedVersion => {
                f.write_str("the peer's ProtocolVersion message is malformed")
            }
            Self::UnsupportedVersion { server } => write!(
                f,
                "the server speaks RFB {}.{}, which is not supported",
                server.major, server.minor
            ),
            Self::UnsupportedClientVersion { client } => write!(
                f,
                "the client asked for RFB {}.{}, which is not supported",
                client.major, client.minor
            ),
            Self::ConnectionRefused { reason } => {
                write!(f, "the server refused the connection: {reason:?}")
            }
            Self::PasswordRequired => {
                f.write_str("the server requires VNC authentication and no password is set")
            }
            Self::UnexpectedSecurityType { value } => {
                write!(f, "unexpected security type {value}")
            }
            Self::NoSupportedSecurityType { offered } => write!(
                f,
                "none of the security types the server offered ({offered:?}) is supported"
            ),
            Self::SecurityFailed { reason: None } => f.write_str("authentication failed"),
            Self::SecurityFailed {
                reason: Some(reason),
            } => write!(f, "authentication failed: {reason:?}"),
            Self::LimitExceeded { field, length, max } => {
                write!(f, "{field} {length} is above the limit of {max}")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMessage { message_type } => {
                write!(f, "message type {message_type} is not supported")
            }
            Self::UnsupportedEncoding { encoding } => {
                write!(f, "encoding {encoding} cannot be framed")
            }
            Self::RectangleOutOfBounds { rect } => write!(
                f,
                "rectangle {}x{} at ({}, {}) reaches outside the framebuffer",
                rect.width, rect.height, rect.x, rect.y
            ),
            Self::Malformed(what) => write!(f, "malformed message: {what}"),
            Self::LimitExceeded { field, length, max } => {
                write!(f, "{field} {length} is above the limit of {max}")
            }
        }
    }
}

impl std::error::Error for MessageError {}
