//! Drives a [`ClientSession`] over a Tokio stream.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{ClientConfig, ClientSession, Error, Event, PixelFormat, Rect, ServerInit};

const READ_BUFFER: usize = 64 * 1024;

/// A client role connected to a server whose handshake has completed.
#[derive(Debug)]
pub struct Connection<S> {
    stream: S,
    session: ClientSession,
    desktop: ServerInit,
    buf: Vec<u8>,
}

/// Why connecting or driving the connection failed.
#[derive(Debug)]
pub enum ConnectError {
    Protocol(Error),
    Io(std::io::Error),
    /// The server closed the connection.
    Closed,
}

/// Runs the client handshake over `stream`.
pub async fn connect<S>(stream: S, config: ClientConfig) -> Result<Connection<S>, ConnectError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut stream = stream;
    let mut session = ClientSession::new(config);
    let mut buf = vec![0u8; READ_BUFFER];
    loop {
        if let Event::Connected(desktop) = next_event(&mut stream, &mut session, &mut buf).await? {
            return Ok(Connection {
                stream,
                session,
                desktop,
                buf,
            });
        }
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Asks the server for the contents of `area`: everything, or only what changed.
    pub async fn request_update(
        &mut self,
        incremental: bool,
        area: Rect,
    ) -> Result<(), ConnectError> {
        self.session.request_update(incremental, area);
        flush(&mut self.stream, &mut self.session).await
    }

    /// Waits for the session's next event, reading from the server as needed.
    pub async fn next_event(&mut self) -> Result<Event, ConnectError> {
        next_event(&mut self.stream, &mut self.session, &mut self.buf).await
    }
}

impl<S> Connection<S> {
    /// The desktop the server described in ServerInit.
    pub fn desktop(&self) -> &ServerInit {
        &self.desktop
    }

    /// The pixel format rectangles arrive in.
    pub fn pixel_format(&self) -> PixelFormat {
        self.session
            .pixel_format()
            .expect("a Connection exists only after the handshake")
    }

    /// The stream and the session, to keep driving the connection.
    pub fn into_parts(self) -> (S, ClientSession) {
        (self.stream, self.session)
    }
}

async fn next_event<S>(
    stream: &mut S,
    session: &mut ClientSession,
    buf: &mut [u8],
) -> Result<Event, ConnectError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        if let Some(event) = session.poll_event() {
            return Ok(event);
        }
        let n = stream.read(buf).await?;
        if n == 0 {
            return Err(ConnectError::Closed);
        }
        session.receive(&buf[..n]).map_err(ConnectError::Protocol)?;
        flush(stream, session).await?;
    }
}

async fn flush<S>(stream: &mut S, session: &mut ClientSession) -> Result<(), ConnectError>
where
    S: AsyncWrite + Unpin,
{
    while let Some(bytes) = session.poll_transmit() {
        stream.write_all(&bytes).await?;
    }
    stream.flush().await?;
    Ok(())
}

impl From<std::io::Error> for ConnectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
