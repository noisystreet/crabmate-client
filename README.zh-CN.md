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
├── crates/crabmate-client-api/ # 多端共用纯逻辑（URL / 鉴权 / 密钥槽 / 审批 / workspace / sessions / chat body；无 IO）
├── crates/crabmate-tool-card/  # 工具卡 compact/detail（W2 起本仓 path；勿再 git 钉 Server）
├── crates/crabmate-connect/   # 连接页逻辑（本仓 path；勿再 path 回主仓）
├── crates/crabmate-tui-core/  # 远程终端 HTTP/SSE 核心
├── crates/crabmate-tui/       # 二进制 crabmate-tui（P3：chat / repl + 斜杠）
├── crates/crabmate-web-host/  # 二进制 crabmate-web（回环静态 UI 托管）
├── desktop-tauri/             # Desktop Linux（Tauri 2）
├── mobile-tauri/              # Android（Tauri 2）
├── frontend/                  # 业务 UI（Leptos CSR + WASM；契约 crates.io crabmate）
├── e2e/                       # Playwright（浏览器 UI；mock SSE CI）
├── scripts/                   # check / connect 同步 / Victauri / Playwright
└── .github/workflows/         # CI（check + frontend 单测 + Playwright + desktop deb + Victauri nightly + dependency-security）
```

## 与主仓关系

| 项 | 现状 |
|----|------|
| 壳 + connect + 业务 UI | **本仓**维护 |
| 契约 crate | crates.io `crabmate` `0.4.0` + `protocol`（见 [contract_pin.md](docs/design/contract_pin.md)） |
| Server `serve` | 主仓；本机或远程启动，壳不 spawn |
| 主仓 `frontend/` / Playwright | UI 与 Playwright **在本仓**；主仓 Phase C 后无 `frontend/` 源码 |

## Makefile

```bash
make help
make frontend           # trunk build → frontend/dist
make frontend-check     # wasm32 cargo check
make check              # 等同 scripts/check.sh（含 frontend）
make dependency-security # cargo audit + cargo deny（各 workspace；不进 pre-commit）
make test
make desktop-dev        # 需已装 cargo-tauri ^2；另开终端跑 serve
make desktop-release    # 产出 crabmate-desktop_*.deb（自动 trunk --release UI，勿用 debug dist）
make desktop-bin-release
make web-release        # 产出 crabmate-web_*.deb（trunk --release + 回环静态服务，系统浏览器）
make apk                # Android；默认不建 frontend
make tui                # 构建 crabmate-tui（远程终端）
make tui-release        # 产出 crabmate-tui_*.deb（仅二进制；无图标、无配置）
make clean
```

## 远程终端（P3）

先启动 `crabmate serve`，再：

```bash
make tui
./crates/crabmate-tui/target/debug/crabmate-tui \
  --api-base http://127.0.0.1:8080 \
  --bearer "$CM_WEB_API_BEARER_TOKEN" \
  chat "你好"

# 交互 REPL（会话 id 跨轮续聊；非白名单命令可 TTY 审批，或加 --yes 自动 allow_once）
./crates/crabmate-tui/target/debug/crabmate-tui \
  --api-base http://127.0.0.1:8080 \
  repl
# repl 内：/help · /workspace [path] · /conv list|new|use <id>
```

管道把消息喂给 `chat`（无 argv）会读尽 stdin，后续审批无法再读决策，应加 **`--yes`**，或把消息写在参数里：

```bash
echo "你好" | crabmate-tui --api-base http://127.0.0.1:8080 --yes chat
crabmate-tui --api-base http://127.0.0.1:8080 chat "你好"
```

设计见 [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md)。发版包（仅二进制，无菜单图标、无配置文件）：

```bash
make tui-release
sudo dpkg -i crates/crabmate-tui/target/debian/crabmate-tui_*.deb
crabmate-tui --api-base http://127.0.0.1:8080 repl
```

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
| [frontend/README.md](./frontend/README.md) | UI 构建（trunk） |

提交前：`pre-commit run --all-files` 或 `make check`。CI：`.github/workflows/ci.yml`（含 **frontend wasm**、**frontend/TUI 单测**、**desktop / web / tui release .deb**）；依赖审计：`.github/workflows/dependency-security.yml`（`make dependency-security`）；Victauri 壳 E2E：nightly 工作流或 `./scripts/victauri-e2e.sh`。

## 快速开始（Desktop）

前置：已启动 **`crabmate serve`**（默认纯 API）。当前 Server 默认已放行官方壳 Origin（`tauri://localhost`、`http://tauri.localhost`），Desktop/Android **不必**再设 `CM_WEB_CORS_ALLOWED_ORIGINS`。

