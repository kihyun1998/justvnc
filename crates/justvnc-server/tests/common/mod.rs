//! A server session and a client session joined back to back. Both are sans-IO, so the tests
//! move their bytes between them.
#![allow(dead_code)]

use justvnc_client::{ClientConfig, ClientSession, Event as ClientEvent};
use justvnc_proto::vnc_auth;
use justvnc_server::{Error, Event, Framebuffer, PixelFormat, Rect, ServerConfig, ServerSession};

pub const XRGB: PixelFormat = PixelFormat {
    bits_per_pixel: 32,
    depth: 24,
    big_endian: false,
    true_colour: true,
    red_max: 255,
    green_max: 255,
    blue_max: 255,
    red_shift: 16,
    green_shift: 8,
    blue_shift: 0,
};

/// The rectangles of `events`, in order.
pub fn rectangles(events: &[ClientEvent]) -> Vec<Rect> {
    events
        .iter()
        .filter_map(|e| match e {
            ClientEvent::Rectangle(r) => Some(r.rect),
            _ => None,
        })
        .collect()
}

pub struct Pair<F> {
    pub server: ServerSession,
    pub client: ClientSession,
    pub pattern: F,
    pub password: Option<&'static [u8]>,
    pub server_error: Option<Error>,
    pub client_error: Option<justvnc_client::Error>,
    pub server_events: Vec<Event>,
}

impl<F: Framebuffer + Default> Pair<F> {
    pub fn new(server: ServerConfig, client: ClientConfig) -> Self {
        Self::with(server, client, F::default())
    }
}

impl<F: Framebuffer> Pair<F> {
    pub fn with(server: ServerConfig, client: ClientConfig, pattern: F) -> Self {
        Self {
            server: ServerSession::with_challenge(server, [0x5a; 16]),
            client: ClientSession::new(client),
            pattern,
            password: None,
            server_error: None,
            client_error: None,
            server_events: Vec::new(),
        }
    }

    /// Moves bytes both ways until neither side has anything to send, deciding VNC
    /// authentication with `self.password`.
    pub fn pump(&mut self) {
        loop {
            let mut moved = false;
            while let Some(bytes) = self.server.poll_transmit() {
                moved = true;
                if self.client_error.is_none() {
                    self.client_error = self.client.receive(&bytes).err();
                }
            }
            while let Some(bytes) = self.client.poll_transmit() {
                moved = true;
                if self.server_error.is_none() {
                    self.server_error = self.server.receive(&bytes, &mut self.pattern).err();
                }
            }
            while let Some(event) = self.server.poll_event() {
                moved = true;
                if let Event::Authenticate {
                    challenge,
                    response,
                } = &event
                {
                    let password = self.password.unwrap_or_default();
                    if vnc_auth::verify(password, challenge, response) {
                        self.server_error = self.server.accept(&mut self.pattern).err();
                    } else {
                        self.server.reject("Authentication failure");
                    }
                }
                self.server_events.push(event);
            }
            if !moved {
                return;
            }
        }
    }

    pub fn client_events(&mut self) -> Vec<ClientEvent> {
        std::iter::from_fn(|| self.client.poll_event()).collect()
    }
}
