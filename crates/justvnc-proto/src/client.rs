//! The client role's side of the handshake.

use crate::vnc_auth::{self, CHALLENGE_LEN};
use crate::{
    DesktopName, HandshakeError, Limits, Password, PixelFormat, ProtocolVersion, ServerInit,
};

const SECURITY_INVALID: u8 = 0;
const SECURITY_NONE: u8 = 1;
const SECURITY_VNC_AUTH: u8 = 2;

/// Drives the client side of the RFB handshake from received bytes.
#[derive(Debug)]
pub struct ClientHandshake {
    shared: bool,
    password: Option<Password>,
    limits: Limits,
    version: ProtocolVersion,
    state: State,
    received: Vec<u8>,
    transmit: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Version,
    SecurityTypes,
    VncAuthChallenge,
    SecurityResult,
    ServerInit,
    Done,
}

impl ClientHandshake {
    pub fn new(shared: bool, password: Option<Password>, limits: Limits) -> Self {
        Self {
            shared,
            password,
            limits,
            version: ProtocolVersion::V3_8,
            state: State::Version,
            received: Vec::new(),
            transmit: Vec::new(),
        }
    }

    /// Feeds bytes from the server. Returns the ServerInit once the handshake completes.
    pub fn receive(&mut self, data: &[u8]) -> Result<Option<ServerInit>, HandshakeError> {
        self.received.extend_from_slice(data);
        loop {
            let step = match self.state {
                State::Version => self.read_version()?,
                State::SecurityTypes => self.read_security_types()?,
                State::VncAuthChallenge => self.read_vnc_auth_challenge(),
                State::SecurityResult => self.read_security_result()?,
                State::ServerInit => match self.read_server_init()? {
                    Some(init) => return Ok(Some(init)),
                    None => None,
                },
                State::Done => return Ok(None),
            };
            match step {
                Some(consumed) => {
                    self.received.drain(..consumed);
                }
                None => return Ok(None),
            }
        }
    }

