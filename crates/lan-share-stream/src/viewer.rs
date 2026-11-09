use crate::media::H264Decoder;
use crate::webrtc::{apply_remote_message, build_peer_connection, wire_ice_candidates};
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use jpeg_encoder::{ColorType, Encoder as JpegEncoder};
use lan_share_core::{SignalingMessage, ViewerConnectConfig, ViewerFrame};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::rtp::codecs::h264::H264Packet;
use webrtc::rtp::packetizer::Depacketizer;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;

pub const VIEWER_PROGRESS_TOTAL: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewerProgressPhase {
    Running,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct ViewerProgress {
    pub generation: u64,
    pub step: u32,
    pub total: u32,
    pub title: String,
    pub detail: String,
    pub phase: ViewerProgressPhase,
}

#[derive(Clone, Debug)]
pub struct ViewerSnapshot {
    pub connected: bool,
    pub host_info: Option<String>,
    pub last_error: Option<String>,
    pub progress: Option<ViewerProgress>,
}

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
    progress: Option<ViewerProgress>,
}

#[derive(Clone)]
struct ProgressBus {
    generation: u64,
    tx: mpsc::UnboundedSender<ViewerProgress>,
    state: Arc<Mutex<ViewerState>>,
}

impl ProgressBus {
    async fn emit(
        &self,
        step: u32,
        title: &str,
        detail: impl Into<String>,
        phase: ViewerProgressPhase,
    ) {
        let progress = ViewerProgress {
            generation: self.generation,
            step,
            total: VIEWER_PROGRESS_TOTAL,
            title: title.to_string(),
            detail: detail.into(),
            phase,
        };
        {
            let mut guard = self.state.lock().await;
            if phase == ViewerProgressPhase::Error {
                guard.connected = false;
                guard.last_error = Some(progress.detail.clone());
            }
            guard.progress = Some(progress.clone());
        }
        let _ = self.tx.send(progress);
    }
}

impl ViewerSessionHandle {
    pub fn snapshot(&self) -> ViewerSnapshot {
        if let Ok(guard) = self.inner.try_lock() {
            ViewerSnapshot {
                connected: guard.connected,
                host_info: guard.last_host_info.clone(),
                last_error: guard.last_error.clone(),
                progress: guard.progress.clone(),
            }
        } else {
            ViewerSnapshot {
                connected: false,
                host_info: None,
                last_error: None,
                progress: None,
            }
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
    pub async fn connect(
        config: ViewerConnectConfig,
        generation: u64,
        progress_tx: mpsc::UnboundedSender<ViewerProgress>,
    ) -> Result<ViewerSessionHandle> {
        let url = format!("ws://{}:{}/", config.host_addr.trim(), config.port);
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(ViewerState::default()));
        let (frame_tx, _) = watch::channel(None);
        let bus = ProgressBus {
            generation,
            tx: progress_tx,
            state: Arc::clone(&state),
        };

        let frame_task = frame_tx.clone();
        let bus_task = bus.clone();
        tokio::spawn(async move {
            if let Err(e) = run_viewer(&url, stop_rx, bus_task.clone(), frame_task).await {
                let msg = format!("{e:#}");
                warn!("viewer session ended: {msg}");
                let existing = bus_task.state.lock().await.progress.clone();
                if matches!(
                    existing.as_ref().map(|p| p.phase),
                    Some(ViewerProgressPhase::Error)
                ) {
                    return;
                }
                let (step, title) = existing
                    .map(|p| (p.step, p.title))
                    .unwrap_or((2, "连接信令".into()));
                bus_task
                    .emit(step, &title, msg, ViewerProgressPhase::Error)
                    .await;
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
    bus: ProgressBus,
    frame_tx: watch::Sender<Option<ViewerFrame>>,
) -> Result<()> {
    bus.emit(
        2,
        "连接信令",
        format!("正在连接 {url}"),
        ViewerProgressPhase::Running,
    )
    .await;
    let (ws, _) = connect_async(url).await.with_context(|| {
        format!("无法连接信令 {url}。请确认分享端已点「启动分享」，IP 正确，且两端在同一局域网。桌面观看端只需填 IP，默认端口 17890。")
    })?;
    info!(%url, "connected to host signaling");
    {
        let mut guard = bus.state.lock().await;
        guard.connected = true;
        guard.last_error = None;
    }

    bus.emit(
        3,
        "建立媒体通道",
        "信令已接通，正在创建 WebRTC",
        ViewerProgressPhase::Running,
    )
    .await;

    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));

    let hello = SignalingMessage::Hello {
        role: lan_share_core::SessionRole::Viewer,
        name: "Viewer".into(),
    };
    {
        let mut w = write.lock().await;
        w.send(Message::Text(serde_json::to_string(&hello)?))
            .await
            .context("发送握手失败")?;
    }

    let pc = build_peer_connection()
        .await
        .context("创建 WebRTC 失败")?;
    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel();
    wire_ice_candidates(Arc::clone(&pc), sig_tx.clone());

    let frame_tx_track = frame_tx.clone();
    let bus_track = bus.clone();
    let announced_frame = Arc::new(AtomicBool::new(false));
    pc.on_track(Box::new(move |track, _, _| {
        let frame_tx = frame_tx_track.clone();
        let bus_track = bus_track.clone();
        let announced_frame = Arc::clone(&announced_frame);
        Box::pin(async move {
            if track.kind() != RTPCodecType::Video {
                return;
            }
            if let Err(e) = read_video_track(track, frame_tx, bus_track, announced_frame).await {
                warn!("video track reader ended: {e:#}");
            }
        })
    }));

    let bus_pc = bus.clone();
    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        let bus_pc = bus_pc.clone();
        Box::pin(async move {
            info!(?s, "观看端 WebRTC 连接状态");
            if s == RTCPeerConnectionState::Failed {
                let (step, title) = {
                    let guard = bus_pc.state.lock().await;
                    match guard.progress.as_ref() {
                        Some(p) => (p.step, p.title.clone()),
                        None => (6, "接收画面".into()),
                    }
                };
                bus_pc
                    .emit(
                        step,
                        &title,
                        "媒体连接失败。请确认两端在同一局域网，并允许本应用的 UDP。",
                        ViewerProgressPhase::Error,
                    )
                    .await;
            }
        })
    }));

