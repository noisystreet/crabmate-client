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
| `check-boundaries` | 依赖边界机械检查：client-api 纯度（禁 reqwest / tokio / tauri / web-sys / wasm-bindgen）、connect 默认 feature 无 Tauri、契约钉形状唯一、全部包版本一致（`scripts/check-boundaries.sh`） |
| `check-css-breakpoints` | 响应式断点单源检查：样式层 `@media` 窄屏 `max-width: N` 与互补 `min-width: N+1` 必须匹配 `frontend/src/app_prefs.rs` 的 `MOBILE_LAYOUT_BREAKPOINT_PX`；其他断点须在脚本 `EXTRA_BREAKPOINTS` 登记；e2e 移动视口须落在窄屏侧（`scripts/check-css-breakpoints.sh`） |
| `check-css-contract` | 类名契约门禁：本仓 Rust / HTML 消费方产出的类名必须在同作用域 CSS（frontend / connect / splash；第三方 vendor JS 运行时类名不在契约内）有规则，否则须在 `scripts/css_contract_allowlist.txt` 登记并写理由；白名单失效（CSS 已定义 / 消费方已移除 / scope 笔误）同样失败（`scripts/check-css-contract.sh`） |
| `check-css-tokens` | 设计 token 门禁（`frontend/styles` + `frontend/themes`）：`var(--x)` 引用的 token 必须在同范围有定义，`var()` 兜底禁止颜色字面量（兜底不生效，会掩盖主题回归），且**主题覆盖必须完整**——任一主题覆盖过的 token，其余主题（`themes/*.css`，排除 `*.example.css`）必须同样显式覆盖，否则该主题静默沿用默认深色值；主题身份差异（字体族 / 圆角档）登记 `css_tokens_check.py` 的 `THEME_IDENTITY_TOKENS`；运行时注入属性（Rust `setProperty` / Android IME / 内联 `style`）须在 `scripts/css_tokens_allowlist.txt` 登记并写理由，白名单失效（已定义 / 已无引用）同样失败（`scripts/check-css-tokens.sh`） |
| `check-css-literals` | 字面量预算门禁（`frontend/styles/*.css`，排除 `tokens.css`）：逐文件统计 `font-size` 与颜色（`#hex` / `rgb()` / `hsl()` / `white` / `black`）字面量个数，**实际值必须等于** `scripts/css_literals_budget.txt` 的逐文件预算（新增即失败，清理后未同步下调预算也失败）；预算条目失效（文件不存在 / 不在扫描范围 / 两项均为 0）同样失败（`scripts/check-css-literals.sh`） |
| `check-token-mirrors` | 设计令牌镜像门禁（单一来源 = `frontend/styles/tokens.css` 的 `:root`）：非 CSS 表面只能把色值各抄一份，故机械校验——(1) `mobile-tauri/.../res/values/colors.xml` 必须恰好定义被消费令牌的 `cm_<token>` 镜像（`bg` / `surface` / `text` / `muted` / `accent` / `error`）且色值一致、不得混入裸颜色；(2) `res/values*/themes.xml` 不得出现颜色字面量，`Theme.crabmate_mobile` 的四个底色必须引用 `@color/cm_bg` 并显式声明 `windowLightStatusBar` / `windowLightNavigationBar = false`（底色恒深色，DayNight 浅色变体默认给深色图标→不可见）；(3) `frontend/index.html` / `desktop-tauri/splash.html` / `crates/crabmate-connect/assets/connect.html` 的首绘底色必须等于 `--bg`，且声明必须**恰好命中一次**（改名 / 搬走即失败，防门禁静默失效）；三条规则均为精确相等、无白名单（`scripts/check-token-mirrors.sh`） |
| `check-xml-comments` | XML 注释门禁：全仓 `*.xml`（剪枝构建产物与依赖目录）的注释正文**不得出现连续两个减号**（`--`）—— XML 1.0 §2.5 禁止，aapt2 资源合并器会直接报「注释中不允许出现字符串 "--"」；典型踩坑是在注释里写 CSS 自定义属性（`--bg`）或命令行选项（`--release`），改写成不带连字符前缀或纯文字描述即可；`mobile-tauri/src-tauri/gen/android/**/res/**` 是提交进仓库的 Tauri 移动端资源，**在扫描范围内**（`scripts/check-xml-comments.sh`） |
| `cargo-fmt` | 全部 Rust 包循环（单一列表 `scripts/rust-pkg-dirs.txt`；fmt 与工具链无关，frontend 同循环处理） |
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
| `CI` / `check` | `check-no-main-path`、`scripts/check.sh`（含 `check-boundaries` 边界检查 + frontend wasm32 clippy + 复杂度）、`make frontend`（trunk）、`make test-frontend`、`make test-tui`、`make test-web-host`、connect/desktop **unit** test（desktop `cargo test --bins`）、mobile check |
| `CI` / `victauri-e2e` | **Skipped**（`if: false`）；壳 E2E 见 nightly |
| `CI` / `build-desktop-deb` | `CM_PREPARE_SKIP_FRONTEND=1` + stub；`make desktop-release`；校验 `Package: crabmate-desktop`、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-web-deb` | `CM_WEB_SKIP_FRONTEND=1` + stub dist；`make web-release`；校验 `Package: crabmate-web`、菜单图标、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-tui-deb` | `make tui-release`；校验 `Package: crabmate-tui`、仅 `/usr/bin/crabmate-tui`、无图标/配置、无 serve sidecar、无 `/etc/crabmate` |
| `Release`（推 `v*` tag） | guard：tag 须可达自 `origin/main` → `build-frontend`（`make frontend-release`，真实 UI，装 trunk + wasm-opt）→ 分别 `make desktop-release`（`CRABMATE_FRONTEND_DIST` 复用 dist）/ `make web-release`（`CM_WEB_SKIP_FRONTEND=1` + 真 dist）/ `make tui-release`；校验同 CI 三个 deb job → 上传三 .deb，notes 取 `CHANGELOG.md` 对应版本段发 GitHub Release（`.github/workflows/release.yml`） |
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

