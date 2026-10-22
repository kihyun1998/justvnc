//! The Tap and the session's lifecycle events, with recorded bytes on both legs.

use justvnc_proto::client_message::{self, ClientMessageReader};
use justvnc_proto::server::SecurityType;
use justvnc_proto::vnc_auth;
use justvnc_proto::{Limits, PixelFormat};
use justvnc_proxy::{
    ClientMessage, CloseReason, CutText, Error, Event, Framed, ProxyConfig, ProxySession,
    ServerInit, Tap, TargetMessage, UpdateVerdict, Verdict,
};

macro_rules! client_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-client/tests/fixtures/", $name)).as_slice()
    };
}
macro_rules! server_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-server/tests/fixtures/", $name)).as_slice()
    };
}

/// TightVNC Viewer 2.8.85: keys, pointer, and one ClientCutText of "\nThe \"close".
const VIEWER: &[u8] = server_fixture!("tightvnc-viewer-3.8-vncauth.viewer.bin");
const VIEWER_HANDSHAKE: &[u8] = server_fixture!("tightvnc-viewer-3.8-vncauth.server-handshake.bin");
/// TigerVNC sending ServerCutText "from-x\n\u{e9}", then three Bells.
const CLIPBOARD_AND_BELLS: &[u8] =
    client_fixture!("tigervnc-3.8-none-cut-text-and-bell.server.bin");
/// The Target's bytes after its handshake: one ServerCutText of 16 bytes, then three Bells.
const TARGET_MESSAGES: usize = 54;

type ViewerRule = Box<dyn FnMut(&ClientMessage) -> Verdict<ClientMessage>>;
type TargetRule = Box<dyn FnMut(&TargetMessage) -> Verdict<TargetMessage>>;
type UpdateRule = Box<dyn FnMut(&Framed) -> UpdateVerdict>;

/// A Tap that applies the rules a test gives it and remembers the events it was told.
#[derive(Default)]
struct Script {
    viewer: Option<ViewerRule>,
    target: Option<TargetRule>,
    update: Option<UpdateRule>,
    updates_seen: usize,
    /// The bytes the Proxy reported forwarding, one call each.
    to_viewer_seen: Vec<Vec<u8>>,
    to_target_seen: Vec<Vec<u8>>,
    /// The desktops `relay_started` was given, and how many messages were reported before.
    started: Vec<(ServerInit, usize)>,
}

impl Tap for Script {
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        self.viewer
            .as_mut()
            .map_or(Verdict::Forward, |rule| rule(message))
    }

    fn target_message(&mut self, message: &TargetMessage) -> Verdict<TargetMessage> {
        self.target
            .as_mut()
            .map_or(Verdict::Forward, |rule| rule(message))
    }

    fn target_update(&mut self, piece: &Framed) -> UpdateVerdict {
        self.updates_seen += 1;
        self.update
            .as_mut()
            .map_or(UpdateVerdict::Forward, |rule| rule(piece))
    }

    fn relay_started(&mut self, desktop: &ServerInit) {
        let reported = self.to_viewer_seen.len() + self.to_target_seen.len();
        self.started.push((desktop.clone(), reported));
    }

    fn forwarded_to_viewer(&mut self, bytes: &[u8]) {
        self.to_viewer_seen.push(bytes.to_vec());
    }

    fn forwarded_to_target(&mut self, bytes: &[u8]) {
        self.to_target_seen.push(bytes.to_vec());
    }
}

/// A Proxy run with `tap`, the TightVNC Viewer's recording on the viewer leg and `target` on
/// the target leg.
struct Run {
    proxy: ProxySession<Script>,
    to_viewer: Vec<u8>,
    to_target: Vec<u8>,
    events: Vec<Event>,
    error: Option<Error>,
}

impl Run {
    fn drain(&mut self) {
        while let Some(bytes) = self.proxy.poll_viewer() {
            self.to_viewer.extend(bytes);
        }
        while let Some(bytes) = self.proxy.poll_target() {
            self.to_target.extend(bytes);
        }
        while let Some(event) = self.proxy.poll_event() {
            if let Event::Authenticate {
                challenge,
                response,
            } = &event
            {
                assert!(vnc_auth::verify(b"justvnc", challenge, response));
                self.proxy.accept();
            }
            if event == Event::TargetNeeded {
                self.proxy.connect_target(None);
            }
            self.events.push(event);
        }
    }

