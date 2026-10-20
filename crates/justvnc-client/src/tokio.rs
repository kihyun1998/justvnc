//! Drives a [`ClientSession`] over a Tokio stream.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{ClientConfig, ClientSession, Event, HandshakeError, ServerInit};

/// A client role connected to a server whose handshake has completed.
#[derive(Debug)]
pub struct Connection<S> {
    stream: S,
    session: ClientSession,
    desktop: ServerInit,
}

impl<S> Connection<S> {
    /// The desktop the server described in ServerInit.
    pub fn desktop(&self) -> &ServerInit {
        &self.desktop
    }

    /// The stream and the session, to keep driving the connection.
    pub fn into_parts(self) -> (S, ClientSession) {
        (self.stream, self.session)
    }
}

/// Why connecting failed.
#[derive(Debug)]
pub enum ConnectError {
    Handshake(HandshakeError),
    Io(std::io::Error),
    /// The server closed the connection before the handshake completed.
    Closed,
}

/// Runs the client handshake over `stream`.
pub async fn connect<S>(stream: S, config: ClientConfig) -> Result<Connection<S>, ConnectError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut stream = stream;
    let mut session = ClientSession::new(config);
    let mut buf = vec![0u8; 4096];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(ConnectError::Closed);
        }
        session.receive(&buf[..n])?;
        while let Some(bytes) = session.poll_transmit() {
            stream.write_all(&bytes).await?;
        }
        if let Some(Event::Connected(desktop)) = session.poll_event() {
            stream.flush().await?;
            return Ok(Connection {
                stream,
                session,
                desktop,
            });
        }
    }
}

impl From<std::io::Error> for ConnectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<HandshakeError> for ConnectError {
    fn from(e: HandshakeError) -> Self {
        Self::Handshake(e)
    }
}
