use std::collections::VecDeque;

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::client_message::{self, ClientMessage, ClientMessageReader};
use justvnc_proto::server::{SecurityType, ServerHandshake, Step};
use justvnc_proto::server_message::ServerMessageReader;
use justvnc_proto::vnc_auth::CHALLENGE_LEN;
use justvnc_proto::{
    HandshakeError, Limits, MessageError, Password, PixelFormat, ProtocolVersion, ServerInit,
    encoding,
};

/// How a Proxy session behaves.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// The highest version offered to the Viewer: 3.3, 3.7 or 3.8.
    pub version: ProtocolVersion,
    /// The security types offered to the Viewer, most preferred first.
    pub security: Vec<SecurityType>,
    /// The ClientInit shared-flag sent to the Target: whether other clients stay connected.
    pub shared: bool,
    pub limits: Limits,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            version: ProtocolVersion::V3_8,
            security: vec![SecurityType::None],
            shared: true,
            limits: Limits::default(),
        }
    }
}

/// What the session reports to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The Viewer answered the VNC authentication challenge. Decide with
    /// [`ProxySession::accept`] or [`ProxySession::reject`].
    ///
    /// VNC authentication uses only the first 8 bytes of a password. A host that takes one-time
    /// tokens as passwords should make each token's first 8 bytes as random as it can.
    Authenticate {
        challenge: [u8; CHALLENGE_LEN],
        response: [u8; CHALLENGE_LEN],
    },
    /// The Viewer is through security. Connect to the Target and call
    /// [`ProxySession::connect_target`], or [`ProxySession::refuse`] it.
    TargetNeeded,
    /// Both handshakes completed; messages are relayed from now on. `desktop` is the Target's
    /// ServerInit, which the Viewer was sent as it is.
    Connected { desktop: ServerInit },
}

/// Why the session stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    ViewerHandshake(HandshakeError),
    TargetHandshake(HandshakeError),
    /// A message from the Viewer that cannot be framed.
    Viewer(MessageError),
    /// A message from the Target that cannot be framed.
    Target(MessageError),
    /// The Viewer asked for a pixel format the Target's updates cannot be framed in: not
    /// valid, or colour-mapped.
    UnsupportedPixelFormat(PixelFormat),
}

/// The reason a Viewer is given when the Target leg fails during the handshake. The Target's own
/// reason is not passed on.
const TARGET_FAILED: &str = "Connection to the Target failed";

/// A Proxy between one Viewer and one Target, as a state machine: bytes in from either leg,
/// bytes out to either leg.
#[derive(Debug)]
pub struct ProxySession {
    phase: Phase,
    shared: bool,
    limits: Limits,
    events: VecDeque<Event>,
    to_viewer: Vec<u8>,
    to_target: Vec<u8>,
    failed: Option<Error>,
}

#[derive(Debug)]
enum Phase {
    Handshake {
        viewer: ServerHandshake,
        target: Option<ClientHandshake>,
        /// The Target's ServerInit, once its handshake is done.
        desktop: Option<ServerInit>,
        /// Target bytes received before there was a target handshake to give them to.
        early_target: Vec<u8>,
        /// The Viewer's security is settled and its SecurityResult waits for the Target.
        settled: bool,
    },
    Relay {
        viewer_reader: ClientMessageReader,
        target_reader: ServerMessageReader,
        from_viewer: Vec<u8>,
        from_target: Vec<u8>,
    },
}

impl ProxySession {
    /// A session with a random VNC authentication challenge for the Viewer.
    ///
    /// # Panics
    ///
    /// If the operating system's random number source fails.
    pub fn new(config: ProxyConfig) -> Self {
        let mut challenge = [0; CHALLENGE_LEN];
        getrandom::fill(&mut challenge).expect("the system random number source works");
        Self::with_challenge(config, challenge)
    }

    /// A session that sends the Viewer `challenge` if VNC authentication is chosen. It must be
    /// random and never reused.
    ///
    /// # Panics
    ///
    /// If `config.security` is empty or holds more than 255 types.
    pub fn with_challenge(config: ProxyConfig, challenge: [u8; CHALLENGE_LEN]) -> Self {
        let viewer =
            ServerHandshake::new(config.version, config.security, challenge).confirming_none();
        Self {
            phase: Phase::Handshake {
                viewer,
                target: None,
                desktop: None,
                early_target: Vec::new(),
                settled: false,
            },
            shared: config.shared,
            limits: config.limits,
            events: VecDeque::new(),
            to_viewer: Vec::new(),
            to_target: Vec::new(),
            failed: None,
        }
    }

