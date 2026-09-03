## 单元 / 静态检查（分开）

```bash
make test-frontend   # wasm check + frontend lib 单测
make test-tauri      # crabmate-connect + desktop cargo test + mobile check
make test-web-host   # crabmate-web 回环静态服务单测
make test            # frontend → tauri → tui → web-host
```

Victauri 全量 E2E（需 WebView / 外部 serve）另跑：`make victauri-e2e`。  
Playwright（浏览器 UI，需纯 API `serve` + 客户端 `crabmate-web` 自托管）：`make e2e-playwright`。

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
| `cargo-fmt` | desktop / mobile / connect / tui / web-host / frontend |
| `desktop-dist-stubs` → **`tauri-dist-stubs`** | 为 **desktop/mobile** `frontendDist` 建占位（`scripts/ensure-tauri-dist-stubs.sh`；与 CI `check.sh` 共用） |
| `desktop-clippy` / `mobile-clippy` / `connect-clippy` / `tui-clippy` / `web-host-clippy` | `-D warnings` |
| `frontend-clippy` | wasm32 clippy（含类型检查；不再单独 `cargo check`） |
| `lizard-rust` | 全局硬门禁：任何函数 **CCN>10** 即失败（列出命中；无按模块的个数上限配置） |
| `fn-param-ratchet` | 形参 ≤ 9 |
| `fn-nloc-ratchet` | 函数 nloc ≤ 200、单文件 ≤ 920 |
| `taplo-format` / `taplo-lint` | 有 `taplo` 才跑，否则跳过 |
| `ktlint-android` | 手改 Android Kotlin（`edu/crabmate`，排除 `generated/`）；需 **java**；首次下载钉死的 ktlint CLI 到缓存。格式化：`bash scripts/ktlint-android.sh --format` |
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
| `CI` / `check` | `check-no-main-path`、`scripts/check.sh`（含 frontend wasm32 clippy + 复杂度）、`make frontend`（trunk）、`make test-frontend`、`make test-tui`、`make test-web-host`、connect/desktop **unit** test（desktop `cargo test --bins`）、mobile check |
| `CI` / `victauri-e2e` | **Skipped**（`if: false`）；壳 E2E 见 nightly |
| `CI` / `build-desktop-deb` | `CM_PREPARE_SKIP_FRONTEND=1` + stub；`make desktop-release`；校验 `Package: crabmate-desktop`、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-web-deb` | `CM_WEB_SKIP_FRONTEND=1` + stub dist；`make web-release`；校验 `Package: crabmate-web`、菜单图标、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-tui-deb` | `make tui-release`；校验 `Package: crabmate-tui`、仅 `/usr/bin/crabmate-tui`、无图标/配置、无 serve sidecar、无 `/etc/crabmate` |
| `E2E Playwright` | 本仓 `make frontend` + checkout Server 编纯 API `serve` + 本仓编 `crabmate-web`（回环托管 UI）；mock SSE 基线 |
| `Victauri E2E Nightly` | `make frontend` + Server `serve` + `./scripts/victauri-e2e.sh all`（xvfb；不含 `real_llm`）；失败上传桌面/serve 日志 |
| `code-complexity` | 独立门禁：`lizard-rust` / `fn-param` / `fn-nloc` |
| `Dependency security` | 各 Cargo workspace：`cargo audit` + `cargo deny check licenses bans sources`（`deny.toml`）；**不进** pre-commit |

Victauri 全量 E2E **不进** PR 默认 CI（`e2e_test!` 未设 `VICTAURI_E2E` 时会 0 秒假通过）；见 nightly 或本地 `victauri-e2e.sh`。

本地 UI / 打包：

```bash
make frontend                 # trunk debug → frontend/dist（开发迭代）
make frontend-release         # trunk --release（需 wasm-opt；~数 MB WASM）
make desktop-release          # 完整 .deb（beforeBuild 会跑 trunk --release + 体积门禁）
make web-release              # crabmate-web .deb（trunk --release + 回环静态服务）
make tui-release              # crabmate-tui .deb（仅二进制；无图标、无配置）
make desktop-bin-release      # 仅二进制
```

