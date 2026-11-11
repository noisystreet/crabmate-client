# 契约钉版本（壳仓 / 官方 UI）

> **权威发版策略**：Server 主仓 [`client_contract_versioning.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)  
> **展示 crate 下沉**：Server [`client_display_crate_sink.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_display_crate_sink.md)；本仓勾选 [`display_crate_sink.md`](./display_crate_sink.md)  
> **单包 crates.io**：Server [`crates_io_single_package.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/crates_io_single_package.md) — 正式口径钉 crates.io **`crabmate`** + `features = ["protocol"]`；**当前暂回过渡期 git rev 钉 `client-contract-v0.3.0` → `817157e0`**：crates.io 最新仍是 `0.8.0`（2026-11-01），不含 `context_budget`，Server 发含它的版本后即回 semver（见下）。
> **UI 迁出计划**：[`frontend_migrate_plan.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/frontend_migrate_plan.md) Phase B（本仓已迁入 `frontend/`）  
> **本仓职责**：消费侧钉法 + 禁止 path 回主开发树。

## 现状

| 依赖 | 本仓做法 |
|------|----------|
| `crabmate-connect` | **本仓** `path = "../../crates/crabmate-connect"`（已迁入；勿再 git+path 回主仓旧路径）。壳启用 `features = ["tauri"]` |
| 业务 UI（`frontend/`） | **本仓**；线契约一条 `crabmate`（`protocol`），见 `frontend/Cargo.toml` |
| 远程终端（`crabmate-tui` `serve/` 模块，原 `crabmate-tui-core`） | 同一 `crabmate` + `protocol`（**不要**开 `server`） |
| 壳二进制本身 | 暂不直接依赖主仓契约 crate（只经 WebView 加载 UI + HTTP/SSE） |

本地门禁：`bash scripts/check-no-main-path.sh`（需 `rg` 或 `grep`；CI 装 ripgrep）。

当前 UI / tui 钉点（与 `frontend/Cargo.toml`、`crates/crabmate-tui/Cargo.toml` 一致）：

| 项 | 值 |
|----|-----|
| 源码迁入自 | 主仓 `eb0048bf…`（见 `frontend/SOURCE.md`） |
| 契约 | **git rev**：`crabmate` **`817157e0`** + `features = ["protocol"]`（= 契约 tag `client-contract-v0.3.0`） |
| Playwright `serve` checkout | git tag **`v0.8.0`**（Server 最新产品 tag；它早于契约提交，故 E2E 覆盖不到 `context_budget`，等下一个 `v*` tag 再升） |
| lock | 提交 `frontend/Cargo.lock`、`crates/crabmate-tui/Cargo.lock`、`crates/crabmate-tool-card/Cargo.lock` |

`PUT /workspace/file/raw`（工作区本机文件拖放上传）、**`GET /workspace/file/download`**（侧栏保存文件到本机）与 **`GET /workspace/dir/archive`**（保存文件夹为 zip）、**`POST /workspace/file/move`**（树内重命名文件）是 **HTTP 路由**，不在 crates.io `protocol` 面里。官方 UI 对接 **当前 Server 源码（契约 ≥ `817157e0`）与旧的 `v0.8.0` serve**；Playwright 默认 checkout **`v0.8.0`**。`GET /workspace/file/raw` 仍仅为聊天图片（png/jpg/jpeg/webp/gif）。

壳打包 UI 同步：`make desktop-release` / `before-desktop-build.sh` 默认 **`trunk build --release`** 再同步本仓 `frontend/dist`（拒绝 debug 大 WASM）；`CM_PREPARE_SKIP_FRONTEND=1` 或 `CRABMATE_FRONTEND_DIST=-` 跳过；同级主仓回落需 `CRABMATE_ALLOW_SIBLING_FRONTEND=1`。

## 钉主仓契约

```toml
# 正式口径（默认）：crates.io semver
crabmate = { version = "<semver>", default-features = false, features = ["protocol"] }

# 过渡期（当前生效）：Server 未发布所需版本时钉不可变 rev；tag 可移动，禁止
# 语义上等于契约 tag client-contract-v0.3.0（tag 对象 f02273e5，peel 到本 commit）
crabmate = { git = "https://github.com/noisystreet/CrabMate", rev = "817157e029e65dcb72c7e215c1994febe791cb05", default-features = false, features = ["protocol"] }
```