```bash
# 终端 A — Server（壳路径不必 --with-web）
crabmate serve --host 127.0.0.1 --port 8080

# 终端 B — 本仓
make frontend           # prepare-sidecar 会同步进 desktop-tauri/dist
make desktop-dev
```

连接页填写服务器地址与可选 Web Bearer（**不是**模型 `API_KEY`）。连接成功后加载**包内** `index.html`，API 指向该 `serve`。

工作区侧栏（或 IDE 文件树）可把**本机文件**拖到文件夹上上传（先确认；支持文本与二进制）。走 **`PUT /workspace/file/raw`**，需要当前 Server 构建（不是 crates.io / git **`v0.4.0`** 的 `serve`）。目标已存在时会再问是否覆盖；取消覆盖则**中止本批剩余文件**。聊天输入框附图（`POST /upload`，选文件 / 拖放 / 粘贴）发送后会显示在用户气泡里；壳用 Web Bearer 拉取。点已加载的图可放大。粘贴仅在剪贴板有图且**没有**非空 `text/plain` 时当附图（网页复制不会抢走正文）。文件在服务端临时目录，可能被清理（气泡会显示无法加载的占位）。

## 快速开始（系统浏览器里的 Web UI）

不是 Tauri：本机回环静态服务打开默认浏览器。仍然**不是** `crabmate serve` — API 要另开，并在 CORS 里放行页面 Origin。

```bash
# 终端 A — API
crabmate serve --host 127.0.0.1 --port 8080
# 放行 web-host Origin（Server ≥ v0.2.0 默认只放行 Tauri Origin）：
#   CM_WEB_CORS_ALLOWED_ORIGINS=http://127.0.0.1:4173 crabmate serve …

# 终端 B — 本仓
make web-release
sudo dpkg -i crates/crabmate-web-host/target/debian/crabmate-web_*.deb
crabmate-web --api-base http://127.0.0.1:8080
# 不安装时：
#   cargo run --release --manifest-path crates/crabmate-web-host/Cargo.toml -- --root frontend/dist --api-base http://127.0.0.1:8080
```

默认监听 `127.0.0.1:4173`。`--no-open` 跳过 `xdg-open`。Bearer：`--bearer` / `CM_WEB_API_BEARER_TOKEN`（纯浏览器会弱持久化到 `localStorage`）。`.deb` 会安装 **CrabMate Web** 菜单项，图标与 Desktop 壳相同。同一端口上再次启动会打开已有实例，而不是报错退出。

## 个人云（远程纯 API）

公网只暴露 `api.…` → Caddy → 本机 `serve`（不要 `--with-web`）；壳用包内 UI 连接 `https://api.…/` + Bearer。步骤与勾选见 [`docs/design/personal_cloud_runbook.md`](docs/design/personal_cloud_runbook.md)。VPS/systemd/Caddy 见 Server [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md)。

## 快速开始（Playwright / 浏览器 E2E）

Playwright 跑在**客户端自托管**的 Web UI 上：纯 API `serve` + `crabmate-web`（回环静态服务，默认 `127.0.0.1:4173`）。脚本自动起两者，并经 `CM_WEB_CORS_ALLOWED_ORIGINS` 放行 web Origin；不再依赖 `serve --with-web`（Server 保持纯 API）。

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

## 快速开始（Android）

```bash
make apk
# 或：./mobile-tauri/scripts/build-apk.sh
# 需要构建 UI 时：CM_MOBILE_BUILD_FRONTEND=1 make apk
```

Android 壳默认隐藏应用内底部状态栏；仍可从侧栏工具条重新开启。

`/chat/stream` 进行中时，壳会拉起前台服务（通知「对话进行中」），降低按 Home / 锁屏后 WebView 被系统冻结或杀进程的概率。服务端下发命令审批时，同一通知升级为「等待命令审批」（命令预览会截断）。点按通知回到应用内现有审批弹窗。Android 13+ 首次发送会请求通知权限；拒绝后保活/审批提醒不可用（状态栏提示）。部分厂商省电策略仍可能杀进程。

详见 [ADR-0002](docs/adr/0002-android-approval-notification-foreground-keepalive.md)。

## 开发约定

- `crabmate-connect`：本仓 `path = "../../crates/crabmate-connect"`；壳须启用 `features = ["tauri"]`（默认 feature 不含 Tauri）
- `frontend` 契约：git tag / `rev`；勿 `path` 回主仓
- 密钥边界与主仓 ADR §2.3 一致：跨 Origin 只认 Web Bearer + CORS

## 许可证

Apache-2.0（见 [LICENSE](./LICENSE)）
