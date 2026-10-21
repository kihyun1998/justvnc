use std::collections::VecDeque;

use justvnc_proto::client_message::{ClientMessage, ClientMessageReader};
use justvnc_proto::server::{SecurityType, ServerHandshake, Step};
use justvnc_proto::vnc_auth::CHALLENGE_LEN;
use justvnc_proto::{
    DesktopName, HandshakeError, Limits, MessageError, PixelFormat, ProtocolVersion, Rect,
    ServerInit, encoding, server_message,
};

use crate::Framebuffer;
use crate::convert::convert;
use crate::region::{Region, clip};

/// How a server session behaves.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The highest version offered: 3.3, 3.7 or 3.8.
    pub version: ProtocolVersion,
    /// The security types offered, most preferred first. Under 3.3 the first is used.
    pub security: Vec<SecurityType>,
    /// The desktop name sent in ServerInit.
    pub name: DesktopName,
    pub limits: Limits,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            version: ProtocolVersion::V3_8,
            security: vec![SecurityType::None],
            name: DesktopName::new(b"justvnc".to_vec()),
            limits: Limits::default(),
        }
    }
}

/// What the session reports to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The Viewer answered the VNC authentication challenge. Decide with
    /// [`ServerSession::accept`] or [`ServerSession::reject`]: for a fixed password,
    /// `justvnc_server::verify(password, &challenge, &response)`.
    ///
    /// VNC authentication uses only the first 8 bytes of a password. A host that takes one-time
    /// tokens as passwords should make each token's first 8 bytes as random as it can.
    Authenticate {
        challenge: [u8; CHALLENGE_LEN],
        response: [u8; CHALLENGE_LEN],
    },
    /// The handshake completed; `shared` is the Viewer's ClientInit shared-flag.
    Connected { shared: bool },
}

/// Why the session stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Handshake(HandshakeError),
    Message(MessageError),
    /// A pixel format this session cannot serve: not valid, or colour-mapped.
    UnsupportedPixelFormat(PixelFormat),
    /// The framebuffer changed size and the Viewer did not offer DesktopSize, so it cannot
    /// follow.
    ViewerCannotResize,
}

/// The server role as a state machine: bytes in, input to the [`Framebuffer`] and bytes to send
/// out.
#[derive(Debug)]
pub struct ServerSession {
    phase: Phase,
    name: DesktopName,
    limits: Limits,
    events: VecDeque<Event>,
    transmit: Vec<u8>,
    failed: Option<Error>,
    /// An incremental request waiting for a change.
    pending: Option<Rect>,
    /// Changes not yet sent.
    damaged: Region,
    /// A move not yet sent as CopyRect: the destination and the source's top left.
    copy: Option<(Rect, u16, u16)>,
    /// The size the Viewer was last told.
    size: (u16, u16),
    /// The size changed and the Viewer has not been told yet.
    resized: bool,
    /// The format the Viewer asked for, if it asked.
    format: Option<PixelFormat>,
    /// Whether the Viewer offered CopyRect, and DesktopSize.
    copy_rect: bool,
    desktop_size: bool,
}

#[derive(Debug)]
enum Phase {
    Handshake(ServerHandshake),
    Messages {
        reader: ClientMessageReader,
        received: Vec<u8>,
    },
}

impl ServerSession {
    /// A session with a random VNC authentication challenge.
    ///
    /// # Panics
    ///
    /// If the operating system's random number source fails.
    pub fn new(config: ServerConfig) -> Self {
        let mut challenge = [0; CHALLENGE_LEN];
        getrandom::fill(&mut challenge).expect("the system random number source works");
        Self::with_challenge(config, challenge)
    }

    /// A session that sends `challenge` if VNC authentication is chosen. It must be random and
    /// never reused.
    ///
    /// # Panics
    ///
    /// If `config.security` is empty or holds more than 255 types.
    pub fn with_challenge(config: ServerConfig, challenge: [u8; CHALLENGE_LEN]) -> Self {
        Self {
            phase: Phase::Handshake(ServerHandshake::new(
                config.version,
                config.security,
                challenge,
            )),
            name: config.name,
            limits: config.limits,
            events: VecDeque::new(),
            transmit: Vec::new(),
            failed: None,
            pending: None,
            damaged: Region::default(),
            copy: None,
            size: (0, 0),
            resized: false,
            format: None,
            copy_rect: false,
            desktop_size: false,
        }
    }