    fn new(tap: Script, viewer: &[u8], target: &[u8]) -> Self {
        let challenge = VIEWER_HANDSHAKE[14..30].try_into().unwrap();
        let config = ProxyConfig {
            security: vec![SecurityType::VncAuth],
            ..ProxyConfig::default()
        };
        let mut run = Self {
            proxy: ProxySession::with_challenge(config, challenge).with_tap(tap),
            to_viewer: Vec::new(),
            to_target: Vec::new(),
            events: Vec::new(),
            error: None,
        };
        run.error = run.proxy.receive_viewer(viewer).err();
        run.drain();
        run.error = run.error.take().or(run.proxy.receive_target(target).err());
        run.drain();
        run
    }

    /// What reached the Viewer after the Target's ServerInit.
    fn viewer_messages(&self) -> &[u8] {
        let init = 12 + 2 + 16 + 4;
        let name = u32::from_be_bytes(self.to_viewer[init + 20..init + 24].try_into().unwrap());
        &self.to_viewer[init + 24 + name as usize..]
    }

    /// The messages that reached the Target after its handshake, framed.
    fn target_messages(&self) -> Vec<ClientMessage> {
        let mut reader = ClientMessageReader::new(Limits::default());
        let mut at = 14;
        let mut out = Vec::new();
        while let Some((message, used)) = reader.next(&self.to_target[at..]).unwrap() {
            out.push(message);
            at += used;
        }
        assert_eq!(at, self.to_target.len());
        out
    }
}

fn is_cut_text(m: &ClientMessage) -> bool {
    matches!(m, ClientMessage::CutText(_))
}

fn is_key(m: &ClientMessage) -> bool {
    matches!(m, ClientMessage::KeyEvent { .. })
}

#[test]
fn forwards_everything_by_default() {
    let run = Run::new(Script::default(), VIEWER, CLIPBOARD_AND_BELLS);

    assert_eq!(
        run.viewer_messages(),
        &CLIPBOARD_AND_BELLS[TARGET_MESSAGES..]
    );
    assert_eq!(
        run.target_messages()
            .iter()
            .filter(|m| is_cut_text(m))
            .count(),
        1
    );
    assert_eq!(
        run.target_messages().iter().filter(|m| is_key(m)).count(),
        28
    );
    assert_eq!(run.error, None);
}

