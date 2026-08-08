# 测试与检查（crabmate-client）

## pre-commit

```bash
pip install pre-commit lizard   # 或 uv tool install pre-commit；复杂度需 lizard
pre-commit install
pre-commit install --hook-type commit-msg
pre-commit run --all-files
```

钩子：

| 钩子 | 说明 |
|------|------|
| `check-no-main-path` | 禁止 Cargo path 回主仓 |
| `cargo-fmt` | desktop / mobile / connect / frontend |
| `desktop-dist-stubs` | tauri-build 所需 dist 占位 |
| `desktop-clippy` / `mobile-clippy` / `connect-clippy` | `-D warnings` |
| `frontend-wasm-check` / `frontend-clippy` | wasm32 check + clippy |
| `lizard-rust` | 按模块 CCN（含 `frontend`） |
| `fn-param-ratchet` | 形参 ≤ 9 |
| `fn-nloc-ratchet` | 函数 nloc ≤ 200、单文件 ≤ 920 |
| `taplo-format` / `taplo-lint` | 有 `taplo` 才跑，否则跳过 |
| `typos` | 拼写 |
| `e2e-format` / `e2e-lint` | Playwright：`cd e2e && npm run format:check` / `lint`（需先 `cd e2e && npm ci`） |
| `conventional-pre-commit` | commit-msg |

**不含** Victauri 全量 E2E（默认 CI）。**含** Playwright mock E2E（见下节与 `.github/workflows/e2e-playwright.yml`）。

未装 `pre-commit` 时至少：

```bash
bash scripts/check.sh
```

## CI（GitHub Actions）

工作流：[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)

| Job / 工作流 | 内容 |
|--------------|------|
| `CI` / `check` | `check-no-main-path`、`scripts/check.sh`（含 frontend wasm/clippy + 复杂度）、`make frontend`（trunk）、connect/desktop test、mobile check |
| `CI` / `build-desktop-deb` | `CM_PREPARE_SKIP_FRONTEND=1` + stub；`make desktop-release`；校验 `Package: crabmate-desktop`、无 serve sidecar、无 `/etc/crabmate` |
| `E2E Playwright` | 本仓 `make frontend` + checkout Server 编 `serve`；mock SSE 基线 |
| `code-complexity` | 独立门禁：`lizard-rust` / `fn-param` / `fn-nloc` |

Victauri 全量 E2E **不**进默认 CI（需本机/`PATH` 中的 `serve` + WebView）；见下节。

本地 UI / 打包：

```bash
make frontend                 # trunk → frontend/dist
make desktop-release          # 完整 .deb（默认同步本仓 frontend/dist）
make desktop-bin-release      # 仅二进制
```

本地仅跑复杂度：

```bash
bash scripts/lizard-rust.sh
bash scripts/fn-param-ratchet.sh
bash scripts/fn-nloc-ratchet.sh
```

## Playwright（浏览器 Web UI E2E）

权威目录：本仓 [`e2e/`](../e2e/)。一键（起 `serve` + 跑测）：

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

`serve` 解析顺序：`CRABMATE_BIN` → `PATH` 的 `crabmate` → 同级 Server `target/{debug,release}/crabmate` → 同级仓 `cargo run`。正式 CI checkout `noisystreet/CrabMate`。

真实 LLM 规格仅本地：钥匙串已有 `client_llm` 时可不必 `API_KEY`；启用 Web Bearer 时设 `CM_WEB_API_BEARER_TOKEN`。  
`cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com npx playwright test specs/real-llm-*.spec.ts`  
（三轮滚动另需 `REAL_LLM_E2E=1`。）

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

## Server / 协议

主仓：`docs/测试指南.md`、`crabmate e2e`（编排真 LLM）、`client-contract` CI。Playwright **在本仓** `e2e/`。
