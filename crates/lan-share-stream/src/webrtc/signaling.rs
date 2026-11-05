use lan_share_core::SignalingMessage;
use std::sync::Arc;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

pub fn wire_ice_candidates(
    pc: Arc<RTCPeerConnection>,
    outbound: tokio::sync::mpsc::UnboundedSender<SignalingMessage>,
) {
    pc.on_ice_candidate(Box::new(move |candidate| {
        let outbound = outbound.clone();
        Box::pin(async move {
            if let Some(c) = candidate {
                if let Ok(json) = c.to_json() {
                    let _ = outbound.send(SignalingMessage::IceCandidate {
                        candidate: json.candidate,
                        sdp_mid: json.sdp_mid,
                        sdp_mline_index: json.sdp_mline_index,
                    });
                }
            }
        })
    }));
}

pub async fn apply_remote_message(
    pc: &RTCPeerConnection,
    msg: SignalingMessage,
) -> anyhow::Result<Option<SignalingMessage>> {
    match msg {
        SignalingMessage::Offer { sdp } => {
            let offer = RTCSessionDescription::offer(sdp)?;
            pc.set_remote_description(offer).await?;
            let answer = pc.create_answer(None).await?;
            pc.set_local_description(answer.clone()).await?;
            Ok(Some(SignalingMessage::Answer {
                sdp: answer.sdp,
            }))
        }
        SignalingMessage::Answer { sdp } => {
            let answer = RTCSessionDescription::answer(sdp)?;
            pc.set_remote_description(answer).await?;
            Ok(None)
        }
        SignalingMessage::IceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => {
            let init = RTCIceCandidateInit {
                candidate,
                sdp_mid,
                sdp_mline_index,
                ..Default::default()
            };
            pc.add_ice_candidate(init).await?;
            Ok(None)
        }
        _ => Ok(None),
    }
}
