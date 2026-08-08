# AGENTS.md

## Project Identity

- Project: `crabmate-client`
- Purpose: 官方 Client（Desktop Linux / Android Tauri 壳 + `crabmate-connect`）；连接已运行的 `crabmate serve`，不维护 Server
- Tech stack: Rust、Tauri 2、（过渡期）远程加载主仓 Web UI
- Target: Desktop Linux、Android；浏览器直连 UI 仍可由主仓 / 日后 UI 仓提供

## Directory Overview

```text
.
├── crates/crabmate-connect/
├── desktop-tauri/
├── mobile-tauri/
├── scripts/
└── docs/          # 可选；架构权威仍在主仓 design docs
```

## Hard Constraints

- **禁止** `path = "../crabmate_agent/..."` 或任何回主开发树的 path 依赖
- **禁止** 壳 spawn / 打包 `crabmate serve` sidecar
- 契约 crate（`crabmate-api-contract` / `crabmate-sse-protocol` 等）仅经 **git tag** `client-contract-vX.Y.Z` 或 `rev` 钉主仓
- `crabmate-connect` 仅本仓 path（`crates/crabmate-connect`）
- Web Bearer ≠ 模型 `API_KEY`；勿把模型密钥当 Web 鉴权
- 架构决策以主仓 `docs/design/client_shell_split.md` 为准；改发版边界前先读该 ADR

## Required Commands

```bash
# 同步连接页到 mobile/desktop dist
bash scripts/sync-tauri-connect-page.sh

# Desktop
cd desktop-tauri/src-tauri && cargo check
# Android APK（默认不构建 frontend）
./mobile-tauri/scripts/build-apk.sh
```

## Coding Rules

- 跟随既有壳代码风格；行为变更尽量有可跑通的手动冒烟路径
- 与 Server 的兼容只认已发布协议版本 + API 基址 / CORS（主仓 Phase 2）

## Documentation Rules

- 用户可见壳行为变更更新本仓 README
- 契约 / SSE / 密钥边界变更在主仓文档落地，本仓只链过去
