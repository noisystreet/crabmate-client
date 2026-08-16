# 展示 crate 下沉本仓 — 消费侧清单

> **权威决策与波次**：Server [`client_display_crate_sink.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_display_crate_sink.md)。  
> **单包 crates.io**：Server [`crates_io_single_package.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/crates_io_single_package.md)（W3 **缓做**；下一波 Client 是该文 **S3**）。  
> **本文只列 Client 仓要做的事**；线协议仍钉 Server git tag（单包落地前），禁止 `path` 回 `crabmate_agent`。

## 目标态依赖

**当前（W2 后 / 单包前）**

```toml
# 线契约 — git tag（Server）
crabmate-sse-protocol = { git = "https://github.com/noisystreet/CrabMate", tag = "v0.3.0", package = "crabmate-sse-protocol" }
# 另钉：api-contract、chat-export、display-rules、turn-layout

crabmate-tool-card = { path = "../crates/crabmate-tool-card" }
```

**单包后（S3）**

```toml
crabmate = { git = "https://github.com/noisystreet/CrabMate", tag = "v0.4.0", package = "crabmate", default-features = false, features = ["protocol"] }
# 或 crates.io：version = "0.4.0"
crabmate-tool-card = { path = "../crates/crabmate-tool-card" }
```

`crabmate-tui-core` 同样只开 `protocol`，不要开 `server`。

## 勾选（与 Server 波次同 ID）

| ID | 状态 | Client 动作 |
|----|------|-------------|
| W0.2 | ✅ | 本文 + 更新 [`contract_pin.md`](./contract_pin.md) |
| W1.1 | ✅ | 拷贝 `LLM_API_BASE_PRESETS`；去掉 `crabmate-types` 直接依赖 |
| W1.2 | ✅ 跳过 | 保留 `api-contract::StatusShellView`（OpenAPI 同源；拷贝会漂） |
| W1.3 | ✅ | 钉清单 / lockfile |
| W2.1 | ✅ | 迁入 `crates/crabmate-tool-card`；frontend path |
| W2.2 | ✅ | `check-no-main-path.sh` 仍禁 Server path |
| W3.1–W3.4 | ⏸ 缓做 | **不**迁 `turn-layout`；随 Server 单包成为 `crabmate::turn_layout` |
| W5.1–W5.2 | ⏸ 缓做 | 拷 `display-rules` 不阻塞 crates.io |
| S0.2 | ✅ | 本文指向单包 ADR |
| S3.1–S3.4 | ⬜ | 钉 `crabmate` + `protocol`；改 `use`；见 Server ADR §5 S3 |

**不要做**：`path = "../../crabmate_agent/crates/…"`；把 `sse-protocol` 源码拷进本仓；为发 crates.io 去 vendor `turn-layout`。

## 验收命令

```bash
bash scripts/check-no-main-path.sh
# W2 后（本仓无根 workspace，勿用 cargo test -p）：
cd crates/crabmate-tool-card && cargo test
cd frontend && cargo test --lib
# 既有：
make frontend-check   # 若环境已装
```
