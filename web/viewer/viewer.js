const DEFAULT_WSS_PORT = 17891;
const MAX_DIAG_LINES = 400;

const statusEl = document.getElementById("status");
const video = document.getElementById("video");
const hostInput = document.getElementById("host-input");
const portInput = document.getElementById("port-input");
const connectBtn = document.getElementById("connect-btn");
const disconnectBtn = document.getElementById("disconnect-btn");
const certBtn = document.getElementById("cert-btn");
const diagToggle = document.getElementById("diag-toggle");
const diagPanel = document.getElementById("diag-panel");
const diagView = document.getElementById("diag-view");
const diagCopy = document.getElementById("diag-copy");
const diagClear = document.getElementById("diag-clear");

let ws;
let pc;
const diagLines = [];

function diagLog(level, msg) {
  const t = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  const line = `[${t}] [${level}] ${msg}`;
  diagLines.push(line);
  if (diagLines.length > MAX_DIAG_LINES) {
    diagLines.splice(0, diagLines.length - MAX_DIAG_LINES);
  }
  if (!diagView) return;
  diagView.textContent = diagLines.join("\n");
  const follow = document.getElementById("diag-follow");
  if (follow?.checked) {
    diagView.scrollTop = diagView.scrollHeight;
  }
}

function renderDiag() {
  if (diagView) diagView.textContent = diagLines.join("\n");
}

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
  diagLog("info", `打开证书信任页 https://${host}:${port}/`);
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
    diagLog("info", `ontrack: streams=${ev.streams.length}`);
    if (ev.streams[0]) {
      video.srcObject = ev.streams[0];
      video.muted = false;
      setStatus("正在播放视频流…");
    }
  };

  conn.onicecandidate = (ev) => {
    if (!ev.candidate) {
      diagLog("debug", "ICE 本地候选 gathering 结束");
      return;
    }
    const c = ev.candidate.candidate;
    const short = c.length > 72 ? `${c.slice(0, 72)}…` : c;
    diagLog("debug", `ICE 本地 → 分享端: ${short}`);
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
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
    const s = conn.connectionState;
    diagLog(s === "failed" ? "error" : "info", `WebRTC connectionState=${s}`);
    appendStatus(`WebRTC：${s}`);
    if (s === "failed") {
      appendStatus(
        "媒体连接失败：请用 Chrome/Edge；系统防火墙允许本应用 UDP；本机试 127.0.0.1；跨设备试局域网 IP。",
      );
    }
  };
  conn.oniceconnectionstatechange = () => {
    diagLog("info", `ICE iceConnectionState=${conn.iceConnectionState}`);
    appendStatus(`ICE：${conn.iceConnectionState}`);
  };
  conn.onicegatheringstatechange = () => {
    diagLog("debug", `ICE gatheringState=${conn.iceGatheringState}`);
  };
  conn.onsignalingstatechange = () => {
    diagLog("debug", `signalingState=${conn.signalingState}`);
  };

  return conn;
}

function appendStatus(line) {
  const prev = statusEl.textContent.trim();
  statusEl.textContent = prev ? `${prev}\n${line}` : line;
}

async function handleMessage(raw) {
  let msg;
  try {
    msg = JSON.parse(raw);
  } catch (e) {
    diagLog("error", `信令 JSON 解析失败: ${e}`);
    return;
  }
  diagLog("debug", `信令 ← ${msg.type}`);
  switch (msg.type) {
    case "host_info":
      setStatus(
        `已连接分享端：${msg.display_name}\n${msg.video_width}x${msg.video_height} · audio=${msg.has_audio}`,
      );
      diagLog(
        "info",
        `HostInfo ${msg.display_name} ${msg.video_width}x${msg.video_height} audio=${msg.has_audio}`,
      );
      break;
    case "offer":
      diagLog("info", `收到 Offer (sdp ${msg.sdp?.length ?? 0} 字符)`);
      await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      ws.send(JSON.stringify({ type: "answer", sdp: answer.sdp }));
      diagLog("info", `已发送 Answer (sdp ${answer.sdp?.length ?? 0} 字符)`);
      appendStatus("已发送 Answer，等待媒体…");
      break;
    case "ice_candidate":
      if (!msg.candidate) break;
      {
        const c = msg.candidate;
        const short = c.length > 72 ? `${c.slice(0, 72)}…` : c;
        diagLog("debug", `ICE 远端 ← 分享端: ${short}`);
      }
      try {
        await pc.addIceCandidate({
          candidate: msg.candidate,
          sdpMid: msg.sdp_mid ?? undefined,
          sdpMLineIndex: msg.sdp_mline_index ?? undefined,
        });
      } catch (e) {
        diagLog("warn", `addIceCandidate 失败: ${e}`);
        console.warn("ICE candidate", e);
      }
      break;
    case "error":
      diagLog("error", `分享端错误: ${msg.message}`);
      setStatus(`错误：${msg.message}`);
      break;
    default:
      diagLog("debug", `未处理消息 type=${msg.type}`);
      break;
  }
}

function disconnect() {
  diagLog("info", "断开连接");
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
  diagLog("info", `连接信令 ${wsUrl} (页面=${location.href})`);

  ws = new WebSocket(wsUrl);
  ws.onopen = () => {
    diagLog("info", "WebSocket 已打开");
    appendStatus("信令已连接，等待 HostInfo…");
    ws.send(
      JSON.stringify({ type: "hello", role: "viewer", name: "WebViewer" }),
    );
    diagLog("debug", "信令 → hello");
  };
  ws.onmessage = (ev) => void handleMessage(ev.data);
  ws.onclose = (ev) => {
    diagLog("warn", `WebSocket 关闭 code=${ev.code} reason=${ev.reason || "(无)"}`);
    appendStatus("信令连接已关闭");
  };
  ws.onerror = () => {
    diagLog("error", "WebSocket 错误（多为证书未信任或 TCP 不通）");
    setStatus(
      `信令连接失败（多为证书未信任或网络不通）。\n` +
        `1. 若从 GitHub Pages 打开：先点「打开 HTTPS 并信任」，在新页接受证书后再连接\n` +
        `2. 分享端已「启动分享」，IP 为分享端界面显示的局域网地址（端口 ${DEFAULT_WSS_PORT}，勿用 17890）\n` +
        `3. 观看端与分享端同一局域网；分享端防火墙放行 TCP ${port} 与 WebRTC UDP`,
    );
  };
}

diagToggle?.addEventListener("click", () => {
  const open = diagPanel?.hidden !== false;
  if (diagPanel) diagPanel.hidden = !open;
  diagToggle?.setAttribute("aria-expanded", open ? "true" : "false");
  diagToggle.textContent = open ? "诊断日志（已展开）" : "诊断日志";
  if (open) renderDiag();
});

diagCopy?.addEventListener("click", () => {
  const text = diagLines.join("\n");
  if (!text) return;
  void navigator.clipboard.writeText(text).then(
    () => diagLog("info", "诊断日志已复制到剪贴板"),
    () => diagLog("warn", "复制失败，请手动选中日志文本"),
  );
});

diagClear?.addEventListener("click", () => {
  diagLines.length = 0;
  renderDiag();
  diagLog("info", "诊断缓冲已清空");
});

diagLog(
  "info",
  `页面就绪 · ${isGitHubPages() ? "GitHub Pages" : "本地/门户"} · ${navigator.userAgent.split(" ").slice(-2).join(" ")}`,
);

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
