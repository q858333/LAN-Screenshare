use crate::net;
use crate::session::run_host_viewer_session;
use anyhow::{Context, Result};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Html,
    routing::get,
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use lan_share_core::HostSessionConfig;
use tokio::sync::{mpsc, watch};
use tower_http::cors::CorsLayer;
use tracing::info;

const INDEX_HTML: &str = include_str!("../../../web/viewer/index.html");
const VIEWER_CSS: &str = include_str!("../../../web/viewer/viewer.css");
const VIEWER_JS: &str = include_str!("../../../web/viewer/viewer.js");

pub async fn start(config: HostSessionConfig, mut stop: watch::Receiver<bool>) -> Result<()> {
    let port = config.web_port;
    let bind = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let tls = rustls_config()
        .await
        .context("build TLS config for web viewer")?;

    let app = app_router(config);

    info!(%bind, "web viewer portal listening (HTTPS)");

    let handle = axum_server::Handle::new();
    let handle_shutdown = handle.clone();
    tokio::spawn(async move {
        let _ = stop.changed().await;
        handle_shutdown.graceful_shutdown(None);
    });

    axum_server::bind_rustls(bind, tls)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("web portal server")?;

    Ok(())
}

fn app_router(config: HostSessionConfig) -> Router {
    Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route(
            "/viewer.css",
            get(|| async { ([("content-type", "text/css")], VIEWER_CSS) }),
        )
        .route(
            "/viewer.js",
            get(|| async { ([("content-type", "application/javascript")], VIEWER_JS) }),
        )
        .route(
            "/ws",
            get({
                let cfg = config.clone();
                move |ws: WebSocketUpgrade| {
                    let cfg = cfg.clone();
                    async move { ws.on_upgrade(move |socket| handle_web_socket(socket, cfg)) }
                }
            }),
        )
        .layer(CorsLayer::permissive())
}

async fn handle_web_socket(socket: WebSocket, config: HostSessionConfig) {
    let (mut write, mut read) = socket.split();
    let (tx_in, rx_in) = mpsc::unbounded_channel::<String>();
    let (tx_out, mut rx_out) = mpsc::unbounded_channel::<String>();

    let write_task = tokio::spawn(async move {
        while let Some(text) = rx_out.recv().await {
            if write.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = read.next().await {
            let Message::Text(text) = msg else { continue };
            let _ = tx_in.send(text);
        }
    });

    let session = run_host_viewer_session(config, rx_in, tx_out).await;
    write_task.abort();
    read_task.abort();
    if let Err(e) = session {
        tracing::warn!("web viewer session: {e:#}");
    }
}

async fn rustls_config() -> Result<RustlsConfig> {
    let mut names = vec!["localhost".to_string()];
    names.extend(net::list_lan_ipv4());
    let cert = rcgen::generate_simple_self_signed(names).context("generate self-signed cert")?;
    RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.key_pair.serialize_pem().into_bytes(),
    )
    .await
    .context("load rustls config")
}
