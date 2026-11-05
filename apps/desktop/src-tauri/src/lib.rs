use base64::{engine::general_purpose::STANDARD, Engine as _};
use lan_share_core::{HostSessionConfig, ViewerConnectConfig};
use lan_share_stream::{HostSession, HostSessionHandle, ViewerSession, ViewerSessionHandle};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

struct AppState {
    host: Mutex<Option<HostSessionHandle>>,
    viewer: Mutex<Option<ViewerSessionHandle>>,
    runtime: tokio::runtime::Runtime,
}

#[derive(Serialize)]
struct HostStatus {
    running: bool,
    addresses: Vec<String>,
    port: u16,
}

#[derive(Serialize)]
struct ViewerStatus {
    connected: bool,
    host_info: Option<String>,
    last_error: Option<String>,
}

#[derive(Clone, Serialize)]
struct ViewerFrameEvent {
    width: u32,
    height: u32,
    jpeg_base64: String,
}

#[tauri::command]
fn start_host_session(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.host.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }
    let handle = state
        .runtime
        .block_on(HostSession::start(HostSessionConfig::default()))
        .map_err(|e| e.to_string())?;
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_host_session(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.host.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.take() {
        h.stop();
    }
    Ok(())
}

#[tauri::command]
fn host_status(state: State<'_, AppState>) -> Result<HostStatus, String> {
    let guard = state.host.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.as_ref() {
        Ok(HostStatus {
            running: true,
            addresses: h.lan_addresses(),
            port: h.port(),
        })
    } else {
        Ok(HostStatus {
            running: false,
            addresses: vec![],
            port: lan_share_core::DEFAULT_SIGNALING_PORT,
        })
    }
}

#[tauri::command]
fn connect_viewer(
    app: AppHandle,
    state: State<'_, AppState>,
    host_addr: String,
) -> Result<(), String> {
    if host_addr.trim().is_empty() {
        return Err("请输入分享端 IP".into());
    }
    let mut guard = state.viewer.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }
    let cfg = ViewerConnectConfig {
        host_addr,
        ..ViewerConnectConfig::default()
    };
    let handle = state
        .runtime
        .block_on(ViewerSession::connect(cfg))
        .map_err(|e| e.to_string())?;

    let mut frame_rx = handle.subscribe_frames();
    let app_emit = app.clone();
    state.runtime.spawn(async move {
        loop {
            if frame_rx.changed().await.is_err() {
                break;
            }
            let frame = frame_rx.borrow().clone();
            if let Some(frame) = frame {
                let payload = ViewerFrameEvent {
                    width: frame.width,
                    height: frame.height,
                    jpeg_base64: STANDARD.encode(frame.jpeg),
                };
                let _ = app_emit.emit("viewer-frame", payload);
            }
        }
    });

    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn disconnect_viewer(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.viewer.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.take() {
        h.stop();
    }
    Ok(())
}

#[tauri::command]
fn viewer_status(state: State<'_, AppState>) -> Result<ViewerStatus, String> {
    let guard = state.viewer.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.as_ref() {
        let (connected, host_info, last_error) = h.snapshot();
        Ok(ViewerStatus {
            connected,
            host_info,
            last_error,
        })
    } else {
        Ok(ViewerStatus {
            connected: false,
            host_info: None,
            last_error: None,
        })
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let runtime = ::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    tauri::Builder::default()
        .manage(AppState {
            host: Mutex::new(None),
            viewer: Mutex::new(None),
            runtime,
        })
        .invoke_handler(tauri::generate_handler![
            start_host_session,
            stop_host_session,
            host_status,
            connect_viewer,
            disconnect_viewer,
            viewer_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
