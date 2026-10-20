use justvnc_client::{
    ClientConfig, ClientSession, CutText, Error, Event, Limits, MessageError, NotLatin1,
};

/// TigerVNC answering an X clipboard change to "from-x\né" (UTF-8) and one `xkbbell`.
const CUT_TEXT_AND_BELL: &[u8] =
    include_bytes!("fixtures/tigervnc-3.8-none-cut-text-and-bell.server.bin");

fn drain_events(session: &mut ClientSession) -> Vec<Event> {
    std::iter::from_fn(|| session.poll_event()).collect()
}

#[test]
fn reports_tigervncs_clipboard_text_and_bells() {
    let mut session = ClientSession::new(ClientConfig::default());

    session.receive(CUT_TEXT_AND_BELL).unwrap();

    let events = drain_events(&mut session);
    assert!(matches!(events[0], Event::Connected(_)));
    // TigerVNC turns the UTF-8 X selection into Latin-1, and one xkbbell rings three times.
    assert_eq!(
        events[1..],
        [
            Event::CutText(CutText::from_latin1(b"from-x\n\xe9".to_vec())),
            Event::Bell,
            Event::Bell,
            Event::Bell,
        ]
    );
    let Event::CutText(text) = &events[1] else {
        unreachable!()
    };
    assert_eq!(text.to_string(), "from-x\né");
}

/// Server bytes up to the end of ServerInit in the recording.
const HANDSHAKE_LEN: usize = 54;

fn connected(config: ClientConfig) -> ClientSession {
    let mut session = ClientSession::new(config);
    session
        .receive(&CUT_TEXT_AND_BELL[..HANDSHAKE_LEN])
        .unwrap();
    while session.poll_transmit().is_some() {}
    drain_events(&mut session);
    session
}

fn transmit(session: &mut ClientSession) -> Vec<u8> {
    std::iter::from_fn(|| session.poll_transmit())
        .flatten()
        .collect()
}

#[test]
fn rejects_clipboard_text_longer_than_the_limit_before_it_arrives() {
    let config = ClientConfig {
        limits: Limits {
            max_cut_text_length: 7,
            ..Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = connected(config);

    // The recorded ServerCutText header announces 8 bytes; the text itself is not fed.
    let result = session.receive(&CUT_TEXT_AND_BELL[HANDSHAKE_LEN..][..8]);

    assert_eq!(
        result,
        Err(Error::Message(MessageError::LimitExceeded {
            field: "cut-text-length",
            length: 8,
            max: 7
        }))
    );
}

#[test]
fn sends_key_events() {
    let mut session = connected(ClientConfig::default());

    session.key_event(true, 0x61);
    session.key_event(false, 0xff0d);

    // RFC 6143 7.5.4: type 4, down-flag, 2 bytes padding, keysym as U32.
    assert_eq!(
        transmit(&mut session),
        [4, 1, 0, 0, 0, 0, 0, 0x61, 4, 0, 0, 0, 0, 0, 0xff, 0x0d]
    );
}

#[test]
fn sends_pointer_events() {
    let mut session = connected(ClientConfig::default());

    session.pointer_event(0b0000_0101, 300, 2);

    // RFC 6143 7.5.5: type 5, button-mask, then x and y as U16.
    assert_eq!(transmit(&mut session), [5, 0b0000_0101, 0x01, 0x2c, 0, 2]);
}

#[test]
fn sends_clipboard_text_as_latin1() {
    let mut session = connected(ClientConfig::default());

    session.send_cut_text(&CutText::from_text("café\r\nline2").unwrap());

    // RFC 6143 7.5.6: type 6, 3 bytes padding, length as U32, Latin-1 text with LF line ends.
    let mut expected = vec![6, 0, 0, 0, 0, 0, 0, 10];
    expected.extend_from_slice(b"caf\xe9\nline2");
    assert_eq!(transmit(&mut session), expected);
}

#[test]
fn refuses_clipboard_text_latin1_cannot_hold() {
    assert_eq!(
        CutText::from_text("안녕"),
        Err(NotLatin1 { character: '안' })
    );
}

#[test]
fn keeps_a_carriage_return_that_does_not_end_a_line() {
    assert_eq!(CutText::from_text("a\rb").unwrap().as_bytes(), b"a\rb");
}
