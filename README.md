# crabmate-client

官方 **Client** 仓（路径 A）：Desktop Linux / Android Tauri 壳 + 共用 `crabmate-connect`。  
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
└── scripts/                   # 连接页同步等
```

## 与主仓关系（过渡）

| 项 | 现状 |
|----|------|
| 壳 + connect | **本仓**维护与发版 |
| 业务 UI（`frontend/`） | 仍在主仓；壳默认导航到远程 `serve` 托管的 UI |
| 契约 crate | 主仓发布；钉 `client-contract-vX.Y.Z`（见上文档） |
| 主仓同树副本 | Phase 4 前可短期双轨；**禁止**本仓 `path = "../crabmate_agent/..."` |

路径 A 终点：本仓自带业务 UI（或依赖版本化 UI 产物）+ API 基址连 `serve`。

## 文档

| 文档 | 内容 |
|------|------|
| [AGENTS.md](./AGENTS.md) | Agent 约束、命令、文档同步规则 |
| [docs/TESTING.md](./docs/TESTING.md) | pre-commit / Victauri |
| [docs/design/tauri_gui_mvp_design.md](./docs/design/tauri_gui_mvp_design.md) | 壳架构（路径 A） |
| [docs/design/shell_smoke_runbook.md](./docs/design/shell_smoke_runbook.md) | Desktop/Android 人工冒烟 |

提交前：`pre-commit run --all-files` 或 `bash scripts/check.sh`。

## 快速开始（Desktop）

前置：本机或远程已启动 **`crabmate serve`**（默认 `http://127.0.0.1:8080/`）。

```bash
# 可选：同步主仓已构建的 frontend/dist 进桌面 dist（deb / 调试）
# export CRABMATE_FRONTEND_DIST=../crabmate_agent/frontend/dist

cd desktop-tauri/src-tauri
cargo tauri dev
```

连接页填写服务器地址与可选 Web Bearer（**不是**模型 `API_KEY`）。

## 快速开始（Android）

```bash
./mobile-tauri/scripts/build-apk.sh
# 默认不构建 frontend；需要时：CM_MOBILE_BUILD_FRONTEND=1 CRABMATE_FRONTEND_DIR=../crabmate_agent/frontend
```

## 开发约定

- `crabmate-connect`：本仓 `path = "../../crates/crabmate-connect"`
- 勿依赖主仓未发布 path；契约用 git tag / `rev`
- 密钥边界与主仓 ADR §2.3 一致：跨 Origin 只认 Web Bearer + CORS

## 许可证

Apache-2.0（见 [LICENSE](./LICENSE)）
