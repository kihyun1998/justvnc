use std::collections::VecDeque;

use justvnc_proto::client::ClientHandshake;
use justvnc_proto::client_message::{self, ClientMessage, ClientMessageReader};
use justvnc_proto::server::{SecurityType, ServerHandshake, Step};
use justvnc_proto::server_message::{self, Framed, ServerMessageReader};
use justvnc_proto::vnc_auth::CHALLENGE_LEN;
use justvnc_proto::{
    HandshakeError, Limits, MessageError, Password, PixelFormat, ProtocolVersion, ServerInit,
    encoding,
};

use crate::tap::{ForwardAll, Tap, TargetMessage, UpdateVerdict, Verdict};

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
    /// The Viewer passed security: its response was accepted, or it chose None.
    ViewerAuthenticated,
    /// Follows [`Event::ViewerAuthenticated`]. Connect to the Target and call
    /// [`ProxySession::connect_target`], or [`ProxySession::refuse`] it.
    TargetNeeded,
    /// The Target's handshake completed. `desktop` is its ServerInit, which the Viewer is sent
    /// as it is; messages are relayed from then on.
    TargetConnected { desktop: ServerInit },
    /// The session ended. It is the last event, and comes once.
    Closed { reason: CloseReason },
}

/// Why a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// The host reported that the Viewer closed its connection.
    ViewerClosed,
    /// The host reported that the Target closed its connection.
    TargetClosed,
    /// The host rejected the Viewer or refused it a Target, with this reason.
    Refused(String),
    /// The Tap ended the session, with this reason.
    Ended(String),
    /// A leg broke the protocol, or failed its handshake.
    Failed(Error),
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
    /// The Tap ended the session, with this reason.
    Ended(String),
}

/// The reason a Viewer is given when the Target leg fails during the handshake. The Target's own
/// reason is not passed on.
const TARGET_FAILED: &str = "Connection to the Target failed";

/// A Proxy between one Viewer and one Target, as a state machine: bytes in from either leg,
/// bytes out to either leg.
#[derive(Debug)]
pub struct ProxySession<T = ForwardAll> {
    phase: Phase,
    tap: T,
    closed: bool,
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
            tap: ForwardAll,
            closed: false,
            shared: config.shared,
            limits: config.limits,
            events: VecDeque::new(),
            to_viewer: Vec::new(),
            to_target: Vec::new(),
            failed: None,
        }
    }
}

impl<T: Tap> ProxySession<T> {
    /// The session with `tap` looking at every message it relays.
    pub fn with_tap<U: Tap>(self, tap: U) -> ProxySession<U> {
        ProxySession {
            phase: self.phase,
            tap,
            closed: self.closed,
            shared: self.shared,
            limits: self.limits,
            events: self.events,
            to_viewer: self.to_viewer,
            to_target: self.to_target,
            failed: self.failed,
        }
    }

    pub fn tap(&self) -> &T {
        &self.tap
    }

    pub fn tap_mut(&mut self) -> &mut T {
        &mut self.tap
    }

    /// Tells the session that the Viewer closed its connection; [`Event::Closed`] follows.
    pub fn viewer_closed(&mut self) {
        self.close(CloseReason::ViewerClosed);
    }

    /// Tells the session that the Target closed its connection; [`Event::Closed`] follows.
    pub fn target_closed(&mut self) {
        self.close(CloseReason::TargetClosed);
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
            self.events.push_back(Event::ViewerAuthenticated);
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
            self.close(CloseReason::Refused(reason.to_owned()));
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
            let reason = match error {
                Error::Ended(reason) => CloseReason::Ended(reason.clone()),
                error => CloseReason::Failed(error.clone()),
            };
            self.close(reason);
        }
        result
    }

    fn close(&mut self, reason: CloseReason) {
        if !self.closed {
            self.closed = true;
            self.events.push_back(Event::Closed { reason });
        }
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
        self.events.push_back(Event::TargetConnected {
            desktop: init.clone(),
        });
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
        self.relay_viewer(&from_viewer)?;
        self.relay_target(&from_target)
    }

    fn relay_viewer(&mut self, data: &[u8]) -> Result<(), Error> {
        let Self {
            phase:
                Phase::Relay {
                    viewer_reader,
                    target_reader,
                    from_viewer,
                    ..
                },
            tap,
            to_target,
            ..
        } = self
        else {
            return Ok(());
        };
        from_viewer.extend_from_slice(data);
        let mut at = 0;
        let result = (|| {
            while let Some((message, used)) = viewer_reader
                .next(&from_viewer[at..])
                .map_err(Error::Viewer)?
            {
                let bytes = &from_viewer[at..at + used];
                at += used;
                let (message, bytes) = match tap.viewer_message(&message) {
                    Verdict::Forward => (message, Some(bytes)),
                    Verdict::Drop => continue,
                    Verdict::Replace(replacement) => (replacement, None),
                    Verdict::End(reason) => return Err(Error::Ended(reason)),
                };
                let encoded =
                    || bytes.map_or_else(|| client_message::encode(&message), <[u8]>::to_vec);
                match &message {
                    ClientMessage::SetPixelFormat(format) => {
                        if !format.true_colour || !format.is_valid() {
                            return Err(Error::UnsupportedPixelFormat(*format));
                        }
                        target_reader.set_pixel_format(format);
                        to_target.extend(encoded());
                    }
                    ClientMessage::SetEncodings(encodings) => {
                        let narrowed = narrow_encodings(encodings);
                        target_reader.set_last_rect(narrowed.contains(&encoding::LAST_RECT));
                        to_target.extend(client_message::set_encodings(&narrowed));
                    }
                    _ => to_target.extend(encoded()),
                }
            }
            Ok(())
        })();
        from_viewer.drain(..at);
        result
    }

    fn relay_target(&mut self, data: &[u8]) -> Result<(), Error> {
        let Self {
            phase:
                Phase::Relay {
                    target_reader,
                    from_target,
                    ..
                },
            tap,
            to_viewer,
            ..
        } = self
        else {
            return Ok(());
        };
        from_target.extend_from_slice(data);
        let mut at = 0;
        let result = (|| {
            while let Some((framed, used)) = target_reader
                .next(&from_target[at..])
                .map_err(Error::Target)?
            {
                let bytes = &from_target[at..at + used];
                at += used;
                let message = match framed {
                    Framed::Bell => TargetMessage::Bell,
                    Framed::CutText(text) => TargetMessage::CutText(text),
                    piece => match tap.target_update(&piece) {
                        UpdateVerdict::Forward => {
                            to_viewer.extend_from_slice(bytes);
                            continue;
                        }
                        UpdateVerdict::End(reason) => return Err(Error::Ended(reason)),
                    },
                };
                match tap.target_message(&message) {
                    Verdict::Forward => to_viewer.extend_from_slice(bytes),
                    Verdict::Drop => {}
                    Verdict::Replace(TargetMessage::Bell) => {
                        to_viewer.extend(server_message::bell());
                    }
                    Verdict::Replace(TargetMessage::CutText(text)) => {
                        to_viewer.extend(server_message::server_cut_text(&text));
                    }
                    Verdict::End(reason) => return Err(Error::Ended(reason)),
                }
            }
            Ok(())
        })();
        from_target.drain(..at);
        result
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