`use` 只走 `crabmate::cm_types`、`cm_sse_protocol`、`cm_api_contract`、`cm_turn_layout`、`cm_display_rules`、`cm_chat_export`。不要 `crabmate::sse` / `crabmate::types`（仅 Server `server` feature 别名）。

`crabmate-tool-card` 为本仓 path（W2：`crates/crabmate-tool-card`）；勿再 git 钉 Server 该包。

**当前（2026-11-11 起）暂回过渡期钉法**：`rev = 817157e0…`，语义上等于契约 tag `client-contract-v0.3.0`（Server #1347 S3b / PR #1351）。理由：crates.io 最新仍是 `0.8.0`，不含 `StatusShellView.context_budget`（可选）与「`tiktokenPromptTokens.used_input_tokens` 恒为 tiktoken 估算、上游 usage 只走 `provider_input_tokens`」这条语义变更。客户端改动只有一处测试夹具（`frontend/src/app_prefs.rs` 的 `StatusShellView` 字面量补 `context_budget: None`），业务代码仍读 `used_input_tokens`；状态栏分母**保持会话窗口**、不改用 `context_budget.max_input_tokens`（窗口分母在会话内恒定，见 `crates/crabmate-client-api/src/context_axis.rs` 头部论证，Server 设计稿 `context_usage_single_source.md` 同结论）。代价是 CI 多拉一次 Server 整仓（可缓存 `~/.cargo/git`，本地可用 `CARGO_NET_GIT_FETCH_WITH_CLI=true` + git `insteadOf` 重写）。Server 发布含该契约的 crates.io 版本后，按正式口径回到 semver。

crates.io **`crabmate`** 为正式渠道，2026-11-01 至 2026-11-11 期间本仓钉的是 **`0.8.0`**（含 permission-levels 与空闲态会话状态写入口 `cm_api_contract/session_state.rs` / `POST /conversation/state`），过渡期的 git rev 钉法到此结束。历史留档：过渡 rev `630cd3b0` 相对上一钉点（`f0a1bc06`）只多了三件事，都在会话状态写入口这条线上：新增该路由（`3d080382`）、缺 `conversation_id` 回 400 而非 422（`021d4606`）、库写失败回 500 而不是伪装成 404（`630cd3b0`）——最后一条尤其重要：客户端按 `error.code` 分流 404 的两义，若库故障回 404，客户端会把「写失败、可重试」显示成「会话不存在，请重新绑定」。升级时先只验证契约可见性（`cargo check --manifest-path crates/crabmate-client-api/Cargo.toml` + 一句 `use crabmate::cm_api_contract::session_state::ConversationStateView;`），再动业务代码；卡在那一步通常是 pin 没生效或 feature 未覆盖，**不要**误判成「服务端没实现」。换回 semver 后，各包 lock 由 `cargo fetch` / `cargo check` 自动把 `git+…` 源改写成 registry 源（本次已改 7 份 lock），仍须一并提交。未升级的 Client 仍可钉 crates.io / git **`v0.4.0` / `v0.5.0` / `v0.5.1` / `v0.5.2` / `v0.6.0` / `v0.7.0`**；更旧版本可继续使用产品 tag **`v0.3.0`** / `client-contract-v0.2.0` 的旧多包名。

（过渡期）钉 git 依赖会让 CI 多拉一次整仓（建议缓存 `~/.cargo/git`）；本地可用 `CARGO_NET_GIT_FETCH_WITH_CLI=true` + git `insteadOf` 把该 URL 重写到本地仓库，避免每次整仓 clone（`Cargo.toml` / lock 里仍是 https 源，不影响可分发）。当前又回到过渡期钉法，这两条重新适用。

`crabmate-types` 不再由 `frontend` **直接**依赖（W1：网关预设表在 `frontend/src/client_llm_presets.rs`）。

升级契约时改 `version`（过渡期改 `rev`）并提交 lock；打产品 tag 前 Server 须绿：`bash scripts/check-client-contract.sh`。

**禁止**：`path = "../crabmate_agent/..."` 或任何回主开发树的 Cargo path。

## 冒烟清单

1. `bash scripts/check-no-main-path.sh`
2. `bash scripts/check.sh`（含 frontend wasm32 clippy）
3. `make frontend` → `trunk build`；产物在 `frontend/dist`
4. 外部 `crabmate serve --with-web` + `CM_WEB_STATIC_DIR=$PWD/frontend/dist`，或 `make prepare-sidecar` 后桌面壳一轮对话（[`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）
