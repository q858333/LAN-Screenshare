const statusEl = document.getElementById("status");
const video = document.getElementById("video");

function setStatus(text) {
  statusEl.textContent = text;
}

const pc = new RTCPeerConnection({
  iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
});

pc.ontrack = (ev) => {
  if (ev.streams[0]) {
    video.srcObject = ev.streams[0];
    video.muted = false;
    setStatus("正在播放视频流…");
  }
};

pc.onicecandidate = (ev) => {
  if (!ev.candidate || !ws || ws.readyState !== WebSocket.OPEN) return;
  ws.send(
    JSON.stringify({
      type: "ice_candidate",
      candidate: ev.candidate.candidate,
      sdp_mid: ev.candidate.sdpMid,
      sdp_mline_index: ev.candidate.sdpMLineIndex,
    }),
  );
};

pc.onconnectionstatechange = () => {
  setStatus(`WebRTC 状态：${pc.connectionState}`);
};

const wsScheme = location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsScheme}//${location.host}/ws`;
let ws;

async function handleMessage(raw) {
  const msg = JSON.parse(raw);
  switch (msg.type) {
    case "host_info":
      setStatus(
        `已连接分享端：${msg.display_name}\n${msg.video_width}x${msg.video_height} · audio=${msg.has_audio}`,
      );
      break;
    case "offer":
      await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      ws.send(JSON.stringify({ type: "answer", sdp: answer.sdp }));
      setStatus((s) => `${s}\n已发送 Answer，等待媒体…`);
      break;
    case "ice_candidate":
      if (!msg.candidate) break;
      try {
        await pc.addIceCandidate({
          candidate: msg.candidate,
          sdpMid: msg.sdp_mid ?? undefined,
          sdpMLineIndex: msg.sdp_mline_index ?? undefined,
        });
      } catch (e) {
        console.warn("ICE candidate", e);
      }
      break;
    case "error":
      setStatus(`错误：${msg.message}`);
      break;
    default:
      break;
  }
}

function connect() {
  ws = new WebSocket(wsUrl);
  ws.onopen = () => {
    setStatus("信令已连接，等待 HostInfo…");
    ws.send(
      JSON.stringify({ type: "hello", role: "viewer", name: "WebViewer" }),
    );
  };
  ws.onmessage = (ev) => void handleMessage(ev.data);
  ws.onclose = () => setStatus("信令连接已关闭");
  ws.onerror = () => setStatus("信令连接失败，请确认分享端已启动且使用 HTTPS 打开本页");
}

connect();
