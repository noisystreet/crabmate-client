# crabmate-client

[English](./README.md) | **简体中文**

<p align="center">
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/ci.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/code-complexity.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/code-complexity.yml/badge.svg?branch=main" alt="code-complexity" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/dependency-security.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/dependency-security.yml/badge.svg?branch=main" alt="Dependency security" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/actions/workflows/e2e-playwright.yml"><img src="https://github.com/noisystreet/crabmate-client/actions/workflows/e2e-playwright.yml/badge.svg?branch=main" alt="E2E Playwright" /></a>
  <br />
  <a href="https://github.com/noisystreet/crabmate-client/stargazers"><img src="https://img.shields.io/github/stars/noisystreet/crabmate-client?style=flat&logo=github" alt="GitHub stars" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/commits/main"><img src="https://img.shields.io/github/last-commit/noisystreet/crabmate-client?logo=github" alt="Last commit" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/issues"><img src="https://img.shields.io/github/issues/noisystreet/crabmate-client" alt="Issues" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/pulls"><img src="https://img.shields.io/github/issues-pr/noisystreet/crabmate-client" alt="Pull requests" /></a>
  <a href="https://github.com/noisystreet/crabmate-client/blob/main/LICENSE"><img src="https://img.shields.io/github/license/noisystreet/crabmate-client" alt="License" /></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust 1.85+" /></a>
</p>

官方 **Client** 仓（路径 A）：Desktop Linux / Android Tauri 壳 + 共用 `crabmate-connect` + 业务 UI `frontend/`。
连接兼容的 **`crabmate serve`**（本机或远程），**不** spawn / 内嵌 Agent 进程。

