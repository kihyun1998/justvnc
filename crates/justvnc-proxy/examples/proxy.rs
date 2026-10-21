//! A Proxy in front of one Target, for any number of Viewers.
//!
//! Usage: cargo run -p justvnc-proxy --features tokio --example proxy -- [LISTEN] [TARGET]
//! (defaults 127.0.0.1:15952 and 127.0.0.1:15900). `JUSTVNC_TOKEN` is the password Viewers
//! give, with VNC authentication; without it Viewers get security type None.
//! `JUSTVNC_TARGET_PASSWORD` is the Target's own password.

use justvnc_proxy::tokio::serve;
use justvnc_proxy::{Password, ProxyConfig, SecurityType, verify};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let listen = args.next().unwrap_or_else(|| "127.0.0.1:15952".into());
    let target = args.next().unwrap_or_else(|| "127.0.0.1:15900".into());
    let token = std::env::var("JUSTVNC_TOKEN").ok();
    let target_password = std::env::var("JUSTVNC_TARGET_PASSWORD").ok();
    let listener = TcpListener::bind(&listen).await?;
    println!("listening on {listen}, relaying to {target}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let (target, token, target_password) =
            (target.clone(), token.clone(), target_password.clone());
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
            match serve(stream, config, authenticate, connect).await {
                Ok(ended) => println!("{peer} {ended:?}"),
                Err(e) => println!("{peer} ended: {e:?}"),
            }
        });
    }
}
