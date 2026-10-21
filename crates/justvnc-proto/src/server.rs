//! The server role's side of the handshake.

use crate::vnc_auth::CHALLENGE_LEN;
use crate::{HandshakeError, ProtocolVersion, ServerInit};

/// A security type the server role can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityType {
    None,
    VncAuth,
}

impl SecurityType {
    fn value(self) -> u8 {
        match self {
            Self::None => 1,
            Self::VncAuth => 2,
        }
    }
}

/// What the handshake needs from its host before it can go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The client answered the VNC authentication challenge. Decide with `accept` or `reject`.
    Authenticate {
        challenge: [u8; CHALLENGE_LEN],
        response: [u8; CHALLENGE_LEN],
    },
    /// The client chose security type None, and the handshake was made `confirming_none`.
    /// Decide with `accept` or `reject`.
    NoAuthentication,
    /// The client sent ClientInit. Answer with `server_init`.
    ClientInit { shared: bool },
}

/// Drives the server side of the RFB handshake from received bytes. Its ProtocolVersion is
/// ready to send as soon as it is made.
#[derive(Debug)]
pub struct ServerHandshake {
    offered: ProtocolVersion,
    version: ProtocolVersion,
    security: Vec<SecurityType>,
    challenge: [u8; CHALLENGE_LEN],
    confirm_none: bool,
    chosen: Option<SecurityType>,
    state: State,
    received: Vec<u8>,
    transmit: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Version,
    SecurityType,
    VncAuthResponse,
    Deciding,
    ClientInit,
    ServerInit,
    Done,
    Failed(HandshakeError),
}

impl ServerHandshake {
    /// A handshake offering `version` (3.3, 3.7 or 3.8) and `security`, in order of preference.
    /// `challenge` is sent if VNC authentication is chosen; it must be random.
    ///
    /// # Panics
    ///
    /// If `security` is empty or holds more than 255 types.
    pub fn new(
        version: ProtocolVersion,
        security: Vec<SecurityType>,
        challenge: [u8; CHALLENGE_LEN],
    ) -> Self {
        assert!(
            (1..=255).contains(&security.len()),
            "one to 255 security types"
        );
        Self {
            offered: version,
            version,
            security,
            challenge,
            confirm_none: false,
            chosen: None,
            state: State::Version,
            received: Vec::new(),
            transmit: version.to_bytes().to_vec(),
        }
    }

    /// Makes security type None wait for the host too: it is reported as
    /// `Step::NoAuthentication` and answered with `accept` or `reject`, as VNC authentication is.
    /// A Proxy uses this to hold the Viewer until the Target has answered.
    pub fn confirming_none(mut self) -> Self {
        self.confirm_none = true;
        self
    }

    /// Feeds bytes from the client. Returns a step when the host has to act; after acting, call
    /// it again (with no bytes if none came) to go on with what is already buffered.
    pub fn receive(&mut self, data: &[u8]) -> Result<Option<Step>, HandshakeError> {
        self.received.extend_from_slice(data);
        loop {
            let (step, consumed) = match &self.state {
                State::Version => match self.read_version()? {
                    Some(used) if self.state == State::Deciding => {
                        (Some(Step::NoAuthentication), Some(used))
                    }
                    used => (None, used),
                },
                State::SecurityType => match self.read_security_type()? {
                    Some(used) if self.state == State::Deciding => {
                        (Some(Step::NoAuthentication), Some(used))
                    }
                    used => (None, used),
                },
                State::VncAuthResponse => match self.read_vnc_auth_response() {
                    Some(step) => (Some(step), Some(CHALLENGE_LEN)),
                    None => (None, None),
                },
                State::ClientInit => match self.received.first() {
                    Some(&flag) => {
                        self.state = State::ServerInit;
                        (Some(Step::ClientInit { shared: flag != 0 }), Some(1))
                    }
                    None => (None, None),
                },
                State::Deciding | State::ServerInit | State::Done => return Ok(None),
                State::Failed(error) => return Err(error.clone()),
            };
            if let Some(consumed) = consumed {
                self.received.drain(..consumed);
            }
            match (step, consumed) {
                (Some(step), _) => return Ok(Some(step)),
                (None, Some(_)) => {}
                (None, None) => return Ok(None),
            }
        }
    }