    /// Feeds bytes received from the Viewer. After an error, every later call returns it again.
    pub fn receive_viewer(&mut self, data: &[u8]) -> Result<(), Error> {
        self.guarded(|session| session.viewer_bytes(data))
    }

    /// Feeds bytes received from the Target. After an error, every later call returns it again.
    pub fn receive_target(&mut self, data: &[u8]) -> Result<(), Error> {
        self.guarded(|session| session.target_bytes(data))
    }

    /// Accepts the Viewer's response to the last [`Event::Authenticate`]. The Viewer is not told
    /// until the Target leg is up; [`Event::TargetNeeded`] follows.
    pub fn accept(&mut self) {
        if let Phase::Handshake { settled, .. } = &mut self.phase
            && !*settled
        {
            *settled = true;
            self.events.push_back(Event::TargetNeeded);
        }
    }

    /// Rejects the Viewer's response to the last [`Event::Authenticate`]. A 3.8 Viewer is told
    /// `reason`; then the session has ended.
    pub fn reject(&mut self, reason: &str) {
        self.refuse(reason);
    }

    /// Starts the Target leg: the host has connected to the Target, and `password` is the
    /// Target's own for VNC authentication. The Target speaks first, so nothing is sent until
    /// its bytes arrive through [`ProxySession::receive_target`]. Before [`Event::TargetNeeded`]
    /// it does nothing: the Viewer has not passed security yet.
    pub fn connect_target(&mut self, password: Option<Password>) {
        if let Phase::Handshake {
            target: target @ None,
            settled: true,
            ..
        } = &mut self.phase
        {
            *target = Some(ClientHandshake::new(self.shared, password, self.limits));
        }
        let early = match &mut self.phase {
            Phase::Handshake { early_target, .. } => std::mem::take(early_target),
            Phase::Relay { .. } => Vec::new(),
        };
        if !early.is_empty() {
            let _ = self.receive_target(&early);
        }
    }

    /// Ends the session before the Target leg is up, telling a 3.8 Viewer `reason`: the host
    /// could not reach the Target, or will not let this Viewer through.
    pub fn refuse(&mut self, reason: &str) {
        if let Phase::Handshake { viewer, .. } = &mut self.phase {
            viewer.reject(reason);
            self.to_viewer.extend(viewer.take_transmit());
            if let Err(e) = viewer.receive(&[]) {
                self.failed.get_or_insert(Error::ViewerHandshake(e));
            }
        }
    }

    /// Next bytes to send to the Viewer, if any.
    pub fn poll_viewer(&mut self) -> Option<Vec<u8>> {
        if let Phase::Handshake { viewer, .. } = &mut self.phase {
            self.to_viewer.extend(viewer.take_transmit());
        }
        let bytes = std::mem::take(&mut self.to_viewer);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Next bytes to send to the Target, if any.
    pub fn poll_target(&mut self) -> Option<Vec<u8>> {
        if let Phase::Handshake {
            target: Some(target),
            ..
        } = &mut self.phase
        {
            self.to_target.extend(target.take_transmit());
        }
        let bytes = std::mem::take(&mut self.to_target);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Next event for the host, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn guarded(&mut self, step: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = step(self);
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }

    fn viewer_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        let Phase::Handshake { viewer, .. } = &mut self.phase else {
            return self.relay_viewer(data);
        };
        match viewer.receive(data).map_err(Error::ViewerHandshake)? {
            Some(Step::Authenticate {
                challenge,
                response,
            }) => self.events.push_back(Event::Authenticate {
                challenge,
                response,
            }),
            Some(Step::NoAuthentication) => self.accept(),
            Some(Step::ClientInit { .. }) => self.start_relay()?,
            None => {}
        }
        Ok(())
    }

    fn target_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        let Phase::Handshake {
            viewer,
            target,
            desktop,
            early_target,
            ..
        } = &mut self.phase
        else {
            return self.relay_target(data);
        };
        let Some(handshake) = target else {
            early_target.extend_from_slice(data);
            return Ok(());
        };
        let init = match handshake.receive(data) {
            Ok(Some(init)) => init,
            Ok(None) => return Ok(()),
            Err(e) => {
                viewer.reject(TARGET_FAILED);
                return Err(Error::TargetHandshake(e));
            }
        };
        *desktop = Some(init);
        // The Target is up: now the Viewer is told its security succeeded.
        viewer.accept();
        let step = viewer.receive(&[]).map_err(Error::ViewerHandshake)?;
        if let Some(Step::ClientInit { .. }) = step {
            self.start_relay()?;
        }
        Ok(())
    }

