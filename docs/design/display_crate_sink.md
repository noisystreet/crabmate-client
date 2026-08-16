# 展示 crate 下沉本仓 — 消费侧清单

> **权威决策与波次**：Server [`client_display_crate_sink.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_display_crate_sink.md)。  
> **本文只列 Client 仓要做的事**；线协议仍钉 Server git tag，禁止 `path` 回 `crabmate_agent`。

## 目标态依赖

```toml
# frontend/Cargo.toml（示意）
# 线契约 — 仍 git tag（Server）
crabmate-sse-protocol = { git = "https://github.com/noisystreet/CrabMate", tag = "v0.3.0", package = "crabmate-sse-protocol" }
# 可选仍钉：api-contract、chat-export、display-rules（W5 前）

# 展示 — 本仓 path（W2 / W3 后）
crabmate-tool-card = { path = "../crates/crabmate-tool-card" }
crabmate-turn-layout = { path = "../crates/crabmate-turn-layout" }
```

`crabmate-tui-core` 继续只钉 `crabmate-sse-protocol`，除非终端开始复用工具卡/投影（另开需求）。

## 勾选（与 Server 波次同 ID）

| ID | 状态 | Client 动作 |
|----|------|-------------|
| W0.2 | ✅ | 本文 + 更新 [`contract_pin.md`](./contract_pin.md) |
| W1.1 | ✅ | 拷贝 `LLM_API_BASE_PRESETS`；去掉 `crabmate-types` 直接依赖 |
| W1.2 | ✅ 跳过 | 保留 `api-contract::StatusShellView`（OpenAPI 同源；拷贝会漂） |
| W1.3 | ✅ | 钉清单 / lockfile |
| W2.1 | ⬜ | 迁入 `crates/crabmate-tool-card`；frontend path |
| W2.2 | ⬜ | `check-no-main-path.sh` 仍禁 Server path |
| W3.1 | ⬜ | 迁入 `crates/crabmate-turn-layout` + `turn_project_*.jsonl` |
| W3.2 | ⬜ | frontend path；`golden_turn_web_stored_sync` |
| W3.3 | ⬜ | CI job：`cargo test -p crabmate-turn-layout` 金样 |
| W3.4 | ⬜ | （可选）少量 AG-UI → 投影表征测 |
| W5.1–W5.2 | ⬜ | （可选）拷 `display-rules` 进 `crabmate-client-api` |

**不要做**：`path = "../../crabmate_agent/crates/…"`；把 `sse-protocol` 源码拷进本仓。

## 验收命令

```bash
bash scripts/check-no-main-path.sh
# W2 后：
cargo test -p crabmate-tool-card
# W3 后：
cargo test -p crabmate-turn-layout
cd frontend && cargo test --lib
# 既有：
make frontend-check   # 若环境已装
```
