# AGENTS.md

## Project Identity

- Project: `crabmate-client`
- Purpose: 官方 Client（Desktop Linux / Android Tauri 壳 + `crabmate-connect`）；连接已运行的 `crabmate serve`，**不**维护 Server
- Tech stack: Rust、Tauri 2
- Target: Desktop Linux、Android；浏览器直连 UI 过渡期仍可由 Server 托管

## Directory Overview

```text
.
├── crates/crabmate-connect/
├── desktop-tauri/
├── mobile-tauri/
├── scripts/                 # sync-connect、victauri-e2e、check.sh
├── docs/
│   ├── TESTING.md
│   └── design/
│       ├── tauri_gui_mvp_design.md
│       └── shell_smoke_runbook.md
├── AGENTS.md
└── .cursor/rules/
```

## Hard Constraints

- **禁止** `path = "../crabmate_agent/..."` 或任何回主开发树的 Cargo path 依赖
- **禁止** 壳 spawn / 打包 `crabmate serve` sidecar
- 契约 crate（若引入）仅经 **git tag** `client-contract-vX.Y.Z` 或 `rev` 钉主仓
- `crabmate-connect` 仅本仓 path（`crates/crabmate-connect`）
- Web Bearer ≠ 模型 `API_KEY`
- **拆仓决策 / 契约 / SSE / CORS** 权威在 Server 主仓；本仓文档只描述壳行为并链过去
- 临时草稿放 **`agent_space/`**（gitignore）；**勿**把 `agent_space/` 当作已提交文档引用源

## Required Commands

```bash
pre-commit run --all-files   # 提交前必跑（见 .cursor/rules/pre-commit-before-commit.mdc）
bash scripts/check.sh        # 无 pre-commit 时的替代（fmt/clippy/复杂度/禁 path）
bash scripts/check-no-main-path.sh
bash scripts/lizard-rust.sh
bash scripts/fn-param-ratchet.sh
bash scripts/fn-nloc-ratchet.sh
bash scripts/sync-tauri-connect-page.sh
cd desktop-tauri/src-tauri && cargo check
./scripts/victauri-e2e.sh all   # 需可用的 crabmate serve 二进制；默认不进 CI
```

## Documentation Rules

| 变更类型 | 更新哪里 |
|----------|----------|
| 壳用户可见行为 / 启动流程 | 本仓 `README.md`、`desktop-tauri/README.md` 或 `mobile-tauri/README.md` |
| 壳架构 / 生命周期 / 连接模型 | 本仓 `docs/design/tauri_gui_mvp_design.md` |
| 壳人工冒烟步骤 | 本仓 `docs/design/shell_smoke_runbook.md` |
| Victauri / pre-commit / CI 命令 | 本仓 `docs/TESTING.md` |
| 契约钉 tag | 本仓 `docs/design/contract_pin.md`；策略权威在主仓 |
| SSE / CORS / Bearer / API 基址 / 契约 semver | **Server 主仓** `docs/`；本仓只加链接 |
| 路径 A 进度勾选 | 主仓 `docs/design/client_shell_split_todo.md` |

## Coding Rules

- 跟随既有壳代码风格
- 提交说明：Conventional Commits + 中英双语 subject（见 `.cursor/rules/conventional-commits.mdc`）
- **禁止** `git commit --no-verify`

## Server 文档索引（只读链接）

- [client_shell_split.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_shell_split.md)
- [client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)
- [client_turn_smoke_runbook.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_turn_smoke_runbook.md)（宿主 + 契约；壳步骤以本仓 `shell_smoke_runbook` 为准）
- [SSE协议.md](https://github.com/noisystreet/CrabMate/blob/main/docs/SSE协议.md)
- [配置说明.md](https://github.com/noisystreet/CrabMate/blob/main/docs/配置说明.md)
