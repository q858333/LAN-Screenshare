use crate::media::{fit_width_dims, scale_rgba_nearest, rgba_to_i420, H264Encoder};
use crate::webrtc::{apply_remote_message, build_peer_connection, mime_type_h264, wire_ice_candidates};
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use lan_share_capture::{open_primary_display, ScreenCapturer};
use lan_share_core::{HostSessionConfig, SignalingMessage};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;
use webrtc::media::Sample;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

pub async fn run_host_viewer_session(
    config: HostSessionConfig,
    mut inbound: mpsc::UnboundedReceiver<String>,
    outbound: mpsc::UnboundedSender<String>,
) -> Result<()> {
    let send = |text: String| {
        let outbound = outbound.clone();
        async move {
            outbound.send(text).map_err(|e| anyhow::anyhow!(e))?;
            Ok::<(), anyhow::Error>(())
        }
    };

    let capturer = open_primary_display(config.target_fps, config.include_audio)
        .context("open screen capture")?;
    let format = capturer.format();
    let (stream_w, stream_h) = fit_width_dims(format.width, format.height, config.max_width);

    send(serde_json::to_string(&SignalingMessage::HostInfo {
        display_name: config.display_name.clone(),
        video_width: stream_w,
        video_height: stream_h,
        has_audio: config.include_audio,
    })?)
    .await?;

    // Wait for viewer hello (optional).
    let _ = inbound.recv().await;

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
    send(serde_json::to_string(&SignalingMessage::Offer {
        sdp: offer.sdp,
    })?)
    .await?;

    let capturer = Arc::new(Mutex::new(capturer));
    let fps = config.target_fps.max(1);
    let frame_duration = Duration::from_secs_f64(1.0 / fps as f64);
    let pump_track = Arc::clone(&video_track);
    let mut pump_started = false;

    loop {
        tokio::select! {
            outbound = sig_rx.recv() => {
                if let Some(msg) = outbound {
                    send(serde_json::to_string(&msg)?).await?;
                }
            }
            incoming = inbound.recv() => {
                let Some(text) = incoming else { break; };
                let parsed: SignalingMessage = serde_json::from_str(&text)
                    .context("signaling json")?;
                let is_answer = matches!(&parsed, SignalingMessage::Answer { .. });
                if let Some(reply) = apply_remote_message(&pc, parsed).await? {
                    send(serde_json::to_string(&reply)?).await?;
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
