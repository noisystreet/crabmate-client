# 契约钉版本（壳仓）

> **权威发版策略**：Server 主仓 [`client_contract_versioning.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)  
> **本仓职责**：消费侧钉法 + 禁止 path 回主开发树。

## 现状（过渡）

| 依赖 | 本仓做法 |
|------|----------|
| `crabmate-connect` | **本仓** `path = "../../crates/crabmate-connect"`（已迁入；勿再 git+path 回主仓旧路径） |
| `crabmate-api-contract` / `crabmate-sse-protocol` | 壳本身暂不直接依赖；业务 UI 仍在主仓 `serve`。将来 UI 进本仓或独立 UI 仓时，按下方钉 tag |

本地门禁：`bash scripts/check-no-main-path.sh`（CI 同跑）。

## 钉主仓契约（有 tag 时）

主仓打注释标签 `client-contract-vX.Y.Z` 后，外仓示例：

```toml
[dependencies]
crabmate-api-contract = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-api-contract" }
crabmate-sse-protocol = { git = "https://github.com/noisystreet/CrabMate", tag = "client-contract-v0.1.0", package = "crabmate-sse-protocol" }
```

开发期可用 `rev = "<sha>"` 代替 `tag`。打标签前主仓须绿：`bash scripts/check-client-contract.sh`。

**禁止**：`path = "../crabmate_agent/crates/..."` 或任何回主开发树的 Cargo path。

## 冒烟清单（P3.3）

1. `bash scripts/check-no-main-path.sh`
2. `bash scripts/check.sh`
3. （可选，有 tag 后）在临时 crate 中仅用 git tag 依赖契约并 `cargo check`
4. Desktop / Android 连已安装或 `PATH` 中的 `crabmate serve` 一轮对话（人工：[`shell_smoke_runbook.md`](./shell_smoke_runbook.md)）
