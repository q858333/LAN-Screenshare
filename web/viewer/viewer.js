const DEFAULT_WSS_PORT = 17891;

const statusEl = document.getElementById("status");
const video = document.getElementById("video");
const hostInput = document.getElementById("host-input");
const portInput = document.getElementById("port-input");
const connectBtn = document.getElementById("connect-btn");
const disconnectBtn = document.getElementById("disconnect-btn");

let ws;
let pc;

function setStatus(text) {
  statusEl.textContent = text;
}

function isGitHubPages() {
  const h = location.hostname;
  return h.endsWith("github.io") || h.endsWith("githubusercontent.com");
}

function embeddedPortalHost() {
  if (isGitHubPages()) return null;
  const port = Number(location.port || (location.protocol === "https:" ? 443 : 80));
  if (port === DEFAULT_WSS_PORT) return location.hostname;
  return null;
}

function createPeerConnection() {
  const conn = new RTCPeerConnection({
    iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
  });

  conn.ontrack = (ev) => {
    if (ev.streams[0]) {
      video.srcObject = ev.streams[0];
      video.muted = false;
      setStatus("正在播放视频流…");
    }
  };

  conn.onicecandidate = (ev) => {
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

  conn.onconnectionstatechange = () => {
    appendStatus(`WebRTC：${conn.connectionState}`);
  };

  return conn;
}

function appendStatus(line) {
  const prev = statusEl.textContent.trim();
  statusEl.textContent = prev ? `${prev}\n${line}` : line;
}

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
      appendStatus("已发送 Answer，等待媒体…");
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

function disconnect() {
  if (ws) {
    ws.close();
    ws = undefined;
  }
  if (pc) {
    pc.close();
    pc = undefined;
  }
  video.srcObject = null;
}

function connect() {
  const host = hostInput.value.trim();
  const port = Number(portInput.value) || DEFAULT_WSS_PORT;
  if (!host) {
    setStatus("请输入分享端 IP");
    return;
  }

  disconnect();
  pc = createPeerConnection();

  const wsUrl = `wss://${host}:${port}/ws`;
  setStatus(`正在连接 ${wsUrl} …`);

  ws = new WebSocket(wsUrl);
  ws.onopen = () => {
    appendStatus("信令已连接，等待 HostInfo…");
    ws.send(
      JSON.stringify({ type: "hello", role: "viewer", name: "WebViewer" }),
    );
  };
  ws.onmessage = (ev) => void handleMessage(ev.data);
  ws.onclose = () => appendStatus("信令连接已关闭");
  ws.onerror = () =>
    setStatus(
      `信令连接失败。\n请确认：\n1. 分享端已「启动分享」\n2. IP/端口正确（默认 ${DEFAULT_WSS_PORT}）\n3. 防火墙放行 TCP ${port}`,
    );
}

connectBtn.addEventListener("click", () => connect());
disconnectBtn.addEventListener("click", () => {
  disconnect();
  setStatus("已断开");
});

const params = new URLSearchParams(location.search);
const hostFromUrl = params.get("host");
const portFromUrl = params.get("port");
if (portFromUrl) portInput.value = portFromUrl;

const autoHost = hostFromUrl || embeddedPortalHost();
if (autoHost) {
  hostInput.value = autoHost;
  connect();
}
