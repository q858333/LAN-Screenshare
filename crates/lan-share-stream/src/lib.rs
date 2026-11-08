mod host;
mod media;
mod net;
mod session;
mod viewer;
mod web_portal;
mod webrtc;

pub use host::{HostSession, HostSessionHandle};
pub use lan_share_core::ViewerFrame;
pub use viewer::{ViewerSession, ViewerSessionHandle};

use std::sync::Once;

/// WebRTC (aws-lc-rs) and axum-server (ring) both link rustls 0.23; pick one provider at process start.
pub fn init_rustls_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("install rustls CryptoProvider (ring)");
    });
}
