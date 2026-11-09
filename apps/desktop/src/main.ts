import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Role = "host" | "viewer" | "logs";

interface HostStatus {
  running: boolean;
  addresses: string[];
  port: number;
  web_port: number;
  web_viewer_urls: string[];
}

interface ViewerProgress {
  generation: number;
  step: number;
  total: number;
  title: string;
  detail: string;
  phase: "running" | "done" | "error";
}

interface ViewerStatus {
  connected: boolean;
  host_info: string | null;
  last_error: string | null;
  progress: ViewerProgress | null;
}

const CONNECT_STEP_TITLES = [
  "检查地址",
  "连接信令",
  "建立媒体通道",
  "获取分享端信息",
  "协商视频",
  "接收画面",
];

let connectBusy = false;
let activeGeneration: number | null = null;
const pendingProgress: ViewerProgress[] = [];
let progressUnlisten: UnlistenFn | undefined;
let progressHideTimer: number | undefined;

interface ViewerFrameEvent {
  width: number;
  height: number;
  jpeg_base64: string;
}

interface LogPathsState {
  log_dir: string;
  log_file: string;
}

const app = document.querySelector<HTMLDivElement>("#app")!;

let role: Role = "host";
let pollTimer: number | undefined;
let logPollTimer: number | undefined;
let frameUnlisten: UnlistenFn | undefined;
let logsFollowTail = true;

function render() {
  app.innerHTML = `
    <h1>LAN 屏幕共享 <span class="badge">macOS / Windows</span></h1>
    <p class="sub">桌面客户端 · 仅观看 · 局域网 WebRTC 视频（H.264）</p>
    <div class="tabs">
      <button id="tab-host" class="${role === "host" ? "active" : ""}">分享端</button>
      <button id="tab-viewer" class="${role === "viewer" ? "active" : ""}">观看端</button>
      <button id="tab-logs" class="${role === "logs" ? "active" : ""}">日志</button>
    </div>
    <div id="panel"></div>
  `;

  document.getElementById("tab-host")!.onclick = () => {
    role = "host";
    stopPoll();
    stopLogPoll();
    void stopFrameListener();
    render();
    void refreshStatus();
  };
  document.getElementById("tab-viewer")!.onclick = () => {
    role = "viewer";
    stopPoll();
    stopLogPoll();
    render();
    void refreshStatus();
    void startFrameListener();
  };
  document.getElementById("tab-logs")!.onclick = () => {
    role = "logs";
    stopPoll();
    void stopFrameListener();
    render();
    startLogPoll();
    void refreshLogs(true);
  };

  renderPanel();
  if (role === "host" || role === "viewer") {
    void refreshStatus();
    startPoll();
  }
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
  } else if (role === "viewer") {
    panel.innerHTML = `
      <div class="card">
        <label for="host-ip">分享端 IP</label>
        <input id="host-ip" type="text" placeholder="例如 192.168.1.12" autocomplete="off" />
        <p class="field-hint">不用填写端口。未写端口时自动使用 17890。只有分享端改过端口，才写成 192.168.1.12:17890。</p>
        <div class="row">
          <button id="viewer-connect">连接</button>
          <button id="viewer-disconnect" class="secondary">断开</button>
        </div>
        <div class="status" id="viewer-status">未连接</div>
        <canvas id="viewer-canvas" width="1280" height="720" style="width:100%;margin-top:1rem;background:#000;border-radius:8px"></canvas>
      </div>
    `;
    const connectButton = document.getElementById("viewer-connect") as HTMLButtonElement;
    connectButton.disabled = connectBusy;
    connectButton.onclick = () => void connectViewer();
    document.getElementById("viewer-disconnect")!.onclick = () => void disconnectViewer();
  } else {
    panel.innerHTML = `
      <div class="card">
        <strong>诊断日志</strong>
        <p class="sub" style="margin:0.35rem 0 0.75rem">分享、WebRTC、网页门户等输出会记录到此处与本地日志文件。出问题时请复制此页内容。</p>
        <div class="status" id="log-paths">正在加载路径…</div>
        <div class="row">
          <button type="button" id="log-refresh">刷新</button>
          <button type="button" id="log-copy" class="secondary">复制全部</button>
          <button type="button" id="log-clear" class="secondary">清空缓冲</button>
          <button type="button" id="log-reveal" class="secondary">在访达中显示日志文件</button>
        </div>
        <label class="log-follow">
          <input type="checkbox" id="log-follow" checked />
          自动滚动到最新
        </label>
        <pre id="log-view" class="log-view" aria-live="polite"></pre>
      </div>
    `;
    document.getElementById("log-refresh")!.onclick = () => void refreshLogs(true);
    document.getElementById("log-copy")!.onclick = () => void copyLogs();
    document.getElementById("log-clear")!.onclick = () => void clearLogs();
    document.getElementById("log-reveal")!.onclick = () => void revealLogFile();
    const follow = document.getElementById("log-follow") as HTMLInputElement;
    follow.onchange = () => {
      logsFollowTail = follow.checked;
    };
    void refreshLogPaths();
    void refreshLogs(true);
  }
}

