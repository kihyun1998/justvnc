use justvnc_proto::CutText;
use justvnc_proto::client_message::ClientMessage;
use justvnc_proto::server_message::Framed;

use crate::Event;

/// What the Tap does with one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict<M> {
    /// Send it on as it came.
    Forward,
    /// Do not send it on.
    Drop,
    /// Send this in its place.
    Replace(M),
    /// End the session, for this reason.
    End(String),
}

/// What the Tap does with one piece of a FramebufferUpdate, which it can only watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateVerdict {
    Forward,
    End(String),
}

/// A Target message the Tap can drop or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetMessage {
    Bell,
    /// ServerCutText.
    CutText(CutText),
}

/// The host's view of every message the Proxy relays, and its say over each.
///
/// A Viewer message the Tap replaces is handled as if the Viewer had sent the replacement: a
/// SetEncodings is still narrowed, and a SetPixelFormat still checked.
pub trait Tap {
    /// A message from the Viewer, before it reaches the Target.
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        let _ = message;
        Verdict::Forward
    }

    /// A Bell or ServerCutText from the Target, before it reaches the Viewer.
    fn target_message(&mut self, message: &TargetMessage) -> Verdict<TargetMessage> {
        let _ = message;
        Verdict::Forward
    }

    /// One piece of a FramebufferUpdate from the Target: a rectangle, a copy, a new desktop
    /// size, or the update's end.
    fn target_update(&mut self, piece: &Framed) -> UpdateVerdict {
        let _ = piece;
        UpdateVerdict::Forward
    }

    /// An event of the session's life, as the tokio driver reports them. A host driving
    /// [`crate::ProxySession`] itself reads them with `poll_event` instead.
    fn event(&mut self, event: &Event) {
        let _ = event;
    }
}

/// A Tap that forwards everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct ForwardAll;

impl Tap for ForwardAll {}
