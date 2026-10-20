use std::collections::VecDeque;

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::{HandshakeError, Limits, Password, ServerInit};

/// How a client session behaves.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Ask the server to leave other clients connected (ClientInit shared-flag).
    pub shared: bool,
    /// Password for VNC authentication. Without one, VNC authentication is never chosen.
    pub password: Option<Password>,
    pub limits: Limits,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            shared: true,
            password: None,
            limits: Limits::default(),
        }
    }
}

/// What the session reports to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The handshake completed and the server described its desktop.
    Connected(ServerInit),
}

/// The client role as a state machine: bytes in, events and bytes to send out.
#[derive(Debug)]
pub struct ClientSession {
    handshake: ClientHandshake,
    events: VecDeque<Event>,
}

impl ClientSession {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            handshake: ClientHandshake::new(config.shared, config.password, config.limits),
            events: VecDeque::new(),
        }
    }

    /// Feeds bytes received from the server.
    pub fn receive(&mut self, data: &[u8]) -> Result<(), HandshakeError> {
        if let Some(init) = self.handshake.receive(data)? {
            self.events.push_back(Event::Connected(init));
        }
        Ok(())
    }

    /// Next bytes to send to the server, if any.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        let bytes = self.handshake.take_transmit();
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Next event for the host, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }
}
