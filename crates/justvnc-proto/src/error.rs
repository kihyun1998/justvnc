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
    /// The server requires VNC authentication and no password was configured.
    PasswordRequired,
    /// A 3.3 server chose a security type other than 0, 1 or 2.
    UnexpectedSecurityType { value: u32 },
    /// None of the security types the server offered is one this session accepts.
    NoSupportedSecurityType { offered: Vec<u8> },
    /// The server reported that authentication failed. Only 3.8 servers send a reason.
    SecurityFailed { reason: Option<String> },
    /// The peer announced a length above the configured limit.
    LimitExceeded {
        field: &'static str,
        length: u32,
        max: u32,
    },
}
