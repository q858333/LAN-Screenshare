use crate::net;
use crate::session::run_host_viewer_session;
use crate::web_portal;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use lan_share_core::HostSessionConfig;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{info, warn};

#[derive(Clone)]
pub struct HostSessionHandle {
    stop: watch::Sender<bool>,
    addrs: Arc<Vec<String>>,
    port: u16,
    web_port: u16,
}

impl HostSessionHandle {
    pub fn lan_addresses(&self) -> Vec<String> {
        self.addrs.as_ref().clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn web_port(&self) -> u16 {
        self.web_port
    }

    pub fn web_viewer_urls(&self) -> Vec<String> {
        self.addrs
            .iter()
            .map(|ip| format!("https://{ip}:{}", self.web_port))
            .collect()
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

pub struct HostSession;

impl HostSession {
    pub async fn start(config: HostSessionConfig) -> Result<HostSessionHandle> {
        let bind = net::signaling_bind_addr(config.port);
        let listener = TcpListener::bind(&bind)
            .await
            .with_context(|| format!("bind signaling server on {bind}"))?;
        info!(%bind, "signaling server listening (desktop viewer)");

        let addrs = Arc::new(net::list_lan_ipv4());
        let (stop_tx, stop_rx) = watch::channel(false);

        let cfg_signaling = config.clone();
        let mut stop_signaling = stop_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_host_loop(listener, stop_signaling, cfg_signaling).await {
                warn!("signaling server ended: {e:#}");
            }
        });

        let cfg_web = config.clone();
        let mut stop_web = stop_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = web_portal::start(cfg_web, stop_web).await {
                warn!("web portal ended: {e:#}");
            }
        });

        Ok(HostSessionHandle {
            stop: stop_tx,
            addrs,
            port: config.port,
            web_port: config.web_port,
        })
    }
}

async fn run_host_loop(
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
    config: HostSessionConfig,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            accept = listener.accept() => {
                let (stream, peer) = accept.context("accept viewer")?;
                info!(%peer, "desktop viewer connected");
                let cfg = config.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_desktop_viewer(stream, cfg).await {
                        warn!(%peer, "desktop viewer session error: {e:#}");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_desktop_viewer(stream: TcpStream, config: HostSessionConfig) -> Result<()> {
    let ws = accept_async(stream).await.context("websocket handshake")?;
    let (mut write, mut read) = ws.split();
    let (tx_in, rx_in) = mpsc::unbounded_channel::<String>();
    let (tx_out, mut rx_out) = mpsc::unbounded_channel::<String>();

    let write_task = tokio::spawn(async move {
        while let Some(text) = rx_out.recv().await {
            if write
                .send(Message::Text(text))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(text) = msg else { continue };
            let _ = tx_in.send(text);
        }
    });

    let result = run_host_viewer_session(config, rx_in, tx_out).await;
    write_task.abort();
    read_task.abort();
    result
}
