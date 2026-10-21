//! Drives a [`ServerSession`] over a Tokio stream.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{CHALLENGE_LEN, Error, Event, Framebuffer, Rect, ServerConfig, ServerSession};

const READ_BUFFER: usize = 64 * 1024;

/// A server role whose handshake with a Viewer has completed.
#[derive(Debug)]
pub struct Connection<S> {
    link: Link<S>,
    shared: bool,
}

/// Why accepting or serving the Viewer failed.
#[derive(Debug)]
pub enum ServeError {
    Protocol(Error),
    Io(std::io::Error),
    /// The Viewer closed the connection.
    Closed,
}

/// The stream and the session, with the bytes taken from the session and not yet written.
#[derive(Debug)]
struct Link<S> {
    stream: S,
    session: ServerSession,
    buf: Vec<u8>,
    outgoing: Vec<u8>,
    written: usize,
}

/// Runs the server handshake over `stream`. `authenticate` decides VNC authentication from
/// the challenge and the Viewer's response; a Viewer it rejects is told "Authentication
/// failure", and the error is returned once that has been sent.
pub async fn accept<S, F, A, AF>(
    stream: S,
    config: ServerConfig,
    framebuffer: &mut F,
    mut authenticate: A,
) -> Result<Connection<S>, ServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: Framebuffer,
    A: FnMut([u8; CHALLENGE_LEN], [u8; CHALLENGE_LEN]) -> AF,
    AF: Future<Output = bool>,
{
    let mut link = Link {
        stream,
        session: ServerSession::new(config),
        buf: vec![0u8; READ_BUFFER],
        outgoing: Vec::new(),
        written: 0,
    };
    link.flush().await?;
    loop {
        while let Some(event) = link.session.poll_event() {
            match event {
                Event::Authenticate {
                    challenge,
                    response,
                } => {
                    if authenticate(challenge, response).await {
                        let accepted = link.session.accept(framebuffer);
                        link.flush().await?;
                        accepted.map_err(ServeError::Protocol)?;
                    } else {
                        link.session.reject("Authentication failure");
                        link.flush().await?;
                        let rejected = link.session.receive(&[], framebuffer);
                        return Err(ServeError::Protocol(
                            rejected.expect_err("a rejected session has ended"),
                        ));
                    }
                }
                Event::Connected { shared } => return Ok(Connection { link, shared }),
            }
        }
        link.read(framebuffer).await?;
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Reads what the Viewer sends next and handles it: input goes to `framebuffer`, and
    /// update requests are answered from it. Cancel-safe, like [`Connection::damage`]: bytes a
    /// cancelled call did not write are written by the next call.
    pub async fn serve<F: Framebuffer>(&mut self, framebuffer: &mut F) -> Result<(), ServeError> {
        self.link.flush().await?;
        self.link.read(framebuffer).await
    }

    /// Tells the session that the pixels in `rect` changed, and sends what a waiting request
    /// asked for.
    pub async fn damage<F: Framebuffer>(
        &mut self,
        rect: Rect,
        framebuffer: &F,
    ) -> Result<(), ServeError> {
        self.link.session.damage(rect, framebuffer);
        self.link.flush().await
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Tells the session that the pixels now in `rect` were moved there from (`src_x`,
    /// `src_y`); see [`ServerSession::copy`].
    pub async fn copy<F: Framebuffer>(
        &mut self,
        rect: Rect,
        src_x: u16,
        src_y: u16,
        framebuffer: &F,
    ) -> Result<(), ServeError> {
        self.link.session.copy(rect, src_x, src_y, framebuffer);
        self.link.flush().await
    }

    /// Tells the session that the framebuffer may have changed size; see
    /// [`ServerSession::resize`].
    pub async fn resize<F: Framebuffer>(&mut self, framebuffer: &F) -> Result<(), ServeError> {
        let resized = self.link.session.resize(framebuffer);
        self.link.flush().await?;
        resized.map_err(ServeError::Protocol)
    }
}

impl<S> Connection<S> {
    /// The Viewer's ClientInit shared-flag.
    pub fn shared(&self) -> bool {
        self.shared
    }

    /// The stream and the session, to keep driving the connection, and bytes taken from the
    /// session that were not written yet.
    pub fn into_parts(self) -> (S, ServerSession, Vec<u8>) {
        let link = self.link;
        let unwritten = link.outgoing[link.written..].to_vec();
        (link.stream, link.session, unwritten)
    }
}

impl<S> Link<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    async fn read<F: Framebuffer>(&mut self, framebuffer: &mut F) -> Result<(), ServeError> {
        let n = self.stream.read(&mut self.buf).await?;
        if n == 0 {
            return Err(ServeError::Closed);
        }
        let result = self.session.receive(&self.buf[..n], framebuffer);
        self.flush().await?;
        result.map_err(ServeError::Protocol)
    }

    async fn flush(&mut self) -> Result<(), ServeError> {
        while let Some(bytes) = self.session.poll_transmit() {
            self.outgoing.extend_from_slice(&bytes);
        }
        while self.written < self.outgoing.len() {
            let n = self.stream.write(&self.outgoing[self.written..]).await?;
            if n == 0 {
                return Err(ServeError::Io(std::io::ErrorKind::WriteZero.into()));
            }
            self.written += n;
        }
        self.outgoing.clear();
        self.written = 0;
        self.stream.flush().await?;
        Ok(())
    }
}

impl From<std::io::Error> for ServeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::Closed => f.write_str("the Viewer closed the connection"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(e) => e.source(),
            Self::Io(e) => e.source(),
            Self::Closed => None,
        }
    }
}
