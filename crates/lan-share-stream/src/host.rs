use crate::media::{fit_width_dims, scale_rgba_nearest, rgba_to_i420, H264Encoder};
use crate::net;
use crate::webrtc::{build_peer_connection, mime_type_h264};
use crate::webrtc::{apply_remote_message, wire_ice_candidates};
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use lan_share_capture::{open_primary_display, ScreenCapturer};
use lan_share_core::{HostSessionConfig, SignalingMessage};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{info, warn};
use webrtc::media::Sample;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

#[derive(Clone)]
pub struct HostSessionHandle {
    stop: watch::Sender<bool>,
    addrs: Arc<Vec<String>>,
    port: u16,
}

impl HostSessionHandle {
    pub fn lan_addresses(&self) -> Vec<String> {
        self.addrs.as_ref().clone()
    }

    pub fn port(&self) -> u16 {
        self.port
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
        info!(%bind, "signaling server listening");

        let addrs = Arc::new(net::list_lan_ipv4());
        let (stop_tx, stop_rx) = watch::channel(false);

        let cfg = config.clone();
        tokio::spawn(async move {
            if let Err(e) = run_host_loop(listener, stop_rx, cfg).await {
                warn!("host session ended: {e:#}");
            }
        });

        Ok(HostSessionHandle {
            stop: stop_tx,
            addrs,
            port: config.port,
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
                info!(%peer, "viewer connected");
                let cfg = config.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_viewer(stream, cfg).await {
                        warn!(%peer, "viewer session error: {e:#}");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_viewer(stream: TcpStream, config: HostSessionConfig) -> Result<()> {
    let ws = accept_async(stream).await.context("websocket handshake")?;
    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));

    let capturer = open_primary_display(config.target_fps, config.include_audio)
        .context("open screen capture")?;
    let format = capturer.format();
    let (stream_w, stream_h) = fit_width_dims(format.width, format.height, config.max_width);

    {
        let mut w = write.lock().await;
        let hello = SignalingMessage::HostInfo {
            display_name: config.display_name.clone(),
            video_width: stream_w,
            video_height: stream_h,
            has_audio: config.include_audio,
        };
        w.send(Message::Text(serde_json::to_string(&hello)?))
            .await?;
    }

    // Wait for viewer hello (optional).
    if let Some(Ok(Message::Text(_))) = read.next().await {}

    let pc = build_peer_connection().await?;
    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: mime_type_h264().to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "lan-screenshare".to_owned(),
    ));
    pc.add_track(Arc::clone(&video_track) as Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync>)
        .await?;

    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel();
    wire_ice_candidates(Arc::clone(&pc), sig_tx.clone());

    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    {
        let mut w = write.lock().await;
        w.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Offer {
                sdp: offer.sdp,
            })?,
        ))
        .await?;
    }

    let capturer = Arc::new(Mutex::new(capturer));
    let fps = config.target_fps.max(1);
    let frame_duration = Duration::from_secs_f64(1.0 / fps as f64);
    let pump_track = Arc::clone(&video_track);
    let mut pump_started = false;

    let write_sig = Arc::clone(&write);
    loop {
        tokio::select! {
            outbound = sig_rx.recv() => {
                if let Some(msg) = outbound {
                    let mut w = write_sig.lock().await;
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
                let is_answer = matches!(&parsed, SignalingMessage::Answer { .. });
                if let Some(reply) = apply_remote_message(&pc, parsed).await? {
                    let mut w = write_sig.lock().await;
                    w.send(Message::Text(serde_json::to_string(&reply)?)).await?;
                }
                if is_answer && !pump_started {
                    pump_started = true;
                    let capturer = Arc::clone(&capturer);
                    let pump_track = Arc::clone(&pump_track);
                    let pump_cfg = config.clone();
                    tokio::spawn(async move {
                        if let Err(e) = pump_video(capturer, pump_track, pump_cfg, stream_w, stream_h, frame_duration).await {
                            warn!("video pump stopped: {e:#}");
                        }
                    });
                }
            }
        }
    }
    Ok(())
}

async fn pump_video(
    capturer: Arc<Mutex<Box<dyn ScreenCapturer>>>,
    track: Arc<TrackLocalStaticSample>,
    config: HostSessionConfig,
    stream_w: u32,
    stream_h: u32,
    frame_duration: Duration,
) -> Result<()> {
    let mut encoder = H264Encoder::new(stream_w, stream_h, config.target_fps)?;
    loop {
        let (rgba, src_w, src_h) = {
            let mut cap = capturer.lock().await;
            let fmt = cap.format();
            (cap.grab_frame_rgba()?, fmt.width, fmt.height)
        };
        let scaled = if src_w != stream_w || src_h != stream_h {
            scale_rgba_nearest(&rgba, src_w, src_h, stream_w, stream_h)
        } else {
            rgba
        };
        let i420 = rgba_to_i420(&scaled, stream_w, stream_h);
        let nal = encoder.encode_i420(&i420)?;
        if !nal.is_empty() {
            track
                .write_sample(&Sample {
                    data: Bytes::from(nal),
                    duration: frame_duration,
                    ..Default::default()
                })
                .await?;
        }
        tokio::time::sleep(frame_duration).await;
    }
}
