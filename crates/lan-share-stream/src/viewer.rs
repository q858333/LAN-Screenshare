use crate::media::H264Decoder;
use crate::webrtc::{apply_remote_message, build_peer_connection, wire_ice_candidates};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use bytes::Bytes;
use jpeg_encoder::{ColorType, Encoder as JpegEncoder};
use webrtc::rtp::packetizer::Depacketizer;
use lan_share_core::{SignalingMessage, ViewerConnectConfig, ViewerFrame};
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp::codecs::h264::H264Packet;

#[derive(Clone)]
pub struct ViewerSessionHandle {
    inner: Arc<Mutex<ViewerState>>,
    stop: watch::Sender<bool>,
    frame_tx: watch::Sender<Option<ViewerFrame>>,
}

#[derive(Default)]
struct ViewerState {
    last_host_info: Option<String>,
    last_error: Option<String>,
    connected: bool,
}

impl ViewerSessionHandle {
    pub fn snapshot(&self) -> (bool, Option<String>, Option<String>) {
        if let Ok(guard) = self.inner.try_lock() {
            (
                guard.connected,
                guard.last_host_info.clone(),
                guard.last_error.clone(),
            )
        } else {
            (false, None, None)
        }
    }

    pub fn subscribe_frames(&self) -> watch::Receiver<Option<ViewerFrame>> {
        self.frame_tx.subscribe()
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

pub struct ViewerSession;

impl ViewerSession {
    pub async fn connect(config: ViewerConnectConfig) -> Result<ViewerSessionHandle> {
        let url = format!("ws://{}:{}/", config.host_addr.trim(), config.port);
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(ViewerState::default()));
        let (frame_tx, _) = watch::channel(None);

        let state_task = Arc::clone(&state);
        let frame_task = frame_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_viewer(&url, stop_rx, state_task, frame_task).await {
                warn!("viewer session ended: {e:#}");
            }
        });

        Ok(ViewerSessionHandle {
            inner: state,
            stop: stop_tx,
            frame_tx,
        })
    }
}

async fn run_viewer(
    url: &str,
    mut stop: watch::Receiver<bool>,
    state: Arc<Mutex<ViewerState>>,
    frame_tx: watch::Sender<Option<ViewerFrame>>,
) -> Result<()> {
    let (ws, _) = connect_async(url).await.with_context(|| format!("connect {url}"))?;
    info!(%url, "connected to host signaling");
    {
        let mut guard = state.lock().await;
        guard.connected = true;
    }

    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));

    let hello = SignalingMessage::Hello {
        role: lan_share_core::SessionRole::Viewer,
        name: "Viewer".into(),
    };
    {
        let mut w = write.lock().await;
        w.send(Message::Text(serde_json::to_string(&hello)?))
            .await?;
    }

    let pc = build_peer_connection().await?;
    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel();
    wire_ice_candidates(Arc::clone(&pc), sig_tx.clone());

    let frame_tx_track = frame_tx.clone();
    pc.on_track(Box::new(move |track, _, _| {
        let frame_tx = frame_tx_track.clone();
        Box::pin(async move {
            if track.kind() != RTPCodecType::Video {
                return;
            }
            if let Err(e) = read_video_track(track, frame_tx).await {
                warn!("video track reader ended: {e:#}");
            }
        })
    }));

    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        info!(?s, "viewer peer state");
        Box::pin(async {})
    }));

    let write_loop = Arc::clone(&write);
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            outbound = sig_rx.recv() => {
                if let Some(msg) = outbound {
                    let mut w = write_loop.lock().await;
                    w.send(Message::Text(serde_json::to_string(&msg)?)).await?;
                }
            }
            incoming = read.next() => {
                let Some(incoming) = incoming else { break; };
                let incoming = incoming.context("websocket read")?;
                if incoming.is_close() {
                    break;
                }
                let Message::Text(text) = incoming else { continue; };
                let parsed: SignalingMessage = serde_json::from_str(&text)
                    .context("signaling json")?;
                match &parsed {
                    SignalingMessage::HostInfo { display_name, video_width, video_height, has_audio } => {
                        let summary = format!(
                            "{display_name} · {video_width}x{video_height} · audio={has_audio}"
                        );
                        let mut guard = state.lock().await;
                        guard.last_host_info = Some(summary);
                    }
                    SignalingMessage::Error { message } => {
                        let mut guard = state.lock().await;
                        guard.last_error = Some(message.clone());
                    }
                    _ => {}
                }
                if let Some(reply) = apply_remote_message(&pc, parsed).await? {
                    let mut w = write_loop.lock().await;
                    w.send(Message::Text(serde_json::to_string(&reply)?)).await?;
                }
            }
        }
    }

    let mut guard = state.lock().await;
    guard.connected = false;
    Ok(())
}

async fn read_video_track(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    frame_tx: watch::Sender<Option<ViewerFrame>>,
) -> Result<()> {
    let mut decoder = H264Decoder::default();
    let mut depacketizer = H264Packet::default();
    let mut buf = vec![0u8; 1500];
    let mut annex_b = Vec::new();

    while let Ok((rtp, _)) = track.read(&mut buf).await {
        let payload = depacketizer
            .depacketize(&Bytes::copy_from_slice(&rtp.payload))
            .context("h264 depacketize")?;
        if payload.is_empty() {
            continue;
        }
        annex_b.extend_from_slice(&payload);
        if let Some((width, height, rgba)) = decoder.decode_to_rgba(&annex_b)? {
            annex_b.clear();
            if let Some(jpeg) = rgba_to_jpeg(&rgba, width, height) {
                let _ = frame_tx.send(Some(ViewerFrame {
                    width,
                    height,
                    jpeg,
                }));
            }
        }
    }
    Ok(())
}

fn rgba_to_jpeg(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let mut out = Vec::new();
    let encoder = JpegEncoder::new(&mut out, 75);
    encoder
        .encode(
            &rgb,
            width as u16,
            height as u16,
            ColorType::Rgb,
        )
        .ok()?;
    Some(out)
}
