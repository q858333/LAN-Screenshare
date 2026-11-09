use anyhow::Result;
use std::sync::Arc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice::mdns::MulticastDnsMode;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::RTCPeerConnection;

pub fn mime_type_h264() -> &'static str {
    MIME_TYPE_H264
}

pub async fn build_peer_connection() -> Result<Arc<RTCPeerConnection>> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media_engine)?;

    let mut setting_engine = SettingEngine::default();
    // 多个本机地址不能当成同一个 1:1 NAT 映射，否则建连会直接失败。
    // 关闭 mDNS，让观看端拿到真实局域网地址。
    setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build();

    // LAN 直连：不依赖公网 STUN，避免本机/局域网 ICE 被打公网地址带偏
    let config = RTCConfiguration {
        ice_servers: vec![],
        ..Default::default()
    };

    Ok(Arc::new(api.new_peer_connection(config).await?))
}
