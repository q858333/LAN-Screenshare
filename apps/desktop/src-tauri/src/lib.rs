mod log_store;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use lan_share_core::{HostSessionConfig, ViewerConnectConfig, DEFAULT_SIGNALING_PORT};
use lan_share_stream::{
    HostSession, HostSessionHandle, ViewerProgress, ViewerProgressPhase, ViewerSession,
    ViewerSessionHandle, VIEWER_PROGRESS_TOTAL,
};
use log_store::{init_tracing, LogStore};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

struct AppState {
    host: Mutex<Option<HostSessionHandle>>,
    viewer: Mutex<Option<ViewerSessionHandle>>,
    runtime: tokio::runtime::Runtime,
    viewer_generation: AtomicU64,
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
    progress: Option<ViewerProgress>,
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
) -> Result<u64, String> {
    let host_addr = host_addr.trim().to_string();
    tracing::info!(%host_addr, "用户操作：观看端连接");
    let (host, port) = parse_viewer_target(&host_addr)?;
    let generation = state.viewer_generation.fetch_add(1, Ordering::SeqCst) + 1;

    {
        let mut guard = state.viewer.lock().map_err(|e| e.to_string())?;
        if let Some(prev) = guard.take() {
            tracing::info!("观看端重新连接，先停止上一次会话");
            prev.stop();
        }
    }

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
    let app_progress = app.clone();
    state.runtime.spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_progress.emit("viewer-progress", progress);
        }
    });
    let _ = progress_tx.send(ViewerProgress {
        generation,
        step: 1,
        total: VIEWER_PROGRESS_TOTAL,
        title: "检查地址".into(),
        detail: format!(
            "将连接 {host}:{port}。只需填写 IP，未写端口时使用默认 {DEFAULT_SIGNALING_PORT}。"
        ),
        phase: ViewerProgressPhase::Done,
    });

    let cfg = ViewerConnectConfig {
        host_addr: host.clone(),
        port,
    };
    let handle = state
        .runtime
        .block_on(ViewerSession::connect(cfg, generation, progress_tx))
        .map_err(|e| {
            tracing::error!(%host, port, "观看端连接失败: {e}");
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

    tracing::info!(%host, port, generation, "观看端会话已开始");
    let mut guard = state.viewer.lock().map_err(|e| e.to_string())?;
    *guard = Some(handle);
    Ok(generation)
}

fn parse_viewer_target(raw: &str) -> Result<(String, u16), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("请输入分享端 IP。不用填写端口，默认是 17890。".into());
    }
    if raw.contains("://") || raw.contains('/') || raw.contains(' ') {
        return Err("只需填写 IP，例如 192.168.1.12，不要带 http://。".into());
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let Some((addr, after)) = rest.split_once(']') else {
            return Err("IPv6 请写成 [地址] 或 [地址]:端口".into());
        };
        if addr.is_empty() {
            return Err("IPv6 地址为空".into());
        }
        let host = format!("[{addr}]");
        if after.is_empty() {
            return Ok((host, DEFAULT_SIGNALING_PORT));
        }
        let Some(port_str) = after.strip_prefix(':') else {
            return Err("IPv6 端口请写成 [地址]:端口".into());
        };
        return Ok((host, parse_port(port_str)?));
    }
    if raw.matches(':').count() > 1 {
        return Err("IPv6 请写成 [地址]，例如 [::1]。平常只填 IPv4，不用写端口。".into());
    }
    if let Some((host, port_str)) = raw.rsplit_once(':') {
        if host.is_empty() {
            return Err("请输入分享端 IP。不用填写端口，默认是 17890。".into());
        }
        return Ok((host.to_string(), parse_port(port_str)?));
    }
    Ok((raw.to_string(), DEFAULT_SIGNALING_PORT))
}

fn parse_port(raw: &str) -> Result<u16, String> {
    if raw.is_empty() {
        return Err("端口为空。只填 IP 即可，例如 192.168.1.12，默认端口 17890。".into());
    }
    let port: u16 = raw.parse().map_err(|_| {
        "端口无效。只填 IP 即可，例如 192.168.1.12；只有改过端口才写成 192.168.1.12:17890。"
            .to_string()
    })?;
    if port == 0 {
        return Err("端口不能为 0。只填 IP 即可，默认端口是 17890。".into());
    }
    Ok(port)
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
        let snap = h.snapshot();
        Ok(ViewerStatus {
            connected: snap.connected,
            host_info: snap.host_info,
            last_error: snap.last_error,
            progress: snap.progress,
        })
    } else {
        Ok(ViewerStatus {
            connected: false,
            host_info: None,
            last_error: None,
            progress: None,
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
            viewer_generation: AtomicU64::new(0),
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

#[cfg(test)]
mod tests {
    use super::{parse_port, parse_viewer_target};
    use lan_share_core::DEFAULT_SIGNALING_PORT;

    #[test]
    fn ip_without_port_uses_default() {
        assert_eq!(
            parse_viewer_target("192.168.1.12").unwrap(),
            ("192.168.1.12".into(), DEFAULT_SIGNALING_PORT)
        );
    }

    #[test]
    fn explicit_port_is_optional() {
        assert_eq!(
            parse_viewer_target("192.168.1.12:18000").unwrap(),
            ("192.168.1.12".into(), 18000)
        );
    }

    #[test]
    fn rejects_url_and_bad_port() {
        assert!(parse_viewer_target("http://192.168.1.12").is_err());
        assert!(parse_viewer_target("").is_err());
        assert!(parse_port("abc").is_err());
        assert!(parse_port("0").is_err());
    }
}