    bus.emit(
        4,
        "获取分享端信息",
        "媒体通道已就绪，等待分享端描述画面",
        ViewerProgressPhase::Running,
    )
    .await;

    let mut saw_host = false;
    let mut saw_answer = false;
    let mut inbound_count = 0u32;
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
                    w.send(Message::Text(serde_json::to_string(&msg)?)).await.context("发送信令失败")?;
                }
            }
            incoming = read.next() => {
                let Some(incoming) = incoming else { break; };
                let incoming = incoming.map_err(|err| {
                    let raw = err.to_string();
                    if raw.contains("Connection reset") || raw.contains("without closing handshake") {
                        anyhow::anyhow!(
                            "分享端在接通后立刻断开了连接。请确认分享端已点「启动分享」，并在系统设置 → 隐私与安全性 → 屏幕录制中允许它。macOS 15 需要使用已更新的分享端。"
                        )
                    } else {
                        anyhow::anyhow!("读取信令失败：{raw}")
                    }
                })?;
                if incoming.is_close() {
                    info!(inbound_count, saw_host, saw_answer, "分享端关闭了信令");
                    break;
                }
                let Message::Text(text) = incoming else { continue; };
                inbound_count += 1;
                let parsed: SignalingMessage = serde_json::from_str(&text)
                    .context("信令数据无法解析")?;
                match &parsed {
                    SignalingMessage::HostInfo { display_name, video_width, video_height, has_audio } => {
                        let summary = format!(
                            "{display_name} · {video_width}x{video_height} · audio={has_audio}"
                        );
                        {
                            let mut guard = bus.state.lock().await;
                            guard.last_host_info = Some(summary.clone());
                        }
                        if !saw_host {
                            saw_host = true;
                            bus.emit(
                                5,
                                "协商视频",
                                format!("已收到分享端信息：{summary}"),
                                ViewerProgressPhase::Running,
                            )
                            .await;
                        }
                    }
                    SignalingMessage::Error { message } => {
                        anyhow::bail!("分享端返回错误：{message}");
                    }
                    _ => {}
                }
                if let Some(reply) = apply_remote_message(&pc, parsed).await.context("处理信令失败")? {
                    let is_answer = matches!(reply, SignalingMessage::Answer { .. });
                    {
                        let mut w = write_loop.lock().await;
                        w.send(Message::Text(serde_json::to_string(&reply)?)).await.context("发送应答失败")?;
                    }
                    if is_answer && !saw_answer {
                        saw_answer = true;
                        bus.emit(
                            6,
                            "接收画面",
                            "视频协商完成，等待第一帧",
                            ViewerProgressPhase::Running,
                        )
                        .await;
                    }
                }
            }
        }
    }

    let stopped = *stop.borrow();
    {
        let mut guard = bus.state.lock().await;
        guard.connected = false;
    }
    if stopped {
        return Ok(());
    }
    let phase = bus.state.lock().await.progress.as_ref().map(|p| p.phase);
    if phase == Some(ViewerProgressPhase::Done) {
        anyhow::bail!("播放过程中信令断开");
    }
    if inbound_count == 0 {
        anyhow::bail!(
            "分享端在握手后立刻关闭了信令，没有发来画面信息。请在分享端允许「本地网络」和「屏幕录制」，并使用已更新的 Mac 安装包后重新点「启动分享」。"
        );
    }
    anyhow::bail!(
        "信令在收到画面前断开（已收到 {inbound_count} 条信令）。请确认分享端已允许屏幕录制，且两台电脑都允许本应用访问本地网络。"
    );
}

async fn read_video_track(
    track: Arc<webrtc::track::track_remote::TrackRemote>,
    frame_tx: watch::Sender<Option<ViewerFrame>>,
    bus: ProgressBus,
    announced_frame: Arc<AtomicBool>,
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
                if announced_frame
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    bus.emit(
                        6,
                        "接收画面",
                        format!("已收到画面 {width}x{height}"),
                        ViewerProgressPhase::Done,
                    )
                    .await;
                }
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
