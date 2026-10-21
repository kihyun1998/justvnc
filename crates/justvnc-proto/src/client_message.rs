//! Client-to-server messages: encoded for the wire, and framed from it.

use crate::{CutText, Limits, MessageError, PixelFormat, Rect};

/// One message from the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    SetPixelFormat(PixelFormat),
    /// The encodings the client accepts, most preferred first.
    SetEncodings(Vec<i32>),
    FramebufferUpdateRequest {
        incremental: bool,
        rect: Rect,
    },
    KeyEvent {
        down: bool,
        keysym: u32,
    },
    /// The pointer at (`x`, `y`) with buttons 1 to 8 in bits 0 to 7.
    PointerEvent {
        buttons: u8,
        x: u16,
        y: u16,
    },
    /// ClientCutText.
    CutText(CutText),
}

/// Frames the client's messages after ClientInit, one at a time.
#[derive(Debug)]
pub struct ClientMessageReader {
    limits: Limits,
}

const SET_PIXEL_FORMAT: u8 = 0;
const SET_ENCODINGS: u8 = 2;
const FRAMEBUFFER_UPDATE_REQUEST: u8 = 3;
const KEY_EVENT: u8 = 4;
const POINTER_EVENT: u8 = 5;
const CLIENT_CUT_TEXT: u8 = 6;

impl ClientMessageReader {
    pub fn new(limits: Limits) -> Self {
        Self { limits }
    }

    /// Frames the message at the start of `buf`. Returns it and how many bytes of `buf` it used,
    /// or `None` when `buf` does not hold all of it yet.
    pub fn next(&mut self, buf: &[u8]) -> Result<Option<(ClientMessage, usize)>, MessageError> {
        let Some(&message_type) = buf.first() else {
            return Ok(None);
        };
        let u16_at = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]);
        let u32_at = |i: usize| u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        let fixed = |len: usize| buf.len() >= len;
        Ok(match message_type {
            SET_PIXEL_FORMAT if fixed(20) => {
                let format = PixelFormat::from_bytes(buf[4..20].try_into().unwrap());
                Some((ClientMessage::SetPixelFormat(format), 20))
            }
            SET_ENCODINGS if fixed(4) => {
                let end = 4 + 4 * usize::from(u16_at(2));
                buf.get(4..end).map(|list| {
                    let encodings = list
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|e| i32::from_be_bytes(*e))
                        .collect();
                    (ClientMessage::SetEncodings(encodings), end)
                })
            }
            FRAMEBUFFER_UPDATE_REQUEST if fixed(10) => {
                let rect = Rect {
                    x: u16_at(2),
                    y: u16_at(4),
                    width: u16_at(6),
                    height: u16_at(8),
                };
                let incremental = buf[1] != 0;
                Some((
                    ClientMessage::FramebufferUpdateRequest { incremental, rect },
                    10,
                ))
            }
            KEY_EVENT if fixed(8) => Some((
                ClientMessage::KeyEvent {
                    down: buf[1] != 0,
                    keysym: u32_at(4),
                },
                8,
            )),
            POINTER_EVENT if fixed(6) => Some((
                ClientMessage::PointerEvent {
                    buttons: buf[1],
                    x: u16_at(2),
                    y: u16_at(4),
                },
                6,
            )),
            CLIENT_CUT_TEXT if fixed(8) => {
                let length = u32_at(4);
                let max = self.limits.max_cut_text_length;
                if length > max {
                    return Err(MessageError::LimitExceeded {
                        field: "cut-text-length",
                        length,
                        max,
                    });
                }
                let end = 8usize.saturating_add(length as usize);
                buf.get(8..end).map(|text| {
                    let text = CutText::from_latin1(text.to_vec());
                    (ClientMessage::CutText(text), end)
                })
            }
            SET_PIXEL_FORMAT
            | SET_ENCODINGS
            | FRAMEBUFFER_UPDATE_REQUEST
            | KEY_EVENT
            | POINTER_EVENT
            | CLIENT_CUT_TEXT => None,
            _ => return Err(MessageError::UnsupportedMessage { message_type }),
        })
    }
}

/// SetPixelFormat (message type 0).
pub fn set_pixel_format(format: &PixelFormat) -> [u8; 20] {
    let mut out = [0u8; 20];
    out[4..].copy_from_slice(&format.to_bytes());
    out
}

/// SetEncodings (message type 2), most preferred first.
///
/// # Panics
///
/// If there are more than 65535 encodings.
pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let count = u16::try_from(encodings.len()).expect("at most 65535 encodings");
    let mut out = vec![2, 0];
    out.extend_from_slice(&count.to_be_bytes());
    for encoding in encodings {
        out.extend_from_slice(&encoding.to_be_bytes());
    }
    out
}

/// FramebufferUpdateRequest (message type 3).
pub fn framebuffer_update_request(incremental: bool, area: Rect) -> [u8; 10] {
    let mut out = [3, u8::from(incremental), 0, 0, 0, 0, 0, 0, 0, 0];
    out[2..4].copy_from_slice(&area.x.to_be_bytes());
    out[4..6].copy_from_slice(&area.y.to_be_bytes());
    out[6..8].copy_from_slice(&area.width.to_be_bytes());
    out[8..10].copy_from_slice(&area.height.to_be_bytes());
    out
}

/// KeyEvent (message type 4): `keysym` pressed or released.
pub fn key_event(down: bool, keysym: u32) -> [u8; 8] {
    let [a, b, c, d] = keysym.to_be_bytes();
    [4, u8::from(down), 0, 0, a, b, c, d]
}

/// PointerEvent (message type 5): the pointer at (`x`, `y`) with buttons 1 to 8 in bits 0 to 7.
pub fn pointer_event(buttons: u8, x: u16, y: u16) -> [u8; 6] {
    let [xh, xl] = x.to_be_bytes();
    let [yh, yl] = y.to_be_bytes();
    [5, buttons, xh, xl, yh, yl]
}

/// ClientCutText (message type 6).
///
/// # Panics
///
/// If the text is longer than `u32::MAX` bytes.
pub fn client_cut_text(text: &CutText) -> Vec<u8> {
    let bytes = text.as_bytes();
    let length = u32::try_from(bytes.len()).expect("cut text fits a U32 length");
    let mut out = vec![6, 0, 0, 0];
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
    out
}

/// `message` encoded for the wire.
///
/// # Panics
///
/// As the encoder for its type does.
pub fn encode(message: &ClientMessage) -> Vec<u8> {
    match message {
        ClientMessage::SetPixelFormat(format) => set_pixel_format(format).to_vec(),
        ClientMessage::SetEncodings(encodings) => set_encodings(encodings),
        ClientMessage::FramebufferUpdateRequest { incremental, rect } => {
            framebuffer_update_request(*incremental, *rect).to_vec()
        }
        ClientMessage::KeyEvent { down, keysym } => key_event(*down, *keysym).to_vec(),
        ClientMessage::PointerEvent { buttons, x, y } => pointer_event(*buttons, *x, *y).to_vec(),
        ClientMessage::CutText(text) => client_cut_text(text),
    }
}
