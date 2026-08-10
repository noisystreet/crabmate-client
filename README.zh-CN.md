# crabmate-client

[English](./README.md) | **简体中文**

官方 **Client** 仓（路径 A）：Desktop Linux / Android Tauri 壳 + 共用 `crabmate-connect` + 业务 UI `frontend/`。  
连接兼容的 **`crabmate serve`**（本机或远程），**不** spawn / 内嵌 Agent 进程。

> **Server / 契约权威仓**：[noisystreet/CrabMate](https://github.com/noisystreet/CrabMate)（本机常见检出目录：`../crabmate_agent`）  
> **决策**：[client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)  
> **契约钉版本**：[client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)

## 目录

```text
.
├── crates/crabmate-connect/   # 连接页逻辑（本仓 path；勿再 path 回主仓）
├── crates/crabmate-tui-core/  # 远程终端 HTTP/SSE 核心
├── crates/crabmate-tui/       # 二进制 crabmate-tui（P3：chat / repl + 斜杠）
├── desktop-tauri/             # Desktop Linux（Tauri 2）
├── mobile-tauri/              # Android（Tauri 2）
├── frontend/                  # 业务 UI（Leptos CSR + WASM；契约 git rev/tag）
├── e2e/                       # Playwright（浏览器 UI；mock SSE CI）
├── scripts/                   # check / connect 同步 / Victauri / Playwright
└── .github/workflows/         # CI（check + frontend + Playwright + desktop deb）
```

## 与主仓关系

| 项 | 现状 |
|----|------|
| 壳 + connect + 业务 UI | **本仓**维护 |
| 契约 crate | 主仓发布；UI 钉 git `rev` / `client-contract-vX.Y.Z`（见 [contract_pin.md](docs/design/contract_pin.md)） |
| Server `serve` | 主仓；本机或远程启动，壳不 spawn |
| 主仓 `frontend/` / Playwright | UI 与 Playwright **在本仓**；主仓 Phase C 后无 `frontend/` 源码 |

## Makefile

```bash
make help
make frontend           # trunk build → frontend/dist
make frontend-check     # wasm32 cargo check
make check              # 等同 scripts/check.sh（含 frontend）
make test
make desktop-dev        # 需已装 cargo-tauri ^2；另开终端跑 serve
make desktop-release    # 产出 crabmate-desktop_*.deb（自动 trunk --release UI，勿用 debug dist）
make desktop-bin-release
make apk                # Android；默认不建 frontend
make tui                # 构建 crabmate-tui（远程终端）
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

设计见 [docs/design/remote_cli_tui.md](./docs/design/remote_cli_tui.md)。

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
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | 契约 git tag / rev 钉法 |
| [frontend/README.md](./frontend/README.md) | UI 构建（trunk） |

提交前：`pre-commit run --all-files` 或 `make check`。CI：`.github/workflows/ci.yml`（含 **frontend wasm** 与 **desktop release .deb**）。

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

## 个人云（远程纯 API）

公网只暴露 `api.…` → Caddy → 本机 `serve`（不要 `--with-web`）；壳用包内 UI 连接 `https://api.…/` + Bearer。步骤与勾选见 [`docs/design/personal_cloud_runbook.md`](docs/design/personal_cloud_runbook.md)。VPS/systemd/Caddy 见 Server [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md)。

## 快速开始（浏览器 + serve 托管 UI）

仅用于 Playwright / 同 Origin 浏览器调试（非 Desktop/Android 主路径）：

```bash
make frontend
CM_WEB_STATIC_DIR="$PWD/frontend/dist" crabmate serve --with-web --host 127.0.0.1 --port 8080
```

## 快速开始（Android）

```bash
make apk
# 或：./mobile-tauri/scripts/build-apk.sh
# 需要构建 UI 时：CM_MOBILE_BUILD_FRONTEND=1 make apk
```

## 开发约定

- `crabmate-connect`：本仓 `path = "../../crates/crabmate-connect"`
- `frontend` 契约：git tag / `rev`；勿 `path` 回主仓
- 密钥边界与主仓 ADR §2.3 一致：跨 Origin 只认 Web Bearer + CORS

## 许可证

Apache-2.0（见 [LICENSE](./LICENSE)）
