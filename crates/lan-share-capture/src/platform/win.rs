use crate::{CaptureError, CaptureFormat, ScreenCapturer, SystemAudioCapturer};
use scrap::{Capturer, Display};
use std::io::ErrorKind;
use std::time::{Duration, Instant};

pub fn open_primary_display(fps: u32, include_audio: bool) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    let display =
        Display::primary().map_err(|e| CaptureError::Platform(format!("Display::primary: {e}")))?;
    let capturer = Capturer::new(display)
        .map_err(|e| CaptureError::Platform(format!("Capturer::new: {e}")))?;
    let width = capturer.width() as u32;
    let height = capturer.height() as u32;
    Ok(Box::new(WinScreenCapturer {
        capturer,
        width,
        height,
        fps,
        includes_system_audio: include_audio,
        last_frame_at: Instant::now(),
        min_frame_interval: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
    }))
}

struct WinScreenCapturer {
    capturer: Capturer,
    width: u32,
    height: u32,
    fps: u32,
    includes_system_audio: bool,
    last_frame_at: Instant,
    min_frame_interval: Duration,
}

impl ScreenCapturer for WinScreenCapturer {
    fn format(&self) -> CaptureFormat {
        CaptureFormat {
            width: self.width,
            height: self.height,
            fps: self.fps,
            includes_system_audio: self.includes_system_audio,
        }
    }

    fn grab_frame_rgba(&mut self) -> Result<Vec<u8>, CaptureError> {
        let elapsed = self.last_frame_at.elapsed();
        if elapsed < self.min_frame_interval {
            std::thread::sleep(self.min_frame_interval - elapsed);
        }
        self.last_frame_at = Instant::now();

        loop {
            match self.capturer.frame() {
                Ok(frame) => {
                    let mut rgba = Vec::with_capacity(frame.len());
                    for chunk in frame.chunks_exact(4) {
                        rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
                    }
                    return Ok(rgba);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => return Err(CaptureError::Platform(format!("frame: {e}"))),
            }
        }
    }
}

pub fn open_system_audio() -> Result<Option<Box<dyn SystemAudioCapturer>>, CaptureError> {
    // WASAPI loopback wiring comes in the next milestone (WebRTC audio track).
    Err(CaptureError::AudioUnavailable(
        "Windows WASAPI loopback not wired yet".into(),
    ))
}