> **Server / 契约权威仓**：[noisystreet/CrabMate](https://github.com/noisystreet/CrabMate)（本机常见检出目录：`../crabmate_agent`）
> **决策**：[client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)
> **契约钉版本**：[client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)

## 目录

```text
.
├── crates/crabmate-client-api/ # 多端共用纯逻辑（URL / 鉴权 / 密钥槽 / 审批 / workspace / sessions / chat body / hash / health JSON；无 IO）
├── crates/crabmate-tool-card/  # 工具卡 compact/detail（W2 起本仓 path；勿再 git 钉 Server）
├── crates/crabmate-connect/   # 连接页逻辑（本仓 path；勿再 path 回主仓）
├── crates/crabmate-tui/       # 二进制 crabmate-tui（chat / repl / 全屏 tui；HTTP/SSE serve 客户端在 src/serve）
├── desktop-tauri/             # Desktop Linux（Tauri 2）
├── mobile-tauri/              # Android（Tauri 2）
├── web-host/                  # 二进制 crabmate-web（回环静态 UI 托管）
├── frontend/                  # 业务 UI（Leptos CSR + WASM；契约 crates.io crabmate）
├── e2e/                       # Playwright（浏览器 UI；mock SSE CI）
├── scripts/                   # check / connect 同步 / Victauri / Playwright
└── .github/workflows/         # CI（check + frontend 单测 + Playwright + desktop deb + Victauri nightly + dependency-security）
```

## 与主仓关系

| 项 | 现状 |
|----|------|
| 壳 + connect + 业务 UI | **本仓**维护 |
| 契约 crate | crates.io `crabmate` `0.6.0` + `protocol`（见 [contract_pin.md](docs/design/contract_pin.md)） |
| Server `serve` | 主仓；本机或远程启动，壳不 spawn |
| 主仓 `frontend/` / Playwright | UI 与 Playwright **在本仓**；主仓 Phase C 后无 `frontend/` 源码 |

## 快速开始

各目标共同前置：已启动 **`crabmate serve`**（默认纯 API）。当前 Server 默认已放行官方壳 Origin（`tauri://localhost`、`http://tauri.localhost`），Desktop/Android **不必**再设 `CM_WEB_CORS_ALLOWED_ORIGINS`。

```bash
# 终端 A — Server（壳路径不必 --with-web）
crabmate serve --host 127.0.0.1 --port 8080
```

### Desktop

```bash
# 终端 B — 本仓
make frontend           # prepare-sidecar 会同步进 desktop-tauri/dist
make desktop-dev
```

连接页填写服务器地址与可选 Web Bearer（**不是**模型 `API_KEY`）。连接成功后加载**包内** `index.html`，API 指向该 `serve`。工作区文件上传 / 下载 / 重命名与聊天附图见 [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md)。

### 系统浏览器里的 Web UI

不是 Tauri：本机回环静态服务打开默认浏览器。仍然**不是** `crabmate serve` — API 要另开，并在 CORS 里放行页面 Origin。

```bash
# 终端 A — API（放行 web-host Origin；Server ≥ v0.2.0 默认只放行 Tauri Origin）
CM_WEB_CORS_ALLOWED_ORIGINS=http://127.0.0.1:4173 crabmate serve --host 127.0.0.1 --port 8080

# 终端 B — 本仓
make web-release
sudo dpkg -i web-host/target/debian/crabmate-web_*.deb
crabmate-web --api-base http://127.0.0.1:8080
# 不安装时：
#   cargo run --release --manifest-path web-host/Cargo.toml -- --root frontend/dist --api-base http://127.0.0.1:8080
```

默认监听 `127.0.0.1:4173`。`--no-open` 跳过 `xdg-open`。Bearer：`--bearer` / `CM_WEB_API_BEARER_TOKEN`（纯浏览器会弱持久化到 `localStorage`）。`.deb` 会安装 **CrabMate Web** 菜单项，图标与 Desktop 壳相同。同一端口上再次启动会打开已有实例，而不是报错退出。

### Playwright / 浏览器 E2E

Playwright 跑在**客户端自托管**的 Web UI 上：纯 API `serve` + `crabmate-web`（回环静态服务，默认 `127.0.0.1:4173`）。脚本自动起两者，并经 `CM_WEB_CORS_ALLOWED_ORIGINS` 放行 web Origin；不再依赖 `serve --with-web`（Server 保持纯 API）。

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

### Android

```bash
make apk
# 或：./mobile-tauri/scripts/build-apk.sh
# 需要构建 UI 时：CM_MOBILE_BUILD_FRONTEND=1 make apk
```

Android 壳默认隐藏应用内底部状态栏；仍可从侧栏工具条重新开启。`/chat/stream` 期间的前台保活与审批通知见 [ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md)。

## 远程终端（crabmate-tui）

先启动 `crabmate serve`，再：

```bash
make tui
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 --bearer "$CM_WEB_API_BEARER_TOKEN" chat "你好"
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 repl   # 交互 REPL
./crates/crabmate-tui/target/debug/crabmate-tui --api-base http://127.0.0.1:8080 tui    # 全屏 TUI
```

bearer 鉴权但服务端未设模型 `API_KEY` 的 serve 会返回 `LLM_API_KEY_REQUIRED`；可用 `--llm-api-key` 自带模型密钥（或 `CM_API_KEY` / `CM_MODEL` / `CM_API_BASE`）。`--bearer` / `--llm-api-key` 缺省时会 **read-only 回退读取桌面壳已保存在同一系统钥匙串里的密钥**（`--no-keyring` 可关闭回退）。

发版包（仅二进制，无菜单图标、无配置文件）：

```bash
make tui-release
sudo dpkg -i crates/crabmate-tui/target/debian/crabmate-tui_*.deb
```

没有 Rust 工具链？同一个仅二进制的 `.deb` 也由 CI 构建并随每个 `v*` 的 GitHub Release 附上，可直接下载后 `dpkg -i` 安装。

键位、布局与设置面板见 [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md)。

## 个人云（远程纯 API）

公网只暴露 `api.…` → Caddy → 本机 `serve`（不要 `--with-web`）；壳用包内 UI 连接 `https://api.…/` + Bearer。步骤与勾选见 [`docs/design/personal_cloud_runbook.md`](docs/design/personal_cloud_runbook.md)。VPS/systemd/Caddy 见 Server [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md)。

## 文档

| 文档 | 内容 |
|------|------|
| [AGENTS.md](./AGENTS.md) | Agent 约束、命令、文档同步规则（仅英文） |
| [CHANGELOG.md](./CHANGELOG.md) | 用户/维护者可见变更（Keep a Changelog；仅英文） |
| [docs/TESTING.md](./docs/TESTING.md) | pre-commit / Victauri / CI |
| [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md) | 壳架构（路径 A） |
| [docs/design/shell_smoke_runbook.md](./docs/design/shell_smoke_runbook.md) | Desktop/Android 人工冒烟 |
| [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md) | 远程终端 crabmate-tui |
| [docs/design/client_shared_logic.md](./docs/design/client_shared_logic.md) | 多端共用纯逻辑抽取规划 |
| [docs/design/client_capability_matrix.md](./docs/design/client_capability_matrix.md) | Desktop / Android / Web / TUI 能力对照 |
| [docs/design/coding_agent_client.md](./docs/design/coding_agent_client.md) | 编程 Agent 客户端规划（审查 / 还原；Wave 1–3） |
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | 契约 git tag / rev 钉法 |
| [docs/design/tui_usability.md](./docs/design/tui_usability.md) | TUI 易用性规划（对齐 Desktop 聊天） |
| [docs/design/tui_settings_panel.md](./docs/design/tui_settings_panel.md) | TUI 设置面板（对齐 Desktop 设置） |
| [docs/design/display_crate_sink.md](./docs/design/display_crate_sink.md) | 展示 crate 下沉本仓（消费侧清单） |
| [docs/design/icon_unification.md](./docs/design/icon_unification.md) | 图标统一样式（共享 `Icon` 组件 / `--icon-*` token） |
| [docs/design/markdown_render_todo.md](./docs/design/markdown_render_todo.md) | Markdown 渲染待办 |
| [docs/design/chat_ui_todo.md](./docs/design/chat_ui_todo.md) | 对话界面待办（composer / transcript / 可访问性） |
| [docs/design/ui_issue_todo.md](./docs/design/ui_issue_todo.md) | UI 问题待办清单（壳层通用） |
| [frontend/README.md](./frontend/README.md) | UI 构建（trunk） |

提交前：`pre-commit run --all-files` 或 `make check`。CI：`.github/workflows/ci.yml`（含 **frontend wasm**、**frontend/TUI 单测**、**desktop / web / tui release .deb**）；依赖审计：`.github/workflows/dependency-security.yml`（`make dependency-security`）；Victauri 壳 E2E：nightly 工作流或 `./scripts/victauri-e2e.sh`。

## 开发约定

- `crabmate-connect`：本仓 `path = "../../crates/crabmate-connect"`；壳须启用 `features = ["tauri"]`（默认 feature 不含 Tauri）
- `frontend` 契约：git tag / `rev`；勿 `path` 回主仓
- 密钥边界与主仓 ADR §2.3 一致：跨 Origin 只认 Web Bearer + CORS

## 许可证

Apache-2.0（见 [LICENSE](./LICENSE)）
