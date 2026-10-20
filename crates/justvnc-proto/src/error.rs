use crate::ProtocolVersion;

/// Why a handshake could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// The peer's ProtocolVersion message is not "RFB xxx.yyy\n".
    MalformedVersion,
    /// The server offers a version this session does not speak.
    UnsupportedVersion { server: ProtocolVersion },
    /// The server refused the connection before authentication, with its reason.
    ConnectionRefused { reason: String },
    /// None of the security types the server offered is one this session accepts.
    NoSupportedSecurityType { offered: Vec<u8> },
    /// The server reported that authentication failed, with its reason.
    SecurityFailed { reason: String },
    /// The peer announced a length above the configured limit.
    LimitExceeded {
        field: &'static str,
        length: u32,
        max: u32,
    },
}