async function refreshLogPaths() {
  const el = document.getElementById("log-paths");
  if (!el) return;
  try {
    const p = await invoke<LogPathsState>("log_paths");
    el.textContent = `文件：${p.log_file}\n目录：${p.log_dir}`;
  } catch (e) {
    el.textContent = `无法读取日志路径：${e}`;
  }
}

async function refreshLogs(scrollToEnd: boolean) {
  const view = document.getElementById("log-view");
  if (!view) return;
  try {
    const lines = await invoke<string[]>("get_app_logs", { limit: 800 });
    view.textContent = lines.length ? lines.join("\n") : "（暂无日志，请先操作分享/观看）";
    if (scrollToEnd && logsFollowTail) {
      view.scrollTop = view.scrollHeight;
    }
  } catch (e) {
    view.textContent = `读取日志失败：${e}`;
  }
}

async function copyLogs() {
  const view = document.getElementById("log-view");
  if (!view?.textContent) return;
  try {
    await navigator.clipboard.writeText(view.textContent);
  } catch {
    // ignore
  }
}

async function clearLogs() {
  await invoke("clear_app_logs");
  await refreshLogs(false);
}

async function revealLogFile() {
  try {
    await invoke("reveal_log_file");
  } catch (e) {
    const view = document.getElementById("log-view");
    if (view) view.textContent = `${view.textContent}\n[错误] ${e}`;
  }
}

async function startHost() {
  await invoke("start_host_session");
  await refreshStatus();
  if (role === "logs") await refreshLogs(true);
}

async function stopHost() {
  await invoke("stop_host_session");
  await refreshStatus();
}

async function connectViewer() {
  const ip = (document.getElementById("host-ip") as HTMLInputElement).value.trim();
  pendingProgress.length = 0;
  activeGeneration = null;
  setConnectBusy(true);
  if (!ip) {
    applyProgress(localProgress(1, "error", "请输入分享端 IP。不用填写端口，默认是 17890。"));
    return;
  }
  applyProgress(
    localProgress(1, "running", `正在检查 ${ip}。不用填写端口，默认使用 17890。`),
  );
  let generation: number;
  try {
    generation = await invoke<number>("connect_viewer", { hostAddr: ip });
  } catch (e) {
    applyProgress(localProgress(1, "error", String(e)));
    const status = document.getElementById("viewer-status");
    if (status) status.textContent = String(e);
    return;
  }
  activeGeneration = generation;
  const queued = pendingProgress.splice(0);
  for (const progress of queued) {
    if (progress.generation === generation) applyProgress(progress);
  }
  await refreshStatus();
}

async function disconnectViewer() {
  activeGeneration = -1;
  pendingProgress.length = 0;
  setConnectBusy(false);
  hideConnectModal();
  try {
    await invoke("disconnect_viewer");
  } catch (e) {
    const status = document.getElementById("viewer-status");
    if (status) status.textContent = String(e);
  }
  await refreshStatus();
}

function localProgress(
  step: number,
  phase: ViewerProgress["phase"],
  detail: string,
): ViewerProgress {
  return {
    generation: 0,
    step,
    total: CONNECT_STEP_TITLES.length,
    title: CONNECT_STEP_TITLES[step - 1] ?? "检查地址",
    detail,
    phase,
  };
}

function setConnectBusy(busy: boolean) {
  connectBusy = busy;
  const button = document.getElementById("viewer-connect") as HTMLButtonElement | null;
  if (button) button.disabled = busy;
}

function ensureConnectModal() {
  if (document.getElementById("connect-modal")) return;
  const modal = document.createElement("div");
  modal.id = "connect-modal";
  modal.className = "modal-backdrop";
  modal.hidden = true;
  modal.innerHTML = `
    <div class="modal-card" role="dialog" aria-modal="true" aria-labelledby="connect-modal-heading">
      <h2 id="connect-modal-heading">正在连接</h2>
      <p id="connect-modal-count" class="modal-count">第 1 / 6 步</p>
      <div class="modal-bar" aria-hidden="true"><div id="connect-modal-fill"></div></div>
      <ol id="connect-modal-steps" class="step-list"></ol>
      <p id="connect-modal-detail" class="modal-detail"></p>
      <p class="field-hint">可以先收起，连接会继续。要取消请点观看页的「断开」。</p>
      <div class="row">
        <button type="button" id="connect-modal-close" class="secondary">收起</button>
      </div>
    </div>
  `;
  document.body.appendChild(modal);
  document.getElementById("connect-modal-close")!.addEventListener("click", () => {
    hideConnectModal();
  });
}

