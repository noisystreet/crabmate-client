# crabmate-client

官方 **Client** 仓（路径 A）：Desktop Linux / Android Tauri 壳 + 共用 `crabmate-connect` + 业务 UI `frontend/`。  
连接兼容的 **`crabmate serve`**（本机或远程），**不** spawn / 内嵌 Agent 进程。

> **Server / 契约权威仓**：[noisystreet/CrabMate](https://github.com/noisystreet/CrabMate)（本机常见检出目录：`../crabmate_agent`）  
> **决策**：[client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)  
> **契约钉版本**：[client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)

## 目录

```text
.
├── crates/crabmate-connect/   # 连接页逻辑（本仓 path；勿再 path 回主仓）
├── desktop-tauri/             # Desktop Linux（Tauri 2）
├── mobile-tauri/              # Android（Tauri 2）
├── frontend/                  # 业务 UI（Leptos CSR + WASM；契约 git rev/tag）
├── scripts/                   # check / connect 同步 / Victauri
└── .github/workflows/         # CI（check + frontend wasm + desktop deb）
```

## 与主仓关系

| 项 | 现状 |
|----|------|
| 壳 + connect + 业务 UI | **本仓**维护 |
| 契约 crate | 主仓发布；UI 钉 git `rev` / `client-contract-vX.Y.Z`（见 [contract_pin.md](docs/design/contract_pin.md)） |
| Server `serve` | 主仓；本机或远程启动，壳不 spawn |
| 主仓 `frontend/` | Phase C 前可能仍双轨；**禁止**本仓 `path = "../crabmate_agent/..."` |

## Makefile

```bash
make help
make frontend           # trunk build → frontend/dist
make frontend-check     # wasm32 cargo check
make check              # 等同 scripts/check.sh（含 frontend）
make test
make desktop-dev        # 需已装 cargo-tauri ^2；另开终端跑 serve
make desktop-release    # 产出 .deb（默认同步本仓 frontend/dist）
make desktop-bin-release
make apk                # Android；默认不建 frontend
make clean
```

## 文档

| 文档 | 内容 |
|------|------|
| [AGENTS.md](./AGENTS.md) | Agent 约束、命令、文档同步规则 |
| [docs/TESTING.md](./docs/TESTING.md) | pre-commit / Victauri / CI |
| [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md) | 壳架构（路径 A） |
| [docs/design/shell_smoke_runbook.md](./docs/design/shell_smoke_runbook.md) | Desktop/Android 人工冒烟 |
| [docs/design/contract_pin.md](./docs/design/contract_pin.md) | 契约 git tag / rev 钉法 |
| [frontend/README.md](./frontend/README.md) | UI 构建（trunk） |

提交前：`pre-commit run --all-files` 或 `make check`。CI：`.github/workflows/ci.yml`（含 **frontend wasm** 与 **desktop release .deb**）。

## 快速开始（业务 UI + serve）

```bash
make frontend
# 另开终端：主仓或已安装的 crabmate
CM_WEB_STATIC_DIR="$PWD/frontend/dist" crabmate serve --host 127.0.0.1 --port 8080
```

## 快速开始（Desktop）

前置：本机或远程已启动 **`crabmate serve`**（默认 `http://127.0.0.1:8080/`）。

```bash
make frontend           # 可选：把 UI 同步进 desktop-tauri/dist
# 或：export CRABMATE_FRONTEND_DIST=$PWD/frontend/dist

make desktop-dev
# 或：cd desktop-tauri/src-tauri && cargo tauri dev
```

连接页填写服务器地址与可选 Web Bearer（**不是**模型 `API_KEY`）。

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
