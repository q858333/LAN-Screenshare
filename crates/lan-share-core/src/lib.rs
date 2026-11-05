use serde::{Deserialize, Serialize};

pub const DEFAULT_SIGNALING_PORT: u16 = 17890;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionRole {
    Host,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalingMessage {
    Hello { role: SessionRole, name: String },
    Offer { sdp: String },
    Answer { sdp: String },
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    HostInfo {
        display_name: String,
        video_width: u32,
        video_height: u32,
        has_audio: bool,
    },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSessionConfig {
    pub display_name: String,
    pub port: u16,
    pub include_audio: bool,
    pub target_fps: u32,
    pub max_width: u32,
}

impl Default for HostSessionConfig {
    fn default() -> Self {
        Self {
            display_name: hostname(),
            port: DEFAULT_SIGNALING_PORT,
            include_audio: true,
            target_fps: 30,
            max_width: 1280,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerConnectConfig {
    pub host_addr: String,
    pub port: u16,
}

impl Default for ViewerConnectConfig {
    fn default() -> Self {
        Self {
            host_addr: String::new(),
            port: DEFAULT_SIGNALING_PORT,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ViewerFrame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Desktop".into())
}
