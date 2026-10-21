//! Drives a [`ProxySession`] over two Tokio streams.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{CHALLENGE_LEN, Error, Event, Password, ProxyConfig, ProxySession};

const READ_BUFFER: usize = 64 * 1024;

/// How a relayed session ended normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    ViewerClosed,
    TargetClosed,
}

/// Why the Proxy stopped before either side closed.
#[derive(Debug)]
pub enum ProxyError {
    Protocol(Error),
    Io(std::io::Error),
    /// The Viewer closed the connection before the relay started.
    ViewerClosed,
}

/// Serves one Viewer on `viewer`. `authenticate` decides VNC authentication from the challenge
/// and the Viewer's response; a Viewer it rejects is told "Authentication failure". Once the
/// Viewer is through, `connect` connects to the Target and returns the stream and the Target's
/// password, or the reason the Viewer is refused. Then messages are relayed until either side
/// closes.
pub async fn serve<V, T, A, AF, C, CF>(
    viewer: V,
    config: ProxyConfig,
    mut authenticate: A,
    connect: C,
) -> Result<Ended, ProxyError>
where
    V: AsyncRead + AsyncWrite + Unpin,
    T: AsyncRead + AsyncWrite + Unpin,
    A: FnMut([u8; CHALLENGE_LEN], [u8; CHALLENGE_LEN]) -> AF,
    AF: Future<Output = bool>,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<(T, Option<Password>), String>>,
{
    let mut viewer = viewer;
    let mut session = ProxySession::new(config);
    let mut buf = vec![0u8; READ_BUFFER];
    flush(&mut viewer, &mut session, None::<&mut T>).await?;
    // The viewer leg, until the Viewer is through security.
    loop {
        match session.poll_event() {
            Some(Event::Authenticate {
                challenge,
                response,
            }) => {
                if authenticate(challenge, response).await {
                    session.accept();
                } else {
                    session.reject("Authentication failure");
                    flush(&mut viewer, &mut session, None::<&mut T>).await?;
                    return Err(ended(&mut session));
                }
            }
            Some(Event::TargetNeeded) => break,
            Some(Event::Connected { .. }) => unreachable!("no Target yet"),
            None => {
                let n = viewer.read(&mut buf).await?;
                if n == 0 {
                    return Err(ProxyError::ViewerClosed);
                }
                session
                    .receive_viewer(&buf[..n])
                    .map_err(ProxyError::Protocol)?;
                flush(&mut viewer, &mut session, None::<&mut T>).await?;
            }
        }
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
                0 => return Ok(Ended::ViewerClosed),
                n => session.receive_viewer(&buf[..n]),
            },
            n = target.read(&mut target_buf) => match n? {
                0 => return Ok(Ended::TargetClosed),
                n => session.receive_target(&target_buf[..n]),
            },
        };
        flush(&mut viewer, &mut session, Some(&mut target)).await?;
        result.map_err(ProxyError::Protocol)?;
        while session.poll_event().is_some() {}
    }
}

/// The error a session that has ended returns.
fn ended(session: &mut ProxySession) -> ProxyError {
    ProxyError::Protocol(
        session
            .receive_viewer(&[])
            .expect_err("a refused session has ended"),
    )
}

async fn flush<V, T>(
    viewer: &mut V,
    session: &mut ProxySession,
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