本地仅跑复杂度：

```bash
bash scripts/lizard-rust.sh
bash scripts/fn-param-ratchet.sh
bash scripts/fn-nloc-ratchet.sh
```

Lizard 全局硬门禁：重构目标是把所有函数的 CCN 压到 ≤10；一旦出现 `CCN>10`，pre-commit / `lizard-rust` 会失败并列出命中，需继续拆分，不存在按模块的个数上限或 `--write-caps`。

## 依赖安全与许可证

工作流：[`.github/workflows/dependency-security.yml`](../.github/workflows/dependency-security.yml)。需安装 **`cargo-audit`**、**`cargo-deny`**：

```bash
make dependency-security
# 或：bash scripts/dependency-security.sh
```

策略见仓库根 **`deny.toml`**。对全部 7 个 Cargo workspace 各跑一遍（各有独立 `Cargo.lock`）。**不进** pre-commit，避免每次提交都拉 RustSec advisory DB。CI 不含 `advisories` deny 检查（与 `cargo audit` 重复，且会把 unmaintained 与漏洞混为一谈）。

## Playwright（浏览器 Web UI E2E）

权威目录：本仓 [`e2e/`](../e2e/)。一键（起纯 API `serve` + `crabmate-web` 托管 UI + 跑测）：

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

`serve` 解析顺序：`CRABMATE_BIN` → `PATH` 的 `crabmate` → 同级 Server `target/{debug,release}/crabmate` → 同级仓 `cargo run`。正式 CI checkout `noisystreet/CrabMate` 钉 git tag **`v0.5.1`**（与 crates.io `crabmate` 0.5.1 同源；见 [`contract_pin.md`](design/contract_pin.md)）。

**UI 托管**：Server 默认纯 API（脚本/CI **不传 `--with-web`**）；SPA 由客户端自托管 `crabmate-web`（本仓 `crates/crabmate-web-host`，默认 `127.0.0.1:4173`，`--api-base` 指向纯 API serve）。页面经 `#cm_api_base=` hash 交接把 API 指向 serve；serve 须经 `CM_WEB_CORS_ALLOWED_ORIGINS` 放行 web Origin（脚本自动追加 `http://127.0.0.1:$CRABMATE_WEB_PORT`）。跨 Origin 直连 API 与 `crabmate-web --api-base` 的真实使用路径一致。

真实 LLM 规格仅本地：本机钥匙串/E2E 注入已有 `client_llm` 时可不必 `API_KEY`；启用 Web Bearer 时设 `CM_WEB_API_BEARER_TOKEN`。  
`cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com npx playwright test specs/real-llm-*.spec.ts`  
（三轮滚动另需 `REAL_LLM_E2E=1`。）

## Victauri（Desktop 壳 E2E）

```bash
./scripts/victauri-e2e.sh all
./scripts/victauri-e2e.sh victauri_scroll_send
REAL_LLM_E2E=1 ./scripts/victauri-e2e.sh real_llm
```

`serve` 二进制解析顺序：`CM_DESKTOP_BACKEND_BIN` → `PATH` 中的 `crabmate` → 同级 `../crabmate_agent/target/debug/crabmate`（仅本地双轨）。正式验收应钉已发布/`PATH` 中的 `serve`。壳加载**包内** UI（Phase 2），脚本启动 `serve` 时**不传** `--with-web`（纯 API 即可）。

脚本在构建前会**临时**写入 `victauri:default` capability（JS bridge 必需），退出时恢复；**勿**把该权限长期留在无 `--features victauri` 的 `capabilities/default.json`（否则普通 `cargo check` 会失败）。

## 人工壳冒烟

见 [`docs/design/shell_smoke_runbook.md`](design/shell_smoke_runbook.md)。

## Server / 协议

主仓：`docs/测试指南.md`、`crabmate e2e`（编排真 LLM）、`client-contract` CI。Playwright **在本仓** `e2e/`。
