//! Screen and system-audio capture (macOS + Windows only).

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
compile_error!("lan-share-capture supports only macOS and Windows");

mod platform;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CaptureFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub includes_system_audio: bool,
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("no display available")]
    NoDisplay,
    #[error("platform capture failed: {0}")]
    Platform(String),
    #[error("audio capture unavailable on this platform: {0}")]
    AudioUnavailable(String),
}

pub trait ScreenCapturer: Send {
    fn format(&self) -> CaptureFormat;
    fn grab_frame_rgba(&mut self) -> Result<Vec<u8>, CaptureError>;
}

pub trait SystemAudioCapturer: Send {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Interleaved PCM f32 samples since last poll.
    fn poll_samples(&mut self) -> Result<Vec<f32>, CaptureError>;
}

pub fn open_primary_display(fps: u32, include_audio: bool) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    platform::open_primary_display(fps, include_audio)
}

pub fn open_system_audio(include_audio: bool) -> Result<Option<Box<dyn SystemAudioCapturer>>, CaptureError> {
    if !include_audio {
        return Ok(None);
    }
    platform::open_system_audio()
}
