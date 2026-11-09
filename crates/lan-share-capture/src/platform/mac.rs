use crate::{CaptureError, CaptureFormat, ScreenCapturer, SystemAudioCapturer};
use std::ffi::{c_char, c_void, CStr};
use std::sync::mpsc::{self, Receiver, SyncSender};

extern "C" {
    fn sck_start(
        fps: u32,
        callback: extern "C" fn(*const u8, u32, u32, usize, *mut c_void),
        ctx: *mut c_void,
        out_width: *mut u32,
        out_height: *mut u32,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut c_void;
    fn sck_stop(handle: *mut c_void);
}

extern "C" fn on_frame(
    bgra: *const u8,
    width: u32,
    height: u32,
    stride: usize,
    ctx: *mut c_void,
) {
    if bgra.is_null() || width == 0 || height == 0 {
        return;
    }
    let tx = unsafe { &*(ctx as *const SyncSender<Vec<u8>>) };
    let src = unsafe { std::slice::from_raw_parts(bgra, stride.saturating_mul(height as usize)) };
    let _ = tx.try_send(bgra_to_rgba(src, width, height, stride));
}

pub fn open_primary_display(
    fps: u32,
    include_audio: bool,
) -> Result<Box<dyn ScreenCapturer>, CaptureError> {
    let (tx, rx) = mpsc::sync_channel(2);
    let sender = Box::new(tx);
    let ctx = Box::into_raw(sender) as *mut c_void;
    let mut err = [0u8; 512];
    let mut width = 0u32;
    let mut height = 0u32;
    let handle = unsafe {
        sck_start(
            fps.max(1),
            on_frame,
            ctx,
            &mut width,
            &mut height,
            err.as_mut_ptr().cast(),
            err.len(),
        )
    };
    if handle.is_null() {
        drop(unsafe { Box::from_raw(ctx as *mut SyncSender<Vec<u8>>) });
        let message = CStr::from_bytes_until_nul(&err)
            .ok()
            .and_then(|text| text.to_str().ok())
            .filter(|text| !text.is_empty())
            .unwrap_or("无法启动屏幕采集");
        return Err(CaptureError::Platform(format!(
            "{message}。请在系统设置 → 隐私与安全性 → 屏幕录制中允许本应用，然后重新点「启动分享」。"
        )));
    }

    Ok(Box::new(MacScreenCapturer {
        rx,
        format: CaptureFormat {
            width,
            height,
            fps: fps.max(1),
            includes_system_audio: include_audio,
        },
        ctx: Some(ctx),
        handle: Some(handle),
    }))
}

fn bgra_to_rgba(src: &[u8], width: u32, height: u32, stride: usize) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height as usize {
        let start = y * stride;
        let Some(row) = src.get(start..start + row_bytes) else {
            break;
        };
        for pixel in row.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    rgba
}

struct MacScreenCapturer {
    rx: Receiver<Vec<u8>>,
    format: CaptureFormat,
    ctx: Option<*mut c_void>,
    handle: Option<*mut c_void>,
}

unsafe impl Send for MacScreenCapturer {}

impl Drop for MacScreenCapturer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { sck_stop(handle) };
        }
        if let Some(ctx) = self.ctx.take() {
            drop(unsafe { Box::from_raw(ctx as *mut SyncSender<Vec<u8>>) });
        }
    }
}

impl ScreenCapturer for MacScreenCapturer {
    fn format(&self) -> CaptureFormat {
        self.format.clone()
    }

    fn grab_frame_rgba(&mut self) -> Result<Vec<u8>, CaptureError> {
        self.rx
            .recv()
            .map_err(|_| CaptureError::Platform("屏幕采集已停止".into()))
    }
}

pub fn open_system_audio() -> Result<Option<Box<dyn SystemAudioCapturer>>, CaptureError> {
    Err(CaptureError::AudioUnavailable(
        "macOS system audio via ScreenCaptureKit not wired yet".into(),
    ))
}