## 多端维护脚本（包列表 / 边界 / 版本）

全部 Rust 包目录的单一来源是 **`scripts/rust-pkg-dirs.txt`**（`#` 为注释行）：Makefile `fmt` / `clippy` / `clean`、`scripts/check.sh`、`scripts/check-boundaries.sh` 都从它取包列表，**新增包只改该文件**（再按需补 Makefile `test-*` 分组与 CI rust-cache 目录）。背景决策（为何不合并单一 Cargo workspace）见 [`docs/adr/0004-multi-client-repo-maintenance.md`](adr/0004-multi-client-repo-maintenance.md)。

```bash
bash scripts/check-boundaries.sh   # 边界机械检查（也是 pre-commit 钩子 + check.sh 一环）
bash scripts/set-version.sh 0.5.1  # 一次改全部包版本 + 各自 Cargo.lock
```

`check-boundaries` 守护四件事：`crabmate-client-api` 纯度（禁 reqwest / tokio / tauri / web-sys / wasm-bindgen）、`crabmate-connect` 默认 feature 不含 Tauri、crates.io `crabmate` 契约钉形状唯一（`default-features = false, features = ["protocol"]`；禁旧包名 `crabmate-sse-protocol`）、全部包版本一致。改版本必须走 `set-version.sh`，手改单个 Cargo.toml 会被该检查拒绝。

## Playwright（浏览器 Web UI E2E）

权威目录：本仓 [`e2e/`](../e2e/)。一键（起纯 API `serve` + `crabmate-web` 托管 UI + 跑测）：

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

`serve` 解析顺序：`CRABMATE_BIN` → `PATH` 的 `crabmate` → 同级 Server `target/{debug,release}/crabmate` → 同级仓 `cargo run`。正式 CI checkout `noisystreet/CrabMate` 钉 git tag **`v0.5.2`**（与 crates.io `crabmate` 0.5.2 同源；见 [`contract_pin.md`](design/contract_pin.md)）。

**UI 托管**：Server 默认纯 API（脚本/CI **不传 `--with-web`**）；SPA 由客户端自托管 `crabmate-web`（本仓 `web-host/`，默认 `127.0.0.1:4173`，`--api-base` 指向纯 API serve）。页面经 `#cm_api_base=` hash 交接把 API 指向 serve；serve 须经 `CM_WEB_CORS_ALLOWED_ORIGINS` 放行 web Origin（脚本自动追加 `http://127.0.0.1:$CRABMATE_WEB_PORT`）。跨 Origin 直连 API 与 `crabmate-web --api-base` 的真实使用路径一致。

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