#[test]
fn blocks_the_clipboard_both_ways() {
    let tap = Script {
        viewer: Some(Box::new(|m| {
            if is_cut_text(m) {
                Verdict::Drop
            } else {
                Verdict::Forward
            }
        })),
        target: Some(Box::new(|m| match m {
            TargetMessage::CutText(_) => Verdict::Drop,
            TargetMessage::Bell => Verdict::Forward,
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    // The Viewer gets the three Bells and not the Target's clipboard text.
    assert_eq!(run.viewer_messages(), [2, 2, 2]);
    // The Target gets every Viewer message but the clipboard text.
    let messages = run.target_messages();
    assert!(!messages.iter().any(is_cut_text));
    assert_eq!(messages.iter().filter(|m| is_key(m)).count(), 28);
    assert_eq!(run.error, None);
}

#[test]
fn replaces_messages_both_ways() {
    let tap = Script {
        viewer: Some(Box::new(|m| match m {
            ClientMessage::CutText(_) => Verdict::Replace(ClientMessage::CutText(
                CutText::from_latin1(b"[blocked]".to_vec()),
            )),
            _ => Verdict::Forward,
        })),
        target: Some(Box::new(|m| match m {
            TargetMessage::CutText(_) => Verdict::Replace(TargetMessage::Bell),
            TargetMessage::Bell => Verdict::Replace(TargetMessage::CutText(CutText::from_latin1(
                b"ding".to_vec(),
            ))),
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    let ding =
        justvnc_proto::server_message::server_cut_text(&CutText::from_latin1(b"ding".to_vec()));
    assert_eq!(
        run.viewer_messages(),
        [&[2][..], &ding, &ding, &ding].concat()
    );
    let cut_texts: Vec<_> = run
        .target_messages()
        .into_iter()
        .filter(is_cut_text)
        .collect();
    assert_eq!(
        cut_texts,
        [ClientMessage::CutText(CutText::from_latin1(
            b"[blocked]".to_vec()
        ))]
    );
}

#[test]
fn ends_the_session_from_the_viewer_side() {
    let tap = Script {
        viewer: Some(Box::new(|m| {
            if is_key(m) {
                Verdict::End("no typing".into())
            } else {
                Verdict::Forward
            }
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    assert_eq!(run.error, Some(Error::Ended("no typing".into())));
    assert!(
        !run.target_messages().iter().any(is_key),
        "the key never reached the Target"
    );
    assert_eq!(
        run.events.last(),
        Some(&Event::Closed {
            reason: CloseReason::Ended("no typing".into())
        })
    );
}

#[test]
fn ends_the_session_from_the_target_side() {
    let tap = Script {
        target: Some(Box::new(|m| match m {
            TargetMessage::Bell => Verdict::End("no bells".into()),
            TargetMessage::CutText(_) => Verdict::Forward,
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    assert_eq!(run.error, Some(Error::Ended("no bells".into())));
    // The clipboard text came before the first Bell and went through; no Bell did.
    assert_eq!(
        run.viewer_messages(),
        &CLIPBOARD_AND_BELLS[TARGET_MESSAGES..TARGET_MESSAGES + 16]
    );
}

#[test]
fn shows_every_piece_of_an_update_and_can_end_on_one() {
    // TigerVNC's updates of a window moving: Raw rectangles and CopyRects.
    let target = client_fixture!("tigervnc-3.8-none-copyrect.server.bin");
    let watched = Run::new(Script::default(), VIEWER, target);
    let all = watched.proxy.tap().updates_seen;

    let tap = Script {
        update: Some(Box::new(|piece| match piece {
            Framed::Copy { .. } => UpdateVerdict::End("no copying".into()),
            _ => UpdateVerdict::Forward,
        })),
        ..Script::default()
    };
    let ended = Run::new(tap, VIEWER, target);

    assert!(all > 10, "{all} pieces");
    assert_eq!(ended.error, Some(Error::Ended("no copying".into())));
    assert!(ended.proxy.tap().updates_seen < all);
    assert!(ended.viewer_messages().len() < watched.viewer_messages().len());
}

#[test]
fn narrows_a_replacement_set_encodings_and_frames_by_it() {
    // The Tap takes LastRect out of the Viewer's SetEncodings, and puts Hextile in.
    let tap = Script {
        viewer: Some(Box::new(|m| match m {
            ClientMessage::SetEncodings(_) => {
                Verdict::Replace(ClientMessage::SetEncodings(vec![7, 5, 16, 0, -223, -24]))
            }
            _ => Verdict::Forward,
        })),
        ..Script::default()
    };
    // TigerVNC's Tight updates end at a LastRect, which a Viewer without it cannot frame.
    let target = client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin");

    let run = Run::new(tap, VIEWER, target);

    assert_eq!(
        run.target_messages()[0],
        ClientMessage::SetEncodings(vec![7, 16, 0, -223, -24])
    );
    assert!(
        matches!(run.error, Some(Error::Target(_))),
        "{:?}",
        run.error
    );
}

#[test]
fn checks_a_replacement_pixel_format_as_if_the_viewer_sent_it() {
    let colour_mapped = PixelFormat {
        bits_per_pixel: 8,
        depth: 8,
        big_endian: false,
        true_colour: false,
        red_max: 0,
        green_max: 0,
        blue_max: 0,
        red_shift: 0,
        green_shift: 0,
        blue_shift: 0,
    };
    let tap = Script {
        viewer: Some(Box::new(move |m| match m {
            ClientMessage::SetEncodings(_) => {
                Verdict::Replace(ClientMessage::SetPixelFormat(colour_mapped))
            }
            _ => Verdict::Forward,
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    assert_eq!(
        run.error,
        Some(Error::UnsupportedPixelFormat(colour_mapped))
    );
    let sent = client_message::set_pixel_format(&colour_mapped);
    assert!(!run.to_target.windows(sent.len()).any(|w| w == sent));
}

#[test]
fn reports_the_session_from_authentication_to_close() {
    let mut run = Run::new(Script::default(), VIEWER, CLIPBOARD_AND_BELLS);
    run.proxy.viewer_closed();
    run.proxy.target_closed();
    run.drain();

    let names: Vec<&str> = run
        .events
        .iter()
        .map(|e| match e {
            Event::Authenticate { .. } => "authenticate",
            Event::ViewerAuthenticated => "viewer authenticated",
            Event::TargetNeeded => "target needed",
            Event::TargetConnected { .. } => "target connected",
            Event::Closed { .. } => "closed",
        })
        .collect();
    assert_eq!(
        names,
        [
            "authenticate",
            "viewer authenticated",
            "target needed",
            "target connected",
            "closed"
        ]
    );
    assert_eq!(
        run.events.last(),
        Some(&Event::Closed {
            reason: CloseReason::ViewerClosed
        }),
        "closed once, by the first report"
    );
}

#[test]
fn reports_a_refused_viewer_as_closed_with_the_reason() {
    let challenge = VIEWER_HANDSHAKE[14..30].try_into().unwrap();
    let config = ProxyConfig {
        security: vec![SecurityType::VncAuth],
        ..ProxyConfig::default()
    };
    let mut proxy = ProxySession::with_challenge(config, challenge);
    proxy.receive_viewer(VIEWER).unwrap();
    proxy.reject("Authentication failure");

    let events: Vec<Event> = std::iter::from_fn(|| proxy.poll_event()).collect();
    assert_eq!(
        events.last(),
        Some(&Event::Closed {
            reason: CloseReason::Refused("Authentication failure".into())
        })
    );
}

#[test]
fn reports_a_target_that_breaks_the_protocol_as_closed_with_the_error() {
    let mut target = CLIPBOARD_AND_BELLS.to_vec();
    target.push(99);

    let run = Run::new(Script::default(), VIEWER, &target);

    let failed =
        Error::Target(justvnc_proto::MessageError::UnsupportedMessage { message_type: 99 });
    assert_eq!(run.error, Some(failed.clone()));
    assert_eq!(
        run.events.last(),
        Some(&Event::Closed {
            reason: CloseReason::Failed(failed)
        })
    );
    assert_eq!(
        run.events
            .iter()
            .filter(|e| matches!(e, Event::Closed { .. }))
            .count(),
        1
    );
}

/// Checks that the Tap was told exactly the bytes that crossed after the handshake, each call
/// one whole message.
fn assert_forwarded_matches(run: &Run) {
    let tap = run.proxy.tap();
    assert_eq!(tap.to_viewer_seen.concat(), run.viewer_messages());
    assert_eq!(tap.to_target_seen.concat(), run.to_target[14..]);
    for bytes in &tap.to_target_seen {
        let mut reader = ClientMessageReader::new(Limits::default());
        let framed = reader.next(bytes).unwrap();
        assert_eq!(framed.map(|(_, used)| used), Some(bytes.len()), "{bytes:?}");
    }
}

#[test]
fn reports_each_forwarded_message_after_the_handshake() {
    let run = Run::new(Script::default(), VIEWER, CLIPBOARD_AND_BELLS);

    assert_forwarded_matches(&run);
    // One ServerCutText and three Bells, one call each.
    assert_eq!(run.proxy.tap().to_viewer_seen.len(), 4);
    assert_eq!(run.error, None);
}

#[test]
fn reports_only_what_crossed_when_the_tap_drops_or_replaces() {
    let tap = Script {
        viewer: Some(Box::new(|m| {
            if is_cut_text(m) {
                Verdict::Drop
            } else {
                Verdict::Forward
            }
        })),
        target: Some(Box::new(|m| match m {
            TargetMessage::CutText(_) => Verdict::Drop,
            TargetMessage::Bell => {
                Verdict::Replace(TargetMessage::CutText(CutText::from_text("ding").unwrap()))
            }
        })),
        ..Script::default()
    };

    let run = Run::new(tap, VIEWER, CLIPBOARD_AND_BELLS);

    assert_forwarded_matches(&run);
    assert!(
        run.proxy
            .tap()
            .to_viewer_seen
            .iter()
            .all(|bytes| bytes[0] == 3),
        "only the replacement clipboard text crossed to the Viewer"
    );
}

#[test]
fn reports_each_piece_of_the_targets_updates() {
    // TigerVNC's updates: Raw rectangles, CopyRects and their headers.
    let updates = client_fixture!("tigervnc-3.8-none-copyrect.server.bin");

    let run = Run::new(Script::default(), VIEWER, updates);

    assert_forwarded_matches(&run);
    let tap = run.proxy.tap();
    assert!(tap.to_viewer_seen.iter().all(|bytes| !bytes.is_empty()));
    assert!(tap.to_viewer_seen.len() > 3);
}

#[test]
fn says_when_relaying_starts_before_forwarding_anything() {
    let run = Run::new(Script::default(), VIEWER, CLIPBOARD_AND_BELLS);

    let tap = run.proxy.tap();
    let init = 12 + 2 + 4;
    let name = u32::from_be_bytes(
        CLIPBOARD_AND_BELLS[init + 20..init + 24]
            .try_into()
            .unwrap(),
    );
    let target_init = &CLIPBOARD_AND_BELLS[init..init + 24 + name as usize];
    assert_eq!(tap.started.len(), 1);
    assert_eq!(tap.started[0].0.to_bytes(), target_init);
    assert_eq!(tap.started[0].1, 0, "nothing was reported before");
}