    /// Bytes to send to the server, emptied by the call.
    pub fn take_transmit(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.transmit)
    }

    /// Bytes received after ServerInit, which belong to the messages that follow it.
    pub fn take_remaining(&mut self) -> Vec<u8> {
        if self.state != State::Done {
            return Vec::new();
        }
        std::mem::take(&mut self.received)
    }

    fn read_version(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(bytes) = self.received.first_chunk::<{ ProtocolVersion::LEN }>() else {
            return Ok(None);
        };
        let server = ProtocolVersion::parse(bytes).ok_or(HandshakeError::MalformedVersion)?;
        self.version = match server {
            ProtocolVersion::V3_7 | ProtocolVersion::V3_8 => server,
            _ => ProtocolVersion::V3_3,
        };
        self.transmit.extend_from_slice(&self.version.to_bytes());
        self.state = State::SecurityTypes;
        Ok(Some(ProtocolVersion::LEN))
    }

    fn read_security_types(&mut self) -> Result<Option<usize>, HandshakeError> {
        if self.version == ProtocolVersion::V3_3 {
            return self.read_server_chosen_security_type();
        }
        let Some(&count) = self.received.first() else {
            return Ok(None);
        };
        if count == 0 {
            return match self.read_reason(1)? {
                Some(reason) => Err(HandshakeError::ConnectionRefused { reason }),
                None => Ok(None),
            };
        }
        let Some(offered) = self.received.get(1..1 + count as usize) else {
            return Ok(None);
        };
        let usable =
            |t: &&u8| **t == SECURITY_NONE || (**t == SECURITY_VNC_AUTH && self.password.is_some());
        let Some(&chosen) = offered.iter().find(usable) else {
            return Err(if offered.contains(&SECURITY_VNC_AUTH) {
                HandshakeError::PasswordRequired
            } else {
                HandshakeError::NoSupportedSecurityType {
                    offered: offered.to_vec(),
                }
            });
        };
        self.transmit.push(chosen);
        self.after_security_type(chosen);
        Ok(Some(1 + count as usize))
    }

    /// Version 3.3: the server picks the security type and sends it as a U32.
    fn read_server_chosen_security_type(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(word) = self.received.first_chunk::<4>() else {
            return Ok(None);
        };
        let chosen = u32::from_be_bytes(*word);
        match u8::try_from(chosen) {
            Ok(SECURITY_INVALID) => match self.read_reason(4)? {
                Some(reason) => Err(HandshakeError::ConnectionRefused { reason }),
                None => Ok(None),
            },
            Ok(SECURITY_NONE) => {
                self.after_security_type(SECURITY_NONE);
                Ok(Some(4))
            }
            Ok(SECURITY_VNC_AUTH) if self.password.is_some() => {
                self.after_security_type(SECURITY_VNC_AUTH);
                Ok(Some(4))
            }
            Ok(SECURITY_VNC_AUTH) => Err(HandshakeError::PasswordRequired),
            _ => Err(HandshakeError::UnexpectedSecurityType { value: chosen }),
        }
    }

    fn after_security_type(&mut self, chosen: u8) {
        self.state = match chosen {
            SECURITY_VNC_AUTH => State::VncAuthChallenge,
            // Before 3.8 there is no SecurityResult after security type None.
            _ if self.version == ProtocolVersion::V3_8 => State::SecurityResult,
            _ => {
                self.send_client_init();
                State::ServerInit
            }
        };
    }

    fn read_vnc_auth_challenge(&mut self) -> Option<usize> {
        let challenge = self.received.first_chunk::<CHALLENGE_LEN>()?;
        let password = self.password.as_ref().map_or(&[][..], Password::as_bytes);
        self.transmit
            .extend_from_slice(&vnc_auth::response(password, challenge));
        self.state = State::SecurityResult;
        Some(CHALLENGE_LEN)
    }

    fn read_security_result(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(status) = self.received.first_chunk::<4>() else {
            return Ok(None);
        };
        if u32::from_be_bytes(*status) != 0 {
            if self.version != ProtocolVersion::V3_8 {
                return Err(HandshakeError::SecurityFailed { reason: None });
            }
            return match self.read_reason(4)? {
                Some(reason) => Err(HandshakeError::SecurityFailed {
                    reason: Some(reason),
                }),
                None => Ok(None),
            };
        }
        self.send_client_init();
        self.state = State::ServerInit;
        Ok(Some(4))
    }

    fn send_client_init(&mut self) {
        self.transmit.push(u8::from(self.shared));
    }

    fn read_server_init(&mut self) -> Result<Option<ServerInit>, HandshakeError> {
        const HEADER: usize = 2 + 2 + PixelFormat::LEN + 4;
        let Some(header) = self.received.first_chunk::<HEADER>() else {
            return Ok(None);
        };
        let name_length = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
        check_limit("name-length", name_length, self.limits.max_name_length)?;
        let Some(name) = self.received.get(HEADER..HEADER + name_length as usize) else {
            return Ok(None);
        };
        let init = ServerInit {
            width: u16::from_be_bytes([header[0], header[1]]),
            height: u16::from_be_bytes([header[2], header[3]]),
            pixel_format: PixelFormat::from_bytes(header[4..20].try_into().unwrap()),
            name: DesktopName::new(name.to_vec()),
        };
        self.received.drain(..HEADER + name_length as usize);
        self.state = State::Done;
        Ok(Some(init))
    }

    /// Reads a U32 length and that many bytes starting at `offset`, as a reason string.
    fn read_reason(&self, offset: usize) -> Result<Option<String>, HandshakeError> {
        let Some(length) = self
            .received
            .get(offset..offset + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
        else {
            return Ok(None);
        };
        check_limit("reason-length", length, self.limits.max_reason_length)?;
        let start = offset + 4;
        Ok(self
            .received
            .get(start..start + length as usize)
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
}

fn check_limit(field: &'static str, length: u32, max: u32) -> Result<(), HandshakeError> {
    if length > max {
        return Err(HandshakeError::LimitExceeded { field, length, max });
    }
    Ok(())
}
