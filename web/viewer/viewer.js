const DEFAULT_WSS_PORT = 17891;

const statusEl = document.getElementById("status");
const video = document.getElementById("video");
const hostInput = document.getElementById("host-input");
const portInput = document.getElementById("port-input");
const connectBtn = document.getElementById("connect-btn");
const disconnectBtn = document.getElementById("disconnect-btn");
const certBtn = document.getElementById("cert-btn");

let ws;
let pc;

function setStatus(text) {
  statusEl.textContent = text;
}

function currentHostPort() {
  const host = hostInput.value.trim();
  const port = Number(portInput.value) || DEFAULT_WSS_PORT;
  return { host, port };
}

function openCertTrustPage() {
  const { host, port } = currentHostPort();
  if (!host) {
    setStatus("请先填写分享端 IP，再打开证书信任页面");
    return;
  }
  window.open(`https://${host}:${port}/`, "_blank", "noopener,noreferrer");
  setStatus(
    `请在新标签页接受证书警告后关闭该页，再回到这里点「连接」。\n目标：https://${host}:${port}/`,
  );
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
    iceServers: [],
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
    if (conn.connectionState === "failed") {
      appendStatus(
        "媒体连接失败：请用 Chrome/Edge；系统防火墙允许本应用 UDP；本机试 127.0.0.1；跨设备试局域网 IP。",
      );
    }
  };
  conn.oniceconnectionstatechange = () => {
    appendStatus(`ICE：${conn.iceConnectionState}`);
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
  const { host, port } = currentHostPort();
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
      `信令连接失败（多为证书未信任或网络不通）。\n` +
        `1. 若从 GitHub Pages 打开：先点「打开 HTTPS 并信任」，在新页接受证书后再连接\n` +
        `2. 分享端已「启动分享」，IP 为分享端界面显示的局域网地址（端口 ${DEFAULT_WSS_PORT}，勿用 17890）\n` +
        `3. 观看端与分享端同一局域网；分享端防火墙放行 TCP ${port} 与 WebRTC UDP`,
    );
}

connectBtn.addEventListener("click", () => connect());
certBtn?.addEventListener("click", () => openCertTrustPage());
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
