import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Role = "host" | "viewer";

interface HostStatus {
  running: boolean;
  addresses: string[];
  port: number;
  web_port: number;
  web_viewer_urls: string[];
}

interface ViewerStatus {
  connected: boolean;
  host_info: string | null;
  last_error: string | null;
}

interface ViewerFrameEvent {
  width: number;
  height: number;
  jpeg_base64: string;
}

const app = document.querySelector<HTMLDivElement>("#app")!;

let role: Role = "host";
let pollTimer: number | undefined;
let frameUnlisten: UnlistenFn | undefined;

function render() {
  app.innerHTML = `
    <h1>LAN 屏幕共享 <span class="badge">macOS / Windows</span></h1>
    <p class="sub">桌面客户端 · 仅观看 · 局域网 WebRTC 视频（H.264）</p>
    <div class="tabs">
      <button id="tab-host" class="${role === "host" ? "active" : ""}">分享端</button>
      <button id="tab-viewer" class="${role === "viewer" ? "active" : ""}">观看端</button>
    </div>
    <div id="panel"></div>
  `;

  document.getElementById("tab-host")!.onclick = () => {
    role = "host";
    stopPoll();
    void stopFrameListener();
    render();
    void refreshStatus();
  };
  document.getElementById("tab-viewer")!.onclick = () => {
    role = "viewer";
    stopPoll();
    render();
    void refreshStatus();
    void startFrameListener();
  };

  renderPanel();
  void refreshStatus();
  startPoll();
  if (role === "viewer") {
    void startFrameListener();
  }
}

function renderPanel() {
  const panel = document.getElementById("panel")!;
  if (role === "host") {
    panel.innerHTML = `
      <div class="card">
        <strong>开始分享本机屏幕</strong>
        <p class="sub" style="margin:0.5rem 0 0">需屏幕录制权限。系统声音：Windows WASAPI / macOS ScreenCaptureKit 待接入。</p>
        <div class="row">
          <button id="host-start">启动分享</button>
          <button id="host-stop" class="secondary">停止</button>
        </div>
        <div class="status" id="host-status">尚未启动</div>
      </div>
    `;
    document.getElementById("host-start")!.onclick = () => void startHost();
    document.getElementById("host-stop")!.onclick = () => void stopHost();
  } else {
    panel.innerHTML = `
      <div class="card">
        <label for="host-ip">分享端 IP 地址</label>
        <input id="host-ip" type="text" placeholder="例如 192.168.1.12" />
        <div class="row">
          <button id="viewer-connect">连接</button>
          <button id="viewer-disconnect" class="secondary">断开</button>
        </div>
        <div class="status" id="viewer-status">未连接</div>
        <canvas id="viewer-canvas" width="1280" height="720" style="width:100%;margin-top:1rem;background:#000;border-radius:8px"></canvas>
      </div>
    `;
    document.getElementById("viewer-connect")!.onclick = () => void connectViewer();
    document.getElementById("viewer-disconnect")!.onclick = () => void disconnectViewer();
  }
}

async function startHost() {
  await invoke("start_host_session");
  await refreshStatus();
}

async function stopHost() {
  await invoke("stop_host_session");
  await refreshStatus();
}

async function connectViewer() {
  const ip = (document.getElementById("host-ip") as HTMLInputElement).value.trim();
  await invoke("connect_viewer", { hostAddr: ip });
  await refreshStatus();
}

async function disconnectViewer() {
  await invoke("disconnect_viewer");
  await refreshStatus();
}

async function refreshStatus() {
  if (role === "host") {
    const st = await invoke<HostStatus>("host_status");
    const el = document.getElementById("host-status");
    if (!el) return;
    if (!st.running) {
      el.textContent = "尚未启动";
      return;
    }
    const desktop = st.addresses.map((a) => `桌面观看端：${a}:${st.port}`).join("\n");
    const web = st.web_viewer_urls.length
      ? st.web_viewer_urls.map((u) => `网页观看：${u}`).join("\n")
      : `网页观看：https://<分享端IP>:${st.web_port}`;
    el.textContent = `分享中\n${desktop}\n${web}`;
  } else {
    const st = await invoke<ViewerStatus>("viewer_status");
    const el = document.getElementById("viewer-status");
    if (!el) return;
    if (!st.connected) {
      el.textContent = st.last_error ?? "未连接";
      return;
    }
    el.textContent = [st.host_info, st.last_error].filter(Boolean).join("\n") || "已连接，等待视频…";
  }
}

async function startFrameListener() {
  await stopFrameListener();
  frameUnlisten = await listen<ViewerFrameEvent>("viewer-frame", (ev) => {
    const canvas = document.getElementById("viewer-canvas") as HTMLCanvasElement | null;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { width, height, jpeg_base64 } = ev.payload;
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    const img = new Image();
    img.onload = () => ctx.drawImage(img, 0, 0, width, height);
    img.src = `data:image/jpeg;base64,${jpeg_base64}`;
  });
}

async function stopFrameListener() {
  if (frameUnlisten) {
    await frameUnlisten();
    frameUnlisten = undefined;
  }
}

function startPoll() {
  stopPoll();
  pollTimer = window.setInterval(() => void refreshStatus(), 1500);
}

function stopPoll() {
  if (pollTimer !== undefined) {
    window.clearInterval(pollTimer);
    pollTimer = undefined;
  }
}

render();
