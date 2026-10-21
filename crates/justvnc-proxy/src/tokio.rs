//! Drives a [`ProxySession`] over two Tokio streams.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{CHALLENGE_LEN, Error, Event, Password, ProxyConfig, ProxySession, Tap};

const READ_BUFFER: usize = 64 * 1024;

/// How a relayed session ended normally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    ViewerClosed,
    TargetClosed,
    /// The Tap ended it, with this reason.
    ByTap(String),
}

/// Why the Proxy stopped before either side closed.
#[derive(Debug)]
pub enum ProxyError {
    Protocol(Error),
    Io(std::io::Error),
    /// The Viewer closed the connection before the relay started.
    ViewerClosed,
}

/// Serves one Viewer on `viewer`, with `tap` looking at every relayed message and told every
/// [`Event`]. `authenticate` decides VNC authentication from the challenge and the Viewer's
/// response; a Viewer it rejects is told "Authentication failure". Once the Viewer is through,
/// `connect` connects to the Target and returns the stream and the Target's password, or the
/// reason the Viewer is refused. Then messages are relayed until either side closes.
pub async fn serve<V, T, P, A, AF, C, CF>(
    viewer: V,
    config: ProxyConfig,
    tap: P,
    mut authenticate: A,
    connect: C,
) -> Result<Ended, ProxyError>
where
    V: AsyncRead + AsyncWrite + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
    P: Tap,
    A: FnMut([u8; CHALLENGE_LEN], [u8; CHALLENGE_LEN]) -> AF,
    AF: Future<Output = bool>,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<(T, Option<Password>), String>>,
{
    let mut viewer = viewer;
    let mut session = ProxySession::new(config).with_tap(tap);
    let mut buf = vec![0u8; READ_BUFFER];
    flush(&mut viewer, &mut session, None::<&mut T>).await?;
    // The viewer leg, until the Viewer is through security.
    'viewer: loop {
        while let Some(event) = next_event(&mut session) {
            match event {
                Event::Authenticate {
                    challenge,
                    response,
                } => {
                    if authenticate(challenge, response).await {
                        session.accept();
                    } else {
                        session.reject("Authentication failure");
                        flush(&mut viewer, &mut session, None::<&mut T>).await?;
                        return Err(ended(&mut session));
                    }
                }
                Event::TargetNeeded => break 'viewer,
                _ => {}
            }
        }
        let n = viewer.read(&mut buf).await?;
        if n == 0 {
            session.viewer_closed();
            while next_event(&mut session).is_some() {}
            return Err(ProxyError::ViewerClosed);
        }
        session
            .receive_viewer(&buf[..n])
            .map_err(ProxyError::Protocol)?;
        flush(&mut viewer, &mut session, None::<&mut T>).await?;
    }
    let mut target = match connect().await {
        Ok((target, password)) => {
            session.connect_target(password);
            target
        }
        Err(reason) => {
            session.refuse(&reason);
            flush(&mut viewer, &mut session, None::<&mut T>).await?;
            return Err(ended(&mut session));
        }
    };
    // Both legs. Reads are cancel-safe, so whichever completes first is handled and the other
    // is started again; writes happen outside the select.
    let mut target_buf = vec![0u8; READ_BUFFER];
    loop {
        let result = tokio::select! {
            n = viewer.read(&mut buf) => match n? {
                0 => {
                    session.viewer_closed();
                    while next_event(&mut session).is_some() {}
                    return Ok(Ended::ViewerClosed);
                }
                n => session.receive_viewer(&buf[..n]),
            },
            n = target.read(&mut target_buf) => match n? {
                0 => {
                    session.target_closed();
                    while next_event(&mut session).is_some() {}
                    return Ok(Ended::TargetClosed);
                }
                n => session.receive_target(&target_buf[..n]),
            },
        };
        flush(&mut viewer, &mut session, Some(&mut target)).await?;
        while next_event(&mut session).is_some() {}
        match result {
            Ok(()) => {}
            Err(Error::Ended(reason)) => return Ok(Ended::ByTap(reason)),
            Err(e) => return Err(ProxyError::Protocol(e)),
        }
    }
}

/// The session's next event, after the Tap has been told of it.
fn next_event<P: Tap>(session: &mut ProxySession<P>) -> Option<Event> {
    let event = session.poll_event()?;
    session.tap_mut().event(&event);
    Some(event)
}

/// The error a session that has ended returns, once the Tap has been told it closed.
fn ended<P: Tap>(session: &mut ProxySession<P>) -> ProxyError {
    while next_event(session).is_some() {}
    ProxyError::Protocol(
        session
            .receive_viewer(&[])
            .expect_err("a refused session has ended"),
    )
}

async fn flush<V, T, P: Tap>(
    viewer: &mut V,
    session: &mut ProxySession<P>,
    target: Option<&mut T>,
) -> Result<(), ProxyError>
where
    V: AsyncWrite + Unpin,
    T: AsyncWrite + Unpin,
{
    while let Some(bytes) = session.poll_viewer() {
        viewer.write_all(&bytes).await?;
    }
    viewer.flush().await?;
    if let Some(target) = target {
        while let Some(bytes) = session.poll_target() {
            target.write_all(&bytes).await?;
        }
        target.flush().await?;
    }
    Ok(())
}

impl From<std::io::Error> for ProxyError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
