use crate::net;
use anyhow::Result;
use std::sync::Arc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::ice_transport::ice_server::RTCIceServer;
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
    let mut ice_ips = vec!["127.0.0.1".to_string()];
    ice_ips.extend(net::list_lan_ipv4());
    setting_engine.set_nat_1to1_ips(ice_ips, RTCIceCandidateType::Host);

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
