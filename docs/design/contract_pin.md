# 契约钉版本（壳仓 / 官方 UI）

> **权威发版策略**：Server 主仓 [`client_contract_versioning.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)  
> **UI 迁出计划**：[`frontend_migrate_plan.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/frontend_migrate_plan.md) Phase B（本仓已迁入 `frontend/`）  
> **本仓职责**：消费侧钉法 + 禁止 path 回主开发树。

## 现状

| 依赖 | 本仓做法 |
|------|----------|
| `crabmate-connect` | **本仓** `path = "../../crates/crabmate-connect"`（已迁入；勿再 git+path 回主仓旧路径） |
| 业务 UI（`frontend/`） | **本仓**；契约 crates 见 `frontend/Cargo.toml` 的 git **`tag`** |
| 壳二进制本身 | 暂不直接依赖主仓契约 crate（只经 WebView 加载 UI + HTTP/SSE） |

本地门禁：`bash scripts/check-no-main-path.sh`（需 `rg` 或 `grep`；CI 装 ripgrep）。

当前 UI 钉点（与 `frontend/Cargo.toml` 一致）：

| 项 | 值 |
|----|-----|
| 源码迁入自 | 主仓 `eb0048bf…`（见 `frontend/SOURCE.md`） |
| 契约 `tag` | **`client-contract-v0.1.0`**（主仓 `c244ebb1`） |
| lock | 提交 `frontend/Cargo.lock` |

壳打包 UI 同步：`make desktop-release` / `before-desktop-build.sh` 默认 **`trunk build --release`** 再同步本仓 `frontend/dist`（拒绝 debug 大 WASM）；`CM_PREPARE_SKIP_FRONTEND=1` 或 `CRABMATE_FRONTEND_DIST=-` 跳过；同级主仓回落需 `CRABMATE_ALLOW_SIBLING_FRONTEND=1`。

## 钉主仓契约

```toml
crabmate-api-contract = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-api-contract" }
crabmate-sse-protocol = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-sse-protocol" }
crabmate-types = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-types" }
crabmate-display-rules = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-display-rules" }
crabmate-turn-layout = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-turn-layout" }
crabmate-tool-card = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-tool-card" }
crabmate-chat-export = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-chat-export" }
```

开发期可用 `rev = "<sha>"` 代替 `tag`。打标签前主仓须绿：`bash scripts/check-client-contract.sh`。

**不必**等 Server 产品发版；契约 tag ≠ `crabmate` 安装包版本。

**禁止**：`path = "../crabmate_agent/crates/..."` 或任何回主开发树的 Cargo path。

## 冒烟清单

1. `bash scripts/check-no-main-path.sh`
2. `bash scripts/check.sh`（含 frontend wasm check / clippy）
3. `make frontend` → `trunk build`；产物在 `frontend/dist`
4. 外部 `crabmate serve` + `CM_WEB_STATIC_DIR=$PWD/frontend/dist`，或 `make prepare-sidecar` 后桌面壳一轮对话（[`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）
