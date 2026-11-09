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
        info!(%bind, "桌面观看信令已监听 (ws://0.0.0.0:{})", config.port);

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
        let stop_web = stop_rx.clone();
        web_portal::start(cfg_web, stop_web)
            .await
            .context("start web viewer portal (HTTPS)")?;
        info!(web_port = config.web_port, "网页观看门户 (HTTPS/WSS) 已就绪");

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

fn local_network_denied(err: &impl std::fmt::Display) -> bool {
    let raw = err.to_string();
    raw.contains("Socket is not connected") || raw.contains("os error 57")
}

async fn handle_desktop_viewer(stream: TcpStream, config: HostSessionConfig) -> Result<()> {
    let ws = accept_async(stream).await.map_err(|err| {
        if local_network_denied(&err) {
            anyhow::anyhow!(
                "系统拒绝了局域网连接。请在系统设置 → 隐私与安全性 → 本地网络中允许 LAN-Screenshare，然后重新点「启动分享」。"
            )
        } else {
            anyhow::anyhow!("websocket handshake: {err}")
        }
    })?;
    let (mut write, mut read) = ws.split();
    let (tx_in, rx_in) = mpsc::unbounded_channel::<String>();
    let (tx_out, mut rx_out) = mpsc::unbounded_channel::<String>();

    let write_task = tokio::spawn(async move {
        while let Some(text) = rx_out.recv().await {
            if write.send(Message::Text(text)).await.is_err() {
                return;
            }
        }
        let _ = write.send(Message::Close(None)).await;
    });

    let read_task = tokio::spawn(async move {
        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if tx_in.send(text).is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) => {
                    info!("观看端关闭了信令");
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    if local_network_denied(&err) {
                        warn!(
                            "读取观看端信令失败：系统拒绝了局域网连接。请允许「本地网络」后重新启动分享。"
                        );
                    } else {
                        warn!("读取观看端信令失败：{err:#}");
                    }
                    break;
                }
                None => break,
            }
        }
    });

    let result = run_host_viewer_session(config, rx_in, tx_out).await;
    match tokio::time::timeout(std::time::Duration::from_secs(2), write_task).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => warn!("signaling writer stopped: {err}"),
        Err(_) => warn!("signaling writer did not finish before timeout"),
    }
    read_task.abort();
    result
}
