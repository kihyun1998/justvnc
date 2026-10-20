use std::collections::VecDeque;

use justvnc_decode::{DecodeError, Decoder};
use justvnc_proto::client::ClientHandshake;
use justvnc_proto::server_message::{Framed, ServerMessageReader};
use justvnc_proto::{
    CutText, HandshakeError, Limits, MessageError, Password, PixelFormat, Rect, ServerInit,
    client_message, encoding,
};

/// Encodings the session asks for, most preferred first.
const ENCODINGS: [i32; 2] = [encoding::DESKTOP_SIZE, encoding::RAW];

/// How a client session behaves.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Ask the server to leave other clients connected (ClientInit shared-flag).
    pub shared: bool,
    /// Password for VNC authentication. Without one, VNC authentication is never chosen.
    pub password: Option<Password>,
    /// Pixel format to ask the server for. Without one, the server's own format is used.
    pub pixel_format: Option<PixelFormat>,
    pub limits: Limits,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            shared: true,
            password: None,
            pixel_format: None,
            limits: Limits::default(),
        }
    }
}

/// What the session reports to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The handshake completed and the server described its desktop.
    Connected(ServerInit),
    /// Pixels for part of the framebuffer.
    Rectangle(Rectangle),
    /// The framebuffer is now `width` x `height`.
    DesktopResized { width: u16, height: u16 },
    /// Every rectangle of one FramebufferUpdate has been reported.
    UpdateComplete,
    /// The server has new clipboard text (ServerCutText).
    CutText(CutText),
    /// The server rang its bell.
    Bell,
}

/// Decoded pixels for `rect`, row by row from its top left, in the session's pixel format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rectangle {
    pub rect: Rect,
    pub pixels: Vec<u8>,
}

/// Why the session stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Handshake(HandshakeError),
    Message(MessageError),
    Decode(DecodeError),
    /// The pixel format in use is one this session cannot decode: colour-mapped, or not 8, 16
    /// or 32 bits per pixel.
    UnsupportedPixelFormat(PixelFormat),
}

/// The client role as a state machine: bytes in, events and bytes to send out.
#[derive(Debug)]
pub struct ClientSession {
    phase: Phase,
    requested_format: Option<PixelFormat>,
    limits: Limits,
    events: VecDeque<Event>,
    transmit: Vec<u8>,
    failed: Option<Error>,
}

#[derive(Debug)]
enum Phase {
    Handshake(ClientHandshake),
    Messages {
        reader: ServerMessageReader,
        decoder: Decoder,
        format: PixelFormat,
        received: Vec<u8>,
    },
}

impl ClientSession {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            phase: Phase::Handshake(ClientHandshake::new(
                config.shared,
                config.password,
                config.limits,
            )),
            requested_format: config.pixel_format,
            limits: config.limits,
            events: VecDeque::new(),
            transmit: Vec::new(),
            failed: None,
        }
    }

    /// Feeds bytes received from the server. After an error, every later call returns it again.
    pub fn receive(&mut self, data: &[u8]) -> Result<(), Error> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let result = self.process(data);
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }

    /// Asks the server for the contents of `area`: everything, or only what changed.
    pub fn request_update(&mut self, incremental: bool, area: Rect) {
        self.transmit
            .extend_from_slice(&client_message::framebuffer_update_request(
                incremental,
                area,
            ));
    }

    /// Tells the server that `keysym` (an X11 keysym) was pressed or released.
    pub fn key_event(&mut self, down: bool, keysym: u32) {
        self.transmit
            .extend_from_slice(&client_message::key_event(down, keysym));
    }

    /// Tells the server where the pointer is and which buttons are held: buttons 1 to 8 are
    /// bits 0 to 7, and a wheel step is a press and release of button 4 (up) or 5 (down).
    pub fn pointer_event(&mut self, buttons: u8, x: u16, y: u16) {
        self.transmit
            .extend_from_slice(&client_message::pointer_event(buttons, x, y));
    }

    /// Gives the server new clipboard text.
    pub fn send_cut_text(&mut self, text: &CutText) {
        self.transmit
            .extend_from_slice(&client_message::client_cut_text(text));
    }

    /// The pixel format rectangles arrive in, once the handshake has completed.
    pub fn pixel_format(&self) -> Option<PixelFormat> {
        match &self.phase {
            Phase::Messages { format, .. } => Some(*format),
            Phase::Handshake(_) => None,
        }
    }

    /// Next bytes to send to the server, if any.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        if let Phase::Handshake(handshake) = &mut self.phase {
            let mut bytes = handshake.take_transmit();
            bytes.append(&mut self.transmit);
            return (!bytes.is_empty()).then_some(bytes);
        }
        let bytes = std::mem::take(&mut self.transmit);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Next event for the host, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn process(&mut self, data: &[u8]) -> Result<(), Error> {
        let data = match &mut self.phase {
            Phase::Handshake(handshake) => {
                let Some(init) = handshake.receive(data).map_err(Error::Handshake)? else {
                    return Ok(());
                };
                let remaining = handshake.take_remaining();
                let mut handshake_bytes = handshake.take_transmit();
                handshake_bytes.append(&mut self.transmit);
                self.transmit = handshake_bytes;
                self.start_messages(init)?;
                remaining
            }
            Phase::Messages { .. } => data.to_vec(),
        };
        self.read_messages(&data)
    }

    fn start_messages(&mut self, init: ServerInit) -> Result<(), Error> {
        self.limits
            .check_framebuffer(init.width, init.height)
            .map_err(Error::Message)?;
        let format = self.requested_format.unwrap_or(init.pixel_format);
        if !format.true_colour || !matches!(format.bits_per_pixel, 8 | 16 | 32) {
            return Err(Error::UnsupportedPixelFormat(format));
        }
        if let Some(requested) = &self.requested_format {
            self.transmit
                .extend_from_slice(&client_message::set_pixel_format(requested));
        }
        self.transmit
            .extend_from_slice(&client_message::set_encodings(&ENCODINGS));
        self.phase = Phase::Messages {
            reader: ServerMessageReader::new(&format, init.width, init.height, self.limits),
            decoder: Decoder::new(),
            format,
            received: Vec::new(),
        };
        self.events.push_back(Event::Connected(init));
        Ok(())
    }

    fn read_messages(&mut self, data: &[u8]) -> Result<(), Error> {
        let Phase::Messages {
            reader,
            decoder,
            format,
            received,
        } = &mut self.phase
        else {
            return Ok(());
        };
        received.extend_from_slice(data);
        let mut start = 0;
        while let Some((framed, used)) = reader.next(&received[start..]).map_err(Error::Message)? {
            start += used;
            let event = match framed {
                Framed::Rectangle {
                    rect,
                    encoding,
                    data,
                } => Event::Rectangle(Rectangle {
                    rect,
                    pixels: decoder
                        .decode(rect, encoding, &data, format)
                        .map_err(Error::Decode)?,
                }),
                Framed::DesktopSize { width, height } => Event::DesktopResized { width, height },
                Framed::UpdateEnd => Event::UpdateComplete,
                Framed::Bell => Event::Bell,
                Framed::CutText(text) => Event::CutText(text),
            };
            self.events.push_back(event);
        }
        received.drain(..start);
        Ok(())
    }
}
