# 契约钉版本（壳仓 / 官方 UI）

> **权威发版策略**：Server 主仓 [`client_contract_versioning.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)  
> **展示 crate 下沉**：Server [`client_display_crate_sink.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_display_crate_sink.md)；本仓勾选 [`display_crate_sink.md`](./display_crate_sink.md)  
> **单包 crates.io**：Server [`crates_io_single_package.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/crates_io_single_package.md) — **S3 已钉** `crabmate` + `features = ["protocol"]`；crates.io `0.4.0` 仍待 S4/S5。  
> **UI 迁出计划**：[`frontend_migrate_plan.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/frontend_migrate_plan.md) Phase B（本仓已迁入 `frontend/`）  
> **本仓职责**：消费侧钉法 + 禁止 path 回主开发树。

## 现状

| 依赖 | 本仓做法 |
|------|----------|
| `crabmate-connect` | **本仓** `path = "../../crates/crabmate-connect"`（已迁入；勿再 git+path 回主仓旧路径） |
| 业务 UI（`frontend/`） | **本仓**；线契约一条 `crabmate`（`protocol`），见 `frontend/Cargo.toml` |
| 远程终端（`crabmate-tui-core`） | 同一 `crabmate` + `protocol`（**不要**开 `server`） |
| 壳二进制本身 | 暂不直接依赖主仓契约 crate（只经 WebView 加载 UI + HTTP/SSE） |

本地门禁：`bash scripts/check-no-main-path.sh`（需 `rg` 或 `grep`；CI 装 ripgrep）。

当前 UI / tui 钉点（与 `frontend/Cargo.toml`、`crates/crabmate-tui-core/Cargo.toml` 一致）：

| 项 | 值 |
|----|-----|
| 源码迁入自 | 主仓 `eb0048bf…`（见 `frontend/SOURCE.md`） |
| 契约 | 单包 **`crabmate`** `features = ["protocol"]`，git **`rev = 27c1fd3a…`**（Server #855 合入后的 `main`） |
| Playwright `serve` checkout | 同一 **`rev`** |
| lock | 提交 `frontend/Cargo.lock`、`crates/crabmate-tui-core/Cargo.lock`、`crates/crabmate-tool-card/Cargo.lock` |

壳打包 UI 同步：`make desktop-release` / `before-desktop-build.sh` 默认 **`trunk build --release`** 再同步本仓 `frontend/dist`（拒绝 debug 大 WASM）；`CM_PREPARE_SKIP_FRONTEND=1` 或 `CRABMATE_FRONTEND_DIST=-` 跳过；同级主仓回落需 `CRABMATE_ALLOW_SIBLING_FRONTEND=1`。

## 钉主仓契约

```toml
crabmate = { git = "https://github.com/noisystreet/CrabMate", rev = "27c1fd3ae2f770a9ac115f567f6ad140b70631c9", package = "crabmate", default-features = false, features = ["protocol"] }
```

`use` 只走 `crabmate::cm_types`、`cm_sse_protocol`、`cm_api_contract`、`cm_turn_layout`、`cm_display_rules`、`cm_chat_export`。不要 `crabmate::sse` / `crabmate::types`（仅 Server `server` feature 别名）。

`crabmate-tool-card` 为本仓 path（W2：`crates/crabmate-tool-card`）；勿再 git 钉 Server 该包。

crates.io **`0.4.0`** 发布后改为 `version = "0.4.0"`（可去掉 git）。未升级的 Client 仍可钉产品 tag **`v0.3.0`** / `client-contract-v0.2.0` 的旧多包名。

`crabmate-types` 不再由 `frontend` **直接**依赖（W1：网关预设表在 `frontend/src/client_llm_presets.rs`）。

开发期可用更新 `rev`；打产品 tag 前主仓须绿：`bash scripts/check-client-contract.sh`。

**禁止**：`path = "../crabmate_agent/..."` 或任何回主开发树的 Cargo path。

## 冒烟清单

1. `bash scripts/check-no-main-path.sh`
2. `bash scripts/check.sh`（含 frontend wasm check / clippy）
3. `make frontend` → `trunk build`；产物在 `frontend/dist`
4. 外部 `crabmate serve --with-web` + `CM_WEB_STATIC_DIR=$PWD/frontend/dist`，或 `make prepare-sidecar` 后桌面壳一轮对话（[`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）
