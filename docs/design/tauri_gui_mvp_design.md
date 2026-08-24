# 基于现有 Web UI 的 Tauri GUI 设计（MVP → 路径 A）

> **权威仓**：本文件在 **`crabmate-client`**。Server 主仓仅保留指针（见 CrabMate `docs/design/tauri_gui_mvp_design.md`）。  
> **发版边界**：[路径 A ADR](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)

## 1. 目标（当前）

官方 Desktop / Android Client：

- **不** spawn / 内嵌 `crabmate serve`
- WebView + 连接页（`crabmate-connect`）连本机或远程已运行的 `serve`
- 业务 UI 源码在本仓 `frontend/`；运行时由 `serve --with-web` 经 `CM_WEB_STATIC_DIR` 托管（或壳同步 `frontend/dist`；Server 默认纯 API）

MVP 历史验收（部分已废弃）：

| 原目标 | 现状 |
|--------|------|
| 启动壳自动拉起后端 | **废弃** — 用户自行起 `serve` |
| 连接页 → `serve` UI | **保留** |
| 关窗回收后端进程 | **废弃** — 壳不管理 `serve` 生命周期 |
| 默认 loopback | **serve 侧**策略；壳可连非回环 |
| 单实例 | **保留**（Desktop） |

## 2. 架构方案（当前）

薄壳 + 远程权威：

1. 用户启动 `crabmate serve`（本机 / LAN / VPS）
2. Tauri 打开连接页（预填 `CM_DESKTOP_SUGGESTED_URL` 或默认 `http://127.0.0.1:8080/`）
3. 探测 `GET /health`；可选 Web Bearer 经 `#cm_web_api_bearer=` 交接
4. 导航到该 `serve` 的 UI（静态资源通常来自本仓 `frontend/dist` + `CM_WEB_STATIC_DIR` + **`--with-web`**）

历史「Web 壳 + 本地后端进程 / ready JSON」仅作考古；壳**不再**依赖 `--desktop-ready-json`。该 CLI 仍留在 Server（工具/脚本可解析）；是否改名/废弃见主仓 Phase 4。

## 3. 代码落地范围

### 3.1 Server（主仓，勿在本仓改）

- `--desktop-ready-json`：`bind` 成功后 stdout 一行 `web_ready` JSON
- HTTP/SSE、CORS、Web Bearer：见主仓 `docs/SSE协议.md`、`docs/配置说明.md`

### 3.2 Client（本仓）

| 路径 | 职责 |
|------|------|
| `desktop-tauri/src-tauri/src/main.rs` | 不 spawn `serve`；连接页或 E2E 直连 `CM_DESKTOP_SERVE_URL` |
| `desktop-tauri/src-tauri/src/desktop_lifecycle.rs` | 单实例、托盘、最小化隐藏 |
| `desktop-tauri/scripts/prepare-sidecar.sh` | 同步 connect（及可选遗留 splash）；默认本仓 `frontend/dist`；`CM_PREPARE_SKIP_FRONTEND=1` 可跳过 |
| `desktop-tauri/scripts/before-desktop-build.sh` | release 构建：`trunk --release` + prepare；拒绝 debug 大 WASM |
| `frontend/` | 业务 UI（Leptos CSR）；契约 crates.io `crabmate` 0.5.0 + `protocol` |
| `mobile-tauri/` | Android 薄壳（流式前台保活 / 审批通知见 [ADR-0002](../adr/0002-android-approval-notification-foreground-keepalive.md)） |
| `crates/crabmate-connect/` | 探测 / Bearer / 钥匙串 |

## 4. 开发启动

```bash
# 终端 A — Server 主仓（如 ../crabmate_agent）；默认纯 API，托管 UI 须 --with-web
CM_WEB_STATIC_DIR=../crabmate-client/frontend/dist \
  cargo run -- serve --with-web

# 终端 B — 本仓
cd desktop-tauri/src-tauri
cargo tauri dev
```

```bash
cargo install tauri-cli --version "^2"
```

## 5. 安全基线

- Web Bearer ≠ 模型 `API_KEY`
- 跨 Origin：主仓 CORS 精确白名单 + Bearer（见主仓 Phase 2）
- 非回环监听策略由 `serve` 配置决定；壳不放宽鉴权

## 6. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 误以为壳会起后端 | README / 连接页文案；E2E 须自备 `serve` |
| monorepo E2E 假设 | `scripts/victauri-e2e.sh` 经 `CM_DESKTOP_BACKEND_BIN` / PATH / 同级主仓解析 `crabmate` |
| 无托盘环境窗口不可恢复 | 托盘失败时保留普通最小化 |
| Android 后台杀进程 / 丢工具审批 | [ADR-0002](../adr/0002-android-approval-notification-foreground-keepalive.md)：attach 时 `dataSync` FGS + 审批通知；`visibilitychange` 续传仍为退路 |
