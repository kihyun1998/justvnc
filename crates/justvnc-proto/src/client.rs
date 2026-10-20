//! The client role's side of the handshake.

use crate::{DesktopName, HandshakeError, Limits, PixelFormat, ProtocolVersion, ServerInit};

const SECURITY_NONE: u8 = 1;

/// Drives the client side of the RFB handshake from received bytes.
#[derive(Debug)]
pub struct ClientHandshake {
    shared: bool,
    limits: Limits,
    state: State,
    received: Vec<u8>,
    transmit: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Version,
    SecurityTypes,
    SecurityResult,
    ServerInit,
    Done,
}

impl ClientHandshake {
    pub fn new(shared: bool, limits: Limits) -> Self {
        Self {
            shared,
            limits,
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
        if server != ProtocolVersion::V3_8 {
            return Err(HandshakeError::UnsupportedVersion { server });
        }
        self.transmit
            .extend_from_slice(&ProtocolVersion::V3_8.to_bytes());
        self.state = State::SecurityTypes;
        Ok(Some(ProtocolVersion::LEN))
    }

    fn read_security_types(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(&count) = self.received.first() else {
            return Ok(None);
        };
        if count == 0 {
            let reason = self.read_reason(1)?;
            return match reason {
                Some(reason) => Err(HandshakeError::ConnectionRefused { reason }),
                None => Ok(None),
            };
        }
        let Some(offered) = self.received.get(1..1 + count as usize) else {
            return Ok(None);
        };
        if !offered.contains(&SECURITY_NONE) {
            return Err(HandshakeError::NoSupportedSecurityType {
                offered: offered.to_vec(),
            });
        }
        self.transmit.push(SECURITY_NONE);
        self.state = State::SecurityResult;
        Ok(Some(1 + count as usize))
    }

    fn read_security_result(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(status) = self.received.first_chunk::<4>() else {
            return Ok(None);
        };
        if u32::from_be_bytes(*status) != 0 {
            return match self.read_reason(4)? {
                Some(reason) => Err(HandshakeError::SecurityFailed { reason }),
                None => Ok(None),
            };
        }
        self.transmit.push(u8::from(self.shared));
        self.state = State::ServerInit;
        Ok(Some(4))
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
