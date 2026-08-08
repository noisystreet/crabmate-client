# 测试与检查（crabmate-client）

## pre-commit

```bash
pip install pre-commit   # 或 uv tool install pre-commit
pre-commit install
pre-commit install --hook-type commit-msg
pre-commit run --all-files
```

钩子：`desktop` / `mobile` / `connect` 的 `cargo fmt` 与 `clippy -D warnings`；typos；Conventional Commits（commit-msg）。  
**不含** Victauri 全量 E2E、也不跑 Server 主仓 frontend wasm。

未装 `pre-commit` 时至少：

```bash
bash scripts/check.sh
```

## CI（GitHub Actions）

工作流：[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)

- `scripts/check-no-main-path.sh`：禁止 path 回主仓
- `scripts/check.sh`：fmt + clippy（desktop / mobile / connect）
- `cargo test`：connect + desktop（未设 `VICTAURI_E2E` 时 Victauri 套件自动跳过）
- `cargo check`：mobile

Victauri 全量 E2E **不**进默认 CI（需本机/`PATH` 中的 `serve` + WebView）；见下节。

## Victauri（Desktop 壳 E2E）

```bash
./scripts/victauri-e2e.sh all
./scripts/victauri-e2e.sh victauri_scroll_send
REAL_LLM_E2E=1 ./scripts/victauri-e2e.sh real_llm
```

`serve` 二进制解析顺序：`CM_DESKTOP_BACKEND_BIN` → `PATH` 中的 `crabmate` → 同级 `../crabmate_agent/target/debug/crabmate`（仅本地双轨）。正式验收应钉已发布/`PATH` 中的 `serve`。

脚本在构建前会**临时**写入 `victauri:default` capability（JS bridge 必需），退出时恢复；**勿**把该权限长期留在无 `--features victauri` 的 `capabilities/default.json`（否则普通 `cargo check` 会失败）。

## 人工壳冒烟

见 [`docs/design/shell_smoke_runbook.md`](design/shell_smoke_runbook.md)。

## Server / 协议 / Playwright

留在主仓：`docs/测试指南.md`、`e2e/`、`crabmate e2e`、`client-contract` CI。