    /// Sends the Viewer the Target's ServerInit and starts relaying, once the Viewer has sent
    /// ClientInit and the Target its ServerInit.
    fn start_relay(&mut self) -> Result<(), Error> {
        let Phase::Handshake {
            viewer,
            target: Some(target),
            desktop: Some(desktop),
            ..
        } = &mut self.phase
        else {
            return Ok(());
        };
        viewer.server_init(desktop);
        self.to_viewer.extend(viewer.take_transmit());
        self.to_target.extend(target.take_transmit());
        let from_viewer = viewer.take_remaining();
        let from_target = target.take_remaining();
        let desktop = desktop.clone();
        self.phase = Phase::Relay {
            viewer_reader: ClientMessageReader::new(self.limits),
            target_reader: ServerMessageReader::new(
                &desktop.pixel_format,
                desktop.width,
                desktop.height,
                self.limits,
            ),
            from_viewer: Vec::new(),
            from_target: Vec::new(),
        };
        self.events.push_back(Event::Connected { desktop });
        self.relay_viewer(&from_viewer)?;
        self.relay_target(&from_target)
    }

    fn relay_viewer(&mut self, data: &[u8]) -> Result<(), Error> {
        let Phase::Relay {
            viewer_reader,
            target_reader,
            from_viewer,
            ..
        } = &mut self.phase
        else {
            return Ok(());
        };
        from_viewer.extend_from_slice(data);
        let mut at = 0;
        while let Some((message, used)) = viewer_reader
            .next(&from_viewer[at..])
            .map_err(Error::Viewer)?
        {
            let bytes = &from_viewer[at..at + used];
            at += used;
            match message {
                ClientMessage::SetPixelFormat(format) => {
                    if !format.true_colour || !format.is_valid() {
                        return Err(Error::UnsupportedPixelFormat(format));
                    }
                    target_reader.set_pixel_format(&format);
                    self.to_target.extend_from_slice(bytes);
                }
                ClientMessage::SetEncodings(encodings) => {
                    let narrowed = narrow_encodings(&encodings);
                    target_reader.set_last_rect(narrowed.contains(&encoding::LAST_RECT));
                    self.to_target
                        .extend_from_slice(&client_message::set_encodings(&narrowed));
                }
                _ => self.to_target.extend_from_slice(bytes),
            }
        }
        from_viewer.drain(..at);
        Ok(())
    }

    fn relay_target(&mut self, data: &[u8]) -> Result<(), Error> {
        let Phase::Relay {
            target_reader,
            from_target,
            ..
        } = &mut self.phase
        else {
            return Ok(());
        };
        from_target.extend_from_slice(data);
        let mut at = 0;
        while let Some((_, used)) = target_reader
            .next(&from_target[at..])
            .map_err(Error::Target)?
        {
            self.to_viewer
                .extend_from_slice(&from_target[at..at + used]);
            at += used;
        }
        from_target.drain(..at);
        Ok(())
    }
}

/// The encodings from `offered` whose rectangles the Proxy can frame, in the Viewer's order:
/// Raw, CopyRect, Tight, ZRLE, DesktopSize and LastRect, and Tight's quality and compression
/// levels, which change only what is inside a Tight rectangle.
pub fn narrow_encodings(offered: &[i32]) -> Vec<i32> {
    const QUALITY_LEVELS: std::ops::RangeInclusive<i32> = -32..=-23;
    const COMPRESS_LEVELS: std::ops::RangeInclusive<i32> = -256..=-247;
    offered
        .iter()
        .copied()
        .filter(|e| {
            [
                encoding::RAW,
                encoding::COPY_RECT,
                encoding::TIGHT,
                encoding::ZRLE,
                encoding::DESKTOP_SIZE,
                encoding::LAST_RECT,
            ]
            .contains(e)
                || QUALITY_LEVELS.contains(e)
                || COMPRESS_LEVELS.contains(e)
        })
        .collect()
}
