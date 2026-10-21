use justvnc_proto::client_message::{self, ClientMessage, ClientMessageReader};
use justvnc_proto::{CutText, Limits, MessageError, PixelFormat, Rect};

const RGB565: PixelFormat = PixelFormat {
    bits_per_pixel: 16,
    depth: 16,
    big_endian: false,
    true_colour: true,
    red_max: 31,
    green_max: 63,
    blue_max: 31,
    red_shift: 11,
    green_shift: 5,
    blue_shift: 0,
};

fn frame_all(bytes: &[u8]) -> Vec<ClientMessage> {
    let mut reader = ClientMessageReader::new(Limits::default());
    let mut at = 0;
    let mut out = Vec::new();
    while let Some((message, used)) = reader.next(&bytes[at..]).unwrap() {
        out.push(message);
        at += used;
    }
    assert_eq!(at, bytes.len(), "bytes left over");
    out
}

#[test]
fn frames_what_the_recorder_sent_tigervnc_after_client_init() {
    let client = include_bytes!(
        "../../justvnc-client/tests/fixtures/tigervnc-3.8-none-retro-update.client.bin"
    );

    let messages = frame_all(&client[14..]);

    assert_eq!(
        messages,
        [
            ClientMessage::SetEncodings(vec![-223, 1, 0]),
            ClientMessage::FramebufferUpdateRequest {
                incremental: false,
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 48,
                    height: 24
                }
            }
        ]
    );
}

fn every_message() -> Vec<(ClientMessage, Vec<u8>)> {
    let area = Rect {
        x: 1,
        y: 2,
        width: 300,
        height: 400,
    };
    let text = CutText::from_latin1(b"caf\xe9".to_vec());
    vec![
        (
            ClientMessage::SetPixelFormat(RGB565),
            client_message::set_pixel_format(&RGB565).to_vec(),
        ),
        (
            ClientMessage::SetEncodings(vec![7, 16, 0, -223]),
            client_message::set_encodings(&[7, 16, 0, -223]),
        ),
        (
            ClientMessage::FramebufferUpdateRequest {
                incremental: true,
                rect: area,
            },
            client_message::framebuffer_update_request(true, area).to_vec(),
        ),
        (
            ClientMessage::KeyEvent {
                down: true,
                keysym: 0xffe1,
            },
            client_message::key_event(true, 0xffe1).to_vec(),
        ),
        (
            ClientMessage::PointerEvent {
                buttons: 0b101,
                x: 640,
                y: 480,
            },
            client_message::pointer_event(0b101, 640, 480).to_vec(),
        ),
        (
            ClientMessage::CutText(text.clone()),
            client_message::client_cut_text(&text),
        ),
    ]
}

#[test]
fn frames_each_message_the_client_role_sends() {
    for (expected, bytes) in every_message() {
        assert_eq!(frame_all(&bytes), [expected]);
    }
}

#[test]
fn waits_for_every_byte_of_a_message() {
    for (_, bytes) in every_message() {
        let mut reader = ClientMessageReader::new(Limits::default());
        for end in 0..bytes.len() {
            assert_eq!(
                reader.next(&bytes[..end]),
                Ok(None),
                "{bytes:?} cut at {end}"
            );
        }
    }
}

#[test]
fn takes_a_down_flag_and_an_incremental_flag_as_any_non_zero_byte() {
    let messages = frame_all(&[4, 0xff, 0, 0, 0, 0, 0, 0x61, 3, 2, 0, 0, 0, 0, 0, 1, 0, 1]);

    assert_eq!(
        messages,
        [
            ClientMessage::KeyEvent {
                down: true,
                keysym: 0x61
            },
            ClientMessage::FramebufferUpdateRequest {
                incremental: true,
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                }
            }
        ]
    );
}

#[test]
fn refuses_cut_text_longer_than_the_limit_before_it_arrives() {
    let mut reader = ClientMessageReader::new(Limits {
        max_cut_text_length: 4,
        ..Limits::default()
    });

    assert_eq!(
        reader.next(&[6, 0, 0, 0, 0, 0, 0, 5]),
        Err(MessageError::LimitExceeded {
            field: "cut-text-length",
            length: 5,
            max: 4
        })
    );
}

#[test]
fn stops_at_a_message_type_it_does_not_know() {
    let mut reader = ClientMessageReader::new(Limits::default());

    assert_eq!(
        reader.next(&[150, 0, 0, 0]),
        Err(MessageError::UnsupportedMessage { message_type: 150 })
    );
}

#[test]
fn frames_a_set_encodings_longer_than_255() {
    let encodings: Vec<i32> = (0..300).collect();

    let messages = frame_all(&client_message::set_encodings(&encodings));

    assert_eq!(messages, [ClientMessage::SetEncodings(encodings)]);
}
