mod log_store;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use lan_share_core::{HostSessionConfig, ViewerConnectConfig};
use lan_share_stream::{HostSession, HostSessionHandle, ViewerSession, ViewerSessionHandle};
use log_store::{init_tracing, LogStore};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

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
    web_port: u16,
    web_viewer_urls: Vec<String>,
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
    tracing::info!("用户操作：启动分享");
    let mut guard = state.host.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        tracing::debug!("分享已在运行，忽略重复启动");
        return Ok(());
    }
    let handle = state
        .runtime
        .block_on(HostSession::start(HostSessionConfig::default()))
        .map_err(|e| {
            tracing::error!("启动分享失败: {e}");
            e.to_string()
        })?;
    tracing::info!(
        port = handle.port(),
        web_port = handle.web_port(),
        addrs = ?handle.lan_addresses(),
        "分享已启动"
    );
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_host_session(state: State<'_, AppState>) -> Result<(), String> {
    tracing::info!("用户操作：停止分享");
    let mut guard = state.host.lock().map_err(|e| e.to_string())?;
    if let Some(h) = guard.take() {
        h.stop();
        tracing::info!("分享已停止");
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
            web_port: h.web_port(),
            web_viewer_urls: h.web_viewer_urls(),
        })
    } else {
        Ok(HostStatus {
            running: false,
            addresses: vec![],
            port: lan_share_core::DEFAULT_SIGNALING_PORT,
            web_port: lan_share_core::DEFAULT_WEB_PORT,
            web_viewer_urls: vec![],
        })
    }
}

#[tauri::command]
fn connect_viewer(
    app: AppHandle,
    state: State<'_, AppState>,
    host_addr: String,
) -> Result<(), String> {
    let host_addr = host_addr.trim().to_string();
    tracing::info!(%host_addr, "用户操作：观看端连接");
    if host_addr.is_empty() {
        return Err("请输入分享端 IP".into());
    }
    let mut guard = state.viewer.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        tracing::debug!("观看端已连接，忽略重复连接");
        return Ok(());
    }
    let cfg = ViewerConnectConfig {
        host_addr: host_addr.clone(),
        ..ViewerConnectConfig::default()
    };
    let handle = state
        .runtime
        .block_on(ViewerSession::connect(cfg))
        .map_err(|e| {
            tracing::error!(%host_addr, "观看端连接失败: {e}");
            e.to_string()
        })?;

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

    tracing::info!(%host_addr, "观看端会话已建立");
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn disconnect_viewer(state: State<'_, AppState>) -> Result<(), String> {
    tracing::info!("用户操作：观看端断开");
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

#[tauri::command]
fn get_app_logs(store: State<'_, LogStore>, limit: Option<usize>) -> Vec<String> {
    store.recent(limit.unwrap_or(500))
}

#[tauri::command]
fn clear_app_logs(store: State<'_, LogStore>) {
    store.clear();
    tracing::info!("应用内日志缓冲已清空");
}

#[derive(Clone, Serialize)]
pub struct LogPathsState {
    pub log_dir: String,
    pub log_file: String,
}

#[tauri::command]
fn log_paths(paths: State<'_, LogPathsState>) -> LogPathsState {
    (*paths).clone()
}

#[tauri::command]
fn reveal_log_file(state: State<'_, LogPathsState>) -> Result<(), String> {
    let path = std::path::Path::new(&state.log_file);
    if !path.exists() {
        return Err("日志文件尚未创建".into());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        return Err("当前平台不支持在访达中显示日志".into());
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    lan_share_stream::init_rustls_crypto_provider();

    let log_store = LogStore::new();

    let runtime = ::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    tauri::Builder::default()
        .manage(log_store.clone())
        .manage(AppState {
            host: Mutex::new(None),
            viewer: Mutex::new(None),
            runtime,
        })
        .setup(move |app| {
            let log_dir = app
                .path()
                .app_log_dir()
                .map_err(|e| format!("resolve log dir: {e}"))?;
            let (bootstrap, log_file) = init_tracing(&log_dir, log_store)?;
            app.manage(bootstrap);
            app.manage(LogPathsState {
                log_dir: log_dir.to_string_lossy().into_owned(),
                log_file: log_file.to_string_lossy().into_owned(),
            });
            tracing::info!(
                log_dir = %log_dir.display(),
                log_file = %log_file.display(),
                "诊断日志已就绪（应用内「日志」页可查看，文件会持续追加）"
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_host_session,
            stop_host_session,
            host_status,
            connect_viewer,
            disconnect_viewer,
            viewer_status,
            get_app_logs,
            clear_app_logs,
            log_paths,
            reveal_log_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
