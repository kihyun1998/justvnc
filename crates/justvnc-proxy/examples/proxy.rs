//! A Proxy in front of one Target, for any number of Viewers.
//!
//! Usage: cargo run -p justvnc-proxy --features tokio --example proxy -- [LISTEN] [TARGET]
//! (defaults 127.0.0.1:15952 and 127.0.0.1:15900). `JUSTVNC_TOKEN` is the password Viewers
//! give, with VNC authentication; without it Viewers get security type None.
//! `JUSTVNC_TARGET_PASSWORD` is the Target's own password. `JUSTVNC_BLOCK_CLIPBOARD` set to
//! anything drops clipboard text both ways. `JUSTVNC_RECORD` set to a directory records each
//! session there, as `<time>-<port>.jvrec`; play one back with justvnc-record's `replay`
//! example. Each session's events are printed.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use justvnc_proxy::tokio::serve;
use justvnc_proxy::{
    ClientMessage, Event, Password, ProxyConfig, SecurityType, ServerInit, Tap, TargetMessage,
    Verdict, verify,
};
use justvnc_record::{Direction, RecordingWriter};
use tokio::net::{TcpListener, TcpStream};

/// Prints the session's events, drops clipboard text and records the session if asked to.
struct Log {
    peer: std::net::SocketAddr,
    block_clipboard: bool,
    record_in: Option<PathBuf>,
    recording: Option<(RecordingWriter<BufWriter<File>>, Instant)>,
}

impl Log {
    fn record(&mut self, direction: Direction, bytes: &[u8]) {
        if let Some((writer, started)) = &mut self.recording
            && let Err(e) = writer.write(direction, started.elapsed(), bytes)
        {
            println!("{} recording stopped: {e}", self.peer);
            self.recording = None;
        }
    }
}

impl Tap for Log {
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        match message {
            ClientMessage::CutText(text) if self.block_clipboard => {
                println!("{} clipboard to the Target dropped: {text}", self.peer);
                Verdict::Drop
            }
            _ => Verdict::Forward,
        }
    }

    fn target_message(&mut self, message: &TargetMessage) -> Verdict<TargetMessage> {
        match message {
            TargetMessage::CutText(text) if self.block_clipboard => {
                println!("{} clipboard to the Viewer dropped: {text}", self.peer);
                Verdict::Drop
            }
            _ => Verdict::Forward,
        }
    }

    fn relay_started(&mut self, desktop: &ServerInit) {
        let Some(dir) = &self.record_in else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let path = dir.join(format!("{}-{}.jvrec", now.as_secs(), self.peer.port()));
        match File::create(&path).and_then(|f| RecordingWriter::new(BufWriter::new(f), desktop)) {
            Ok(writer) => {
                println!("{} recording to {}", self.peer, path.display());
                self.recording = Some((writer, Instant::now()));
            }
            Err(e) => println!("{} cannot record to {}: {e}", self.peer, path.display()),
        }
    }

    fn forwarded_to_viewer(&mut self, bytes: &[u8]) {
        self.record(Direction::ToViewer, bytes);
    }

    fn forwarded_to_target(&mut self, bytes: &[u8]) {
        self.record(Direction::ToTarget, bytes);
    }

    fn event(&mut self, event: &Event) {
        if let Event::Closed { .. } = event
            && let Some((writer, _)) = &mut self.recording
        {
            let _ = writer.flush();
        }
        match event {
            Event::Authenticate { .. } => {}
            Event::TargetConnected { desktop } => println!(
                "{} target connected: {}x{} {:?}",
                self.peer,
                desktop.width,
                desktop.height,
                desktop.name.to_string_lossy()
            ),
            event => println!("{} {event:?}", self.peer),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let listen = args.next().unwrap_or_else(|| "127.0.0.1:15952".into());
    let target = args.next().unwrap_or_else(|| "127.0.0.1:15900".into());
    let token = std::env::var("JUSTVNC_TOKEN").ok();
    let target_password = std::env::var("JUSTVNC_TARGET_PASSWORD").ok();
    let block_clipboard = std::env::var_os("JUSTVNC_BLOCK_CLIPBOARD").is_some();
    let record_in = std::env::var_os("JUSTVNC_RECORD").map(PathBuf::from);
    let listener = TcpListener::bind(&listen).await?;
    println!("listening on {listen}, relaying to {target}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let (target, token, target_password, record_in) = (
            target.clone(),
            token.clone(),
            target_password.clone(),
            record_in.clone(),
        );
        tokio::spawn(async move {
            println!("{peer} connected");
            let config = ProxyConfig {
                security: vec![if token.is_some() {
                    SecurityType::VncAuth
                } else {
                    SecurityType::None
                }],
                ..ProxyConfig::default()
            };
            let authenticate = |challenge, response| {
                let token = token.clone().unwrap_or_default();
                async move { verify(token.as_bytes(), &challenge, &response) }
            };
            let connect = || async move {
                let stream = TcpStream::connect(&target)
                    .await
                    .map_err(|e| format!("Target unreachable: {e}"))?;
                println!("{peer} authenticated; connected to {target}");
                Ok((stream, target_password.map(Password::new)))
            };
            match serve(
                stream,
                config,
                Log {
                    peer,
                    block_clipboard,
                    record_in,
                    recording: None,
                },
                authenticate,
                connect,
            )
            .await
            {
                Ok(ended) => println!("{peer} {ended:?}"),
                Err(e) => println!("{peer} ended: {e:?}"),
            }
        });
    }
}
