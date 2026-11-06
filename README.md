# LAN Screenshare

跨平台（**macOS + Windows**）局域网屏幕共享桌面客户端：分享端推流、观看端只读观看（系统声音开发中）。

仓库：<https://github.com/q858333/LAN-Screenshare>

## 功能概览（v0.1）

- 桌面客户端（Tauri 2），**分享端 / 观看端** 同一安装包内切换
- 局域网 **WebSocket / WSS 信令** + **WebRTC 视频**（H.264，桌面 **17890**，网页 **17891**）
- 观看端：**桌面 Canvas** 或 **浏览器**（HTTPS 网页，无需安装）
- **屏幕采集**：Windows / macOS（`scrap`；macOS 在专用线程内采集）
- **系统声音**：计划中（Windows WASAPI loopback、macOS ScreenCaptureKit）

不支持 Linux。

---

## 环境要求

| 项目 | 说明 |
|------|------|
| Rust | stable（`rustup default stable`） |
| Node.js | 18+ |
| macOS | Xcode Command Line Tools；需授予 **屏幕录制** 权限 |
| Windows | Visual Studio 生成工具（MSVC）；防火墙放行 **17890、17891** |

---

## 使用说明

### 1. 安装客户端

- **macOS**：打开 `.dmg`，将 **LAN Screenshare** 拖入「应用程序」。
- **Windows**：运行 `bundle\nsis\` 下的 `.exe` 安装程序。

首次在 macOS 打开若被拦截：**系统设置 → 隐私与安全性 → 仍要打开**。  
分享端还需：**系统设置 → 隐私与安全性 → 屏幕录制** 中允许本应用。

### 2. 网络条件

- 两台电脑连接 **同一局域网**（同一 Wi‑Fi 或同一网段有线）。
- 记下分享端显示的 IP（形如 `192.168.x.x`），**不要**加 `http://` 前缀。
- 若连接失败，检查防火墙是否放行 **TCP 17890、17891** 及 WebRTC 使用的 UDP 端口。

### 3. 分享端（被观看的电脑）

1. 打开 **LAN Screenshare**。
2. 切换到 **「分享端」**。
3. 点击 **「启动分享」**。
4. 界面会列出本机局域网地址，例如：`观看端连接：192.168.1.12:17890`。
5. 保持应用在前台或后台运行，直至不再需要分享后点击 **「停止」**。

### 4. 观看端（只看屏幕的电脑）

**方式 A：桌面客户端**

1. 打开 **LAN Screenshare**。
2. 切换到 **「观看端」**。
3. 在 **分享端 IP** 输入框填写分享端的 IP（仅 IP，不含端口；端口固定为 17890）。
4. 点击 **「连接」**。
5. 连接成功后下方 **Canvas** 应出现分享端画面；状态栏会显示分辨率等信息。

**方式 B：网页观看（适合 Windows，无需安装）**

**推荐（GitHub Pages，观看页托管在 GitHub）：**

1. 分享端电脑 **启动分享**。
2. 观看端用 Chrome / Edge 打开：  
   **https://q858333.github.io/LAN-Screenshare/**  
   （也可带参数：`?host=192.168.1.12&port=17891`）
3. 输入 **分享端 IP**，端口默认 **17891**，点 **连接**。
4. 浏览器连到分享端 `wss://IP:17891` 时若提示证书问题，按提示允许继续。

**备选（分享端本机直接提供页面）：**

打开 `https://192.168.1.12:17891`（IP 以分享端显示为准），会自动连接，无需填 IP。

网页仅 **观看**；信令与视频仍来自分享端桌面程序。分享端同时监听 **17890**（桌面）与 **17891**（网页 WSS）。

### 首次启用 GitHub Pages（仓库维护者）

1. 仓库 **Settings → Pages → Build and deployment → Source** 选 **GitHub Actions**。
2. 推送 `main` 后 **Actions → Deploy web viewer (GitHub Pages)** 跑绿即可访问上述 URL。

### 5. 常见问题

| 现象 | 处理建议 |
|------|----------|
| macOS 黑屏 / 无法抓屏 | 检查「屏幕录制」权限是否已勾选并重启应用 |
| 一直「未连接」 | 确认 IP 正确、同一局域网、分享端已「启动分享」 |
| 有连接无画面 | 稍等数秒；关闭双方防火墙试一次；查看终端日志（开发模式） |
| 暂无声音 | 当前版本仅视频，音频尚未发布 |

---

## 开发与本地调试

```bash
git clone https://github.com/q858333/LAN-Screenshare.git
cd LAN-Screenshare/apps/desktop
npm install
npm run tauri dev
```

日志（可选）：

```bash
RUST_LOG=info npm run tauri dev
```

---

## 打包步骤

### 通用准备

```bash
cd apps/desktop
npm ci
```

更换应用图标时（可选）：

```bash
# 准备 1024×1024 PNG，保存为 app-icon.png
npm run tauri icon app-icon.png
```

建议在仓库根目录统一 Rust 产物目录（与下文路径一致）：

```bash
export CARGO_TARGET_DIR=../../target   # macOS / Linux shell
# Windows PowerShell: $env:CARGO_TARGET_DIR="..\..\target"
```

---

### macOS 安装包（Apple Silicon / M 系列）

```bash
cd apps/desktop
npm run build:mac
# 等价于：tauri build --target aarch64-apple-darwin
```

**产物位置（相对仓库根目录）：**

| 类型 | 路径 |
|------|------|
| DMG | `target/aarch64-apple-darwin/release/bundle/dmg/LAN Screenshare_0.1.0_aarch64.dmg` |
| .app | `target/aarch64-apple-darwin/release/bundle/macos/LAN Screenshare.app` |

**Intel Mac** 将命令改为：

```bash
npm run build:mac:intel
# 或 --target x86_64-apple-darwin
```

对外分发建议：**Developer ID 签名 + Apple 公证（notarization）**，否则其他 Mac 可能难以直接打开。

---

### Windows 安装包（必须在 Windows 10/11 上执行）

```powershell
cd apps\desktop
npm ci
npm run build:win
```

**产物位置（相对仓库根目录）：**

```text
target\release\bundle\nsis\LAN Screenshare_*_x64-setup.exe
```

（具体文件名以 `nsis` 目录下生成为准。）

无法在 macOS 上可靠生成 Windows 安装包；请使用 Windows 本机或下方 CI。

---

### GitHub Actions 一键打多平台包

仓库已配置 `.github/workflows/release.yml`：

1. 推送标签，例如：`git tag v0.1.0 && git push origin v0.1.0`  
   或在 GitHub **Actions → Release installers → Run workflow**。
2. 在 **Releases** 中下载 Draft 里的 macOS DMG 与 Windows `.exe`。

---

## 仓库结构

```text
crates/lan-share-core     信令协议、会话配置
crates/lan-share-capture  抓屏 / 系统声（仅 macOS & Windows）
crates/lan-share-stream   Host / Viewer 信令与 WebRTC
apps/desktop              Tauri 客户端 UI
```

## 路线图

1. Windows WASAPI loopback 系统声
2. macOS ScreenCaptureKit 屏幕 + 音频
3. mDNS 自动发现、码率 / 分辨率设置
4. 多观看端