    /// Feeds bytes received from the Viewer. Its input goes to `framebuffer`, and update
    /// requests are answered from it. After an error, every later call returns it again.
    pub fn receive<F: Framebuffer>(
        &mut self,
        data: &[u8],
        framebuffer: &mut F,
    ) -> Result<(), Error> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = self.process(data, framebuffer);
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }

    /// Accepts the response of the last [`Event::Authenticate`], and goes on with any bytes
    /// already received.
    pub fn accept<F: Framebuffer>(&mut self, framebuffer: &mut F) -> Result<(), Error> {
        if let Phase::Handshake(handshake) = &mut self.phase {
            handshake.accept();
        }
        self.receive(&[], framebuffer)
    }

    /// Rejects the response of the last [`Event::Authenticate`]. A 3.8 Viewer is told `reason`;
    /// then the session has ended, and the host closes the connection once the bytes are sent.
    pub fn reject(&mut self, reason: &str) {
        if let Phase::Handshake(handshake) = &mut self.phase {
            handshake.reject(reason);
        }
    }

    /// Tells the session that the pixels in `rect` changed. A waiting incremental request is
    /// answered with the part of the change inside it; otherwise the change is kept for the
    /// next one.
    pub fn damage<F: Framebuffer>(&mut self, rect: Rect, framebuffer: &F) {
        if !self.serving() {
            return;
        }
        if let Some(rect) = clip(rect, framebuffer.size()) {
            self.damaged.add(rect);
        }
        self.answer_waiting(framebuffer);
    }

    /// Tells the session that the pixels now in `rect` were moved there from (`src_x`,
    /// `src_y`), and the rest of the framebuffer is as it was. It is sent as CopyRect when the
    /// Viewer offered it and already has those pixels; otherwise as a change to `rect`.
    pub fn copy<F: Framebuffer>(&mut self, rect: Rect, src_x: u16, src_y: u16, framebuffer: &F) {
        if !self.serving() {
            return;
        }
        let source = Rect {
            x: src_x,
            y: src_y,
            ..rect
        };
        let size = framebuffer.size();
        let inside = |r: Rect| clip(r, size) == Some(r);
        let copyable = self.copy_rect
            && self.copy.is_none()
            && !self.resized
            && inside(rect)
            && inside(source)
            && !self.damaged.intersects(source);
        if copyable {
            self.copy = Some((rect, src_x, src_y));
            self.answer_waiting(framebuffer);
        } else {
            self.damage(rect, framebuffer);
        }
    }

    /// Tells the session that the framebuffer may have changed size. A Viewer that offered
    /// DesktopSize is told at its next request and then sent the whole framebuffer; for one
    /// that did not, the session ends with [`Error::ViewerCannotResize`].
    pub fn resize<F: Framebuffer>(&mut self, framebuffer: &F) -> Result<(), Error> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if !self.serving() || framebuffer.size() == self.size {
            return Ok(());
        }
        if !self.desktop_size {
            self.failed = Some(Error::ViewerCannotResize);
            return Err(Error::ViewerCannotResize);
        }
        self.size = framebuffer.size();
        self.resized = true;
        self.damaged = Region::default();
        self.copy = None;
        if self.pending.take().is_some() {
            self.send_resize();
        }
        Ok(())
    }

    /// Next bytes to send to the Viewer, if any.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        let mut bytes = match &mut self.phase {
            Phase::Handshake(handshake) => handshake.take_transmit(),
            Phase::Messages { .. } => Vec::new(),
        };
        bytes.append(&mut self.transmit);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Next event for the host, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn process<F: Framebuffer>(&mut self, data: &[u8], framebuffer: &mut F) -> Result<(), Error> {
        let data = match &mut self.phase {
            Phase::Handshake(handshake) => {
                match handshake.receive(data).map_err(Error::Handshake)? {
                    Some(Step::Authenticate {
                        challenge,
                        response,
                    }) => {
                        self.events.push_back(Event::Authenticate {
                            challenge,
                            response,
                        });
                        return Ok(());
                    }
                    Some(Step::NoAuthentication) => {
                        unreachable!("the session's handshake does not confirm None")
                    }
                    Some(Step::ClientInit { shared }) => {
                        let format = framebuffer.pixel_format();
                        if !format.true_colour || !format.is_valid() {
                            return Err(Error::UnsupportedPixelFormat(format));
                        }
                        let (width, height) = framebuffer.size();
                        self.size = (width, height);
                        handshake.server_init(&ServerInit {
                            width,
                            height,
                            pixel_format: format,
                            name: self.name.clone(),
                        });
                        self.events.push_back(Event::Connected { shared });
                    }
                    None => return Ok(()),
                }
                let remaining = handshake.take_remaining();
                let mut bytes = handshake.take_transmit();
                bytes.append(&mut self.transmit);
                self.transmit = bytes;
                self.phase = Phase::Messages {
                    reader: ClientMessageReader::new(self.limits),
                    received: Vec::new(),
                };
                remaining
            }
            Phase::Messages { .. } => data.to_vec(),
        };
        self.read_messages(&data, framebuffer)
    }

    fn read_messages<F: Framebuffer>(
        &mut self,
        data: &[u8],
        framebuffer: &mut F,
    ) -> Result<(), Error> {
        let Phase::Messages { reader, received } = &mut self.phase else {
            return Ok(());
        };
        received.extend_from_slice(data);
        let mut messages = Vec::new();
        let mut start = 0;
        while let Some((message, used)) = reader.next(&received[start..]).map_err(Error::Message)? {
            start += used;
            messages.push(message);
        }
        received.drain(..start);
        for message in messages {
            self.handle(message, framebuffer)?;
        }
        Ok(())
    }

    fn handle<F: Framebuffer>(
        &mut self,
        message: ClientMessage,
        framebuffer: &mut F,
    ) -> Result<(), Error> {
        match message {
            ClientMessage::SetPixelFormat(format) => {
                if !format.true_colour || !format.is_valid() {
                    return Err(Error::UnsupportedPixelFormat(format));
                }
                self.format = Some(format);
            }
            ClientMessage::SetEncodings(encodings) => {
                self.copy_rect = encodings.contains(&encoding::COPY_RECT);
                self.desktop_size = encodings.contains(&encoding::DESKTOP_SIZE);
            }
            ClientMessage::FramebufferUpdateRequest { incremental, rect } => {
                let area = clip(rect, framebuffer.size());
                if self.resized {
                    self.send_resize();
                } else if !incremental {
                    if let Some(area) = area {
                        self.damaged.take(area);
                    }
                    self.send_update(&Vec::from_iter(area), framebuffer);
                } else if let Some(area) = area {
                    self.pending = Some(area);
                    self.answer_waiting(framebuffer);
                }
            }
            ClientMessage::KeyEvent { down, keysym } => framebuffer.key_event(down, keysym),
            ClientMessage::PointerEvent { buttons, x, y } => {
                framebuffer.pointer_event(buttons, x, y)
            }
            ClientMessage::CutText(text) => framebuffer.cut_text(text),
        }
        Ok(())
    }

    fn serving(&self) -> bool {
        self.failed.is_none() && matches!(self.phase, Phase::Messages { .. })
    }

    /// Answers a waiting incremental request if there is something to send inside it.
    fn answer_waiting<F: Framebuffer>(&mut self, framebuffer: &F) {
        let Some(area) = self.pending else {
            return;
        };
        let changed = self.damaged.take(area);
        if !changed.is_empty() || self.copy.is_some() {
            self.pending = None;
            self.send_update(&changed, framebuffer);
        }
    }

    /// A FramebufferUpdate holding only the DesktopSize pseudo-rectangle. Everything is then
    /// changed for the Viewer, which may have discarded its framebuffer.
    fn send_resize(&mut self) {
        let (width, height) = self.size;
        let whole = Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        self.transmit
            .extend_from_slice(&server_message::framebuffer_update(1));
        self.transmit
            .extend_from_slice(&server_message::rectangle(whole, encoding::DESKTOP_SIZE));
        self.resized = false;
        if width > 0 && height > 0 {
            self.damaged.add(whole);
        }
    }

    /// A FramebufferUpdate holding the move waiting to be sent, as CopyRect, then `rects` as
    /// Raw in the Viewer's format.
    fn send_update<F: Framebuffer>(&mut self, rects: &[Rect], framebuffer: &F) {
        let copy = self.copy.take();
        let count = u16::try_from(rects.len() + usize::from(copy.is_some()))
            .expect("a region holds few rectangles");
        self.transmit
            .extend_from_slice(&server_message::framebuffer_update(count));
        if let Some((rect, src_x, src_y)) = copy {
            self.transmit
                .extend_from_slice(&server_message::rectangle(rect, encoding::COPY_RECT));
            self.transmit.extend_from_slice(&src_x.to_be_bytes());
            self.transmit.extend_from_slice(&src_y.to_be_bytes());
        }
        let native = framebuffer.pixel_format();
        let format = self.format.unwrap_or(native);
        for &rect in rects {
            let pixels = framebuffer.read(rect);
            let expected =
                usize::from(rect.width) * usize::from(rect.height) * native.bytes_per_pixel();
            assert_eq!(
                pixels.len(),
                expected,
                "Framebuffer::read returned the wrong number of bytes for {rect:?}"
            );
            self.transmit
                .extend_from_slice(&server_message::rectangle(rect, encoding::RAW));
            self.transmit
                .extend_from_slice(&convert(&pixels, &native, &format));
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Handshake(e) => e.fmt(f),
            Self::Message(e) => e.fmt(f),
            Self::UnsupportedPixelFormat(format) => {
                write!(f, "pixel format not supported: {format}")
            }
            Self::ViewerCannotResize => {
                f.write_str("the framebuffer changed size and the Viewer did not offer DesktopSize")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Handshake(e) => e.source(),
            Self::Message(e) => e.source(),
            Self::UnsupportedPixelFormat(_) | Self::ViewerCannotResize => None,
        }
    }
}
