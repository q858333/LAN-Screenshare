use crate::{CaptureError, CaptureFormat, ScreenCapturer, SystemAudioCapturer};
use scrap::{Capturer, Display};
use std::io::ErrorKind;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub fn open_primary_display(fps: u32, include_audio: bool) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    let (tx, rx) = mpsc::sync_channel(2);
    let min_frame_interval = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    let (fmt_tx, fmt_rx) = mpsc::sync_channel(1);

    let worker = thread::spawn(move || {
        let run = || -> Result<(), CaptureError> {
            let display = Display::primary()
                .map_err(|e| CaptureError::Platform(format!("Display::primary: {e}")))?;
            let mut capturer = Capturer::new(display)
                .map_err(|e| CaptureError::Platform(format!("Capturer::new: {e}")))?;
            let width = capturer.width() as u32;
            let height = capturer.height() as u32;
            let _ = fmt_tx.send(CaptureFormat {
                width,
                height,
                fps,
                includes_system_audio: include_audio,
            });
            capture_loop(&mut capturer, tx, min_frame_interval);
            Ok(())
        };
        if run().is_err() {
            let _ = fmt_tx.send(CaptureFormat {
                width: 0,
                height: 0,
                fps,
                includes_system_audio: include_audio,
            });
        }
    });

    let format = fmt_rx
        .recv()
        .map_err(|_| CaptureError::Platform("capture thread failed to start".into()))?;
    if format.width == 0 {
        return Err(CaptureError::NoDisplay);
    }

    Ok(Box::new(WinScreenCapturer {
        rx,
        format,
        _worker: worker,
    }))
}

fn capture_loop(
    capturer: &mut Capturer,
    tx: SyncSender<Result<Vec<u8>, CaptureError>>,
    min_frame_interval: Duration,
) {
    let mut last_frame_at = Instant::now();
    loop {
        let elapsed = last_frame_at.elapsed();
        if elapsed < min_frame_interval {
            thread::sleep(min_frame_interval - elapsed);
        }
        last_frame_at = Instant::now();

        let frame = loop {
            match capturer.frame() {
                Ok(frame) => {
                    let mut rgba = Vec::with_capacity(frame.len());
                    for chunk in frame.chunks_exact(4) {
                        rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
                    }
                    break Ok(rgba);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) => break Err(CaptureError::Platform(format!("frame: {e}"))),
            }
        };
        if tx.send(frame).is_err() {
            break;
        }
    }
}

struct WinScreenCapturer {
    rx: Receiver<Result<Vec<u8>, CaptureError>>,
    format: CaptureFormat,
    _worker: JoinHandle<()>,
}

impl ScreenCapturer for WinScreenCapturer {
    fn format(&self) -> CaptureFormat {
        self.format.clone()
    }

    fn grab_frame_rgba(&mut self) -> Result<Vec<u8>, CaptureError> {
        self.rx
            .recv()
            .map_err(|_| CaptureError::Platform("capture thread stopped".into()))?
    }
}

pub fn open_system_audio() -> Result<Option<Box<dyn SystemAudioCapturer>>, CaptureError> {
    Err(CaptureError::AudioUnavailable(
        "Windows WASAPI loopback not wired yet".into(),
    ))
}