    /// Accepts the response of the last `Step::Authenticate`, or the `Step::NoAuthentication`.
    pub fn accept(&mut self) {
        if self.state == State::Deciding {
            if self.has_security_result() {
                self.transmit.extend_from_slice(&0u32.to_be_bytes());
            }
            self.state = State::ClientInit;
        }
    }

    /// Rejects the response of the last `Step::Authenticate`, or the `Step::NoAuthentication`.
    /// Under 3.8 the client is told `reason`; after None before 3.8 it is told nothing, and the
    /// host closes the connection. Every later `receive` returns the failure.
    pub fn reject(&mut self, reason: &str) {
        if self.state != State::Deciding {
            return;
        }
        if self.has_security_result() {
            self.transmit.extend_from_slice(&1u32.to_be_bytes());
        }
        let reason = (self.version == ProtocolVersion::V3_8).then(|| {
            self.transmit.extend_from_slice(&string(reason));
            reason.to_owned()
        });
        self.state = State::Failed(HandshakeError::SecurityFailed { reason });
    }

    /// Sends ServerInit after `Step::ClientInit`, which completes the handshake.
    pub fn server_init(&mut self, init: &ServerInit) {
        if self.state == State::ServerInit {
            self.transmit.extend_from_slice(&init.to_bytes());
            self.state = State::Done;
        }
    }

    /// Whether ServerInit has been sent.
    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// The version both sides speak, once the client has answered.
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// Bytes to send to the client, emptied by the call.
    pub fn take_transmit(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.transmit)
    }

    /// Bytes received after ClientInit, which belong to the messages that follow ServerInit.
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
        let client = ProtocolVersion::parse(bytes).ok_or(HandshakeError::MalformedVersion)?;
        if client.major != 3 {
            return Err(HandshakeError::UnsupportedClientVersion { client });
        }
        let spoken = match client.minor {
            8.. => ProtocolVersion::V3_8,
            7 => ProtocolVersion::V3_7,
            _ => ProtocolVersion::V3_3,
        };
        self.version = spoken.min(self.offered);
        if self.version == ProtocolVersion::V3_3 {
            let chosen = self.security[0];
            self.transmit
                .extend_from_slice(&u32::from(chosen.value()).to_be_bytes());
            self.after_security_type(chosen);
        } else {
            self.transmit.push(self.security.len() as u8);
            self.transmit
                .extend(self.security.iter().map(|t| t.value()));
            self.state = State::SecurityType;
        }
        Ok(Some(ProtocolVersion::LEN))
    }

    fn read_security_type(&mut self) -> Result<Option<usize>, HandshakeError> {
        let Some(&value) = self.received.first() else {
            return Ok(None);
        };
        let Some(&chosen) = self.security.iter().find(|t| t.value() == value) else {
            return Err(HandshakeError::UnexpectedSecurityType {
                value: u32::from(value),
            });
        };
        self.after_security_type(chosen);
        Ok(Some(1))
    }

    /// Whether a SecurityResult follows the chosen security type: always under 3.8, and after
    /// VNC authentication before it.
    fn has_security_result(&self) -> bool {
        self.version == ProtocolVersion::V3_8 || self.chosen == Some(SecurityType::VncAuth)
    }

    fn after_security_type(&mut self, chosen: SecurityType) {
        self.chosen = Some(chosen);
        self.state = match chosen {
            SecurityType::VncAuth => {
                self.transmit.extend_from_slice(&self.challenge);
                State::VncAuthResponse
            }
            SecurityType::None if self.confirm_none => State::Deciding,
            SecurityType::None => {
                // Before 3.8 there is no SecurityResult after security type None.
                if self.version == ProtocolVersion::V3_8 {
                    self.transmit.extend_from_slice(&0u32.to_be_bytes());
                }
                State::ClientInit
            }
        };
    }

    fn read_vnc_auth_response(&mut self) -> Option<Step> {
        let response = *self.received.first_chunk::<CHALLENGE_LEN>()?;
        self.state = State::Deciding;
        Some(Step::Authenticate {
            challenge: self.challenge,
            response,
        })
    }
}

/// A U32 length and the bytes of `text`.
fn string(text: &str) -> Vec<u8> {
    let length = u32::try_from(text.len()).expect("reason fits a U32 length");
    [&length.to_be_bytes()[..], text.as_bytes()].concat()
}