function hideConnectModal() {
  if (progressHideTimer !== undefined) {
    window.clearTimeout(progressHideTimer);
    progressHideTimer = undefined;
  }
  const modal = document.getElementById("connect-modal");
  if (modal) modal.hidden = true;
}

function applyProgress(progress: ViewerProgress) {
  if (progressHideTimer !== undefined) {
    window.clearTimeout(progressHideTimer);
    progressHideTimer = undefined;
  }
  ensureConnectModal();
  const modal = document.getElementById("connect-modal")!;
  modal.hidden = false;
  const finished = progress.phase === "done" && progress.step >= progress.total;
  const failed = progress.phase === "error";
  const heading = document.getElementById("connect-modal-heading")!;
  heading.textContent = failed ? "连接失败" : finished ? "已连接" : "正在连接";
  document.getElementById("connect-modal-count")!.textContent =
    `第 ${progress.step} / ${progress.total} 步`;
  document.getElementById("connect-modal-detail")!.textContent = progress.detail;
  const fill = document.getElementById("connect-modal-fill")!;
  let ratio = (Math.min(progress.step, progress.total) - (progress.phase === "running" ? 0.4 : 0)) / progress.total;
  if (finished) ratio = 1;
  fill.style.width = `${Math.round(Math.max(0.08, Math.min(1, ratio)) * 100)}%`;
  fill.className = failed ? "error" : finished ? "done" : "";
  const list = document.getElementById("connect-modal-steps")!;
  list.replaceChildren();
  for (let i = 1; i <= progress.total; i += 1) {
    const item = document.createElement("li");
    let mark = "○";
    if (failed && i === progress.step) {
      item.className = "error";
      mark = "✕";
    } else if (i < progress.step || (progress.phase === "done" && i === progress.step)) {
      item.className = "done";
      mark = "✓";
    } else if (i === progress.step) {
      item.className = "active";
      mark = "●";
    }
    item.textContent = `${mark}  ${i}. ${CONNECT_STEP_TITLES[i - 1] ?? progress.title}`;
    list.appendChild(item);
  }
  const closeButton = document.getElementById("connect-modal-close")!;
  closeButton.textContent = failed || finished ? "关闭" : "收起";
  setConnectBusy(!(failed || finished));
  const status = document.getElementById("viewer-status");
  if (status && role === "viewer") {
    status.textContent = failed
      ? progress.detail
      : `第 ${progress.step} / ${progress.total} 步：${progress.title}\n${progress.detail}`;
  }
  if (finished) {
    progressHideTimer = window.setTimeout(() => hideConnectModal(), 1200);
  }
}

function onViewerProgress(progress: ViewerProgress) {
  if (activeGeneration === null) {
    pendingProgress.push(progress);
    return;
  }
  if (progress.generation !== activeGeneration) return;
  applyProgress(progress);
}

async function ensureProgressListener() {
  if (progressUnlisten) return;
  try {
    progressUnlisten = await listen<ViewerProgress>("viewer-progress", (event) => {
      onViewerProgress(event.payload);
    });
  } catch (e) {
    console.warn("进度事件监听不可用", e);
  }
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
  } else if (role === "viewer") {
    const st = await invoke<ViewerStatus>("viewer_status");
    const el = document.getElementById("viewer-status");
    if (!el) return;
    if (st.progress?.phase === "error") {
      el.textContent = st.last_error ?? st.progress.detail;
      return;
    }
    if (st.progress?.phase === "running") {
      const head = st.host_info ? `${st.host_info}\n` : "";
      el.textContent = `${head}第 ${st.progress.step} / ${st.progress.total} 步：${st.progress.title}\n${st.progress.detail}`;
      return;
    }
    if (!st.connected) {
      if (connectBusy) return;
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

function startLogPoll() {
  stopLogPoll();
  logPollTimer = window.setInterval(() => void refreshLogs(false), 2000);
}

function stopLogPoll() {
  if (logPollTimer !== undefined) {
    window.clearInterval(logPollTimer);
    logPollTimer = undefined;
  }
}

void ensureProgressListener();
render();
