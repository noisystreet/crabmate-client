## 单元 / 静态检查（分开）

```bash
make test-frontend   # wasm check + frontend lib 单测
make test-tauri      # crabmate-connect + desktop cargo test + mobile check
make test-web-host   # crabmate-web 回环静态服务单测
make test            # frontend → tauri → tui → web-host
```

桌面壳冒烟（真实 WebKitGTK + 外部 serve，xvfb）：`make smoke-desktop-shell`。  
Playwright（浏览器 UI，需纯 API `serve` + 客户端 `crabmate-web` 自托管）：`make e2e-playwright`。

失败时先取 CI 的 `playwright-report` artifact —— 里面既有 html 报告，也有 `test-results/` 的失败截图与 trace。workflow 在开跑前会探活 `:8080/health` 与 `:4173/`，不可达时打印监听端口与进程存活情况，用来把「服务在测试前就已失联」与「测试自身失败」分开。页面就绪统一等 `domcontentloaded` + composer 选择器，**不要**用 `networkidle`：SPA 首屏后会挂水合 / 轮询请求，它永远等不到，超时错误还会盖住真正的原因。

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
| `check-css-contract` | 类名契约门禁：本仓 Rust / HTML 消费方产出的类名必须在同作用域 CSS（frontend / connect；第三方 vendor JS 运行时类名不在契约内）有规则，否则须在 `scripts/css_contract_allowlist.txt` 登记并写理由；白名单失效（CSS 已定义 / 消费方已移除 / scope 笔误）同样失败（`scripts/check-css-contract.sh`） |
| `check-css-tokens` | 注 1（见文末） |
| `check-css-literals` | 字面量预算门禁（`frontend/styles/*.css`，排除 `tokens.css`）：逐文件统计 `font-size`、颜色（`#hex` / `rgb()` / `hsl()` / `white` / `black`）与 `border-radius`（`px` / `rem` / `em` / `%`；裸 `0` 不计，`var(--radius-x, 5px)` 兜底计入）字面量个数，**实际值必须等于** `scripts/css_literals_budget.txt` 的逐文件预算（新增即失败，清理后未同步下调预算也失败）；预算条目失效（文件不存在 / 不在扫描范围 / 三项均为 0）同样失败（`scripts/check-css-literals.sh`） |
| `check-token-mirrors` | 见 `scripts/check-token-mirrors.sh`（七条精确相等规则，无白名单；逐条说明见 `AGENTS.md`） |
| `check-xml-comments` | XML 注释门禁：全仓 `*.xml`（剪枝构建产物与依赖目录）的注释正文**不得出现连续两个减号**（`--`）—— XML 1.0 §2.5 禁止，aapt2 资源合并器会直接报「注释中不允许出现字符串 "--"」；典型踩坑是在注释里写 CSS 自定义属性（`--bg`）或命令行选项（`--release`），改写成不带连字符前缀或纯文字描述即可；`mobile-tauri/src-tauri/gen/android/**/res/**` 是提交进仓库的 Tauri 移动端资源，**在扫描范围内**（`scripts/check-xml-comments.sh`） |
| `check-changelog-style` | CHANGELOG 条目门禁：只校验 `[Unreleased]` 段，每条 ≤ 800 字符 —— 条目写「改了什么 + 为什么 + 代价」，机制 / 备选方案 / 验证清单归 PR 描述，长期有效的理由归 `docs/design/`；**已发布段冻结**（它们就是 `release.yml` 逐字抽取并在 GitHub 固化的发布说明），改写会让文件与已发布 notes 永久不一致，长段请在上面加一段 `**Summary**`（`scripts/check-changelog-style.sh`） |
| `check-toolchain-pin` | 工具链单源门禁：`rust-toolchain.toml` 的 `channel` 必须等于每个 workflow 里 `dtolnay/rust-toolchain` 钉住的版本（`@master` + `with: toolchain:`；禁 `@stable` / `@beta` / `@nightly` 浮动）。原因：`frontend-clippy` 等钩子与 CI `check` job 跑的是同一条命令，浮动工具链下两边并不是同一个编译器，新加的 lint 只在 CI 报（clippy 1.100 的 `needless_bool` 新分支 vs 本地 1.98），表现为「pre-commit 全绿、PR 红」（`scripts/check-toolchain-pin.sh`） |
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
| `check-md-line-length` | Markdown **逐行**上限：任何一行都不得超过 `MAX_COLS` 显示列（东亚宽字符按 2 列，约 80 汉字）。唯一豁免是围栏代码块（折行会改语义），表格行同样受约束；`CHANGELOG.md` 整文件不计（条目单行 ≤800 字符、已发布段冻结）。`MAX_COLS` 当前 600，目标 160，**只许下调**（`--list` 按列数列出各文件最长行，便于挑下一档）（`scripts/check-md-line-length.sh`） |
| `conventional-pre-commit` | commit-msg |

**不含** 桌面壳冒烟与 Playwright 运行本身（CI 里另有 `E2E Playwright` 工作流；壳冒烟不进 CI）。

未装 `pre-commit` 时至少：

```bash
bash scripts/check.sh
```

## CI（GitHub Actions）

工作流：[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)

| Job / 工作流 | 内容 |
|--------------|------|
| `CI` / `check` | `check-no-main-path`、`scripts/check.sh`（含 `check-boundaries` 边界检查 + frontend wasm32 clippy + 复杂度）、`make frontend`（trunk）、`make test-frontend`、`make test-tui`、`make test-web-host`、connect/desktop **unit** test（desktop `cargo test --bins`）、mobile check |
| `CI` / `build-desktop-deb` | `CM_PREPARE_SKIP_FRONTEND=1` + stub；`make desktop-release`；校验 `Package: crabmate-desktop`、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-web-deb` | `CM_WEB_SKIP_FRONTEND=1` + stub dist；`make web-release`；校验 `Package: crabmate-web`、菜单图标、无 serve sidecar、无 `/etc/crabmate` |
| `CI` / `build-tui-deb` | `make tui-release`；校验 `Package: crabmate-tui`、仅 `/usr/bin/crabmate-tui`、无图标/配置、无 serve sidecar、无 `/etc/crabmate` |
| `Release`（推 `v*` tag） | guard：tag 须可达自 `origin/main` → `build-frontend`（`make frontend-release`，真实 UI，装 trunk + wasm-opt）→ 分别 `make desktop-release`（`CRABMATE_FRONTEND_DIST` 复用 dist）/ `make web-release`（`CM_WEB_SKIP_FRONTEND=1` + 真 dist）/ `make tui-release`；校验同 CI 三个 deb job → 上传三 .deb，notes 取 `CHANGELOG.md` 对应版本段发 GitHub Release（`.github/workflows/release.yml`） |
| `E2E Playwright` | 本仓 `make frontend` + checkout Server 编纯 API `serve` + 本仓编 `crabmate-web`（回环托管 UI）；mock SSE 基线 |
| `code-complexity` | 独立门禁：`lizard-rust` / `fn-param` / `fn-nloc` |
| `Dependency security` | 各 Cargo workspace：`cargo audit` + `cargo deny check licenses bans sources`（`deny.toml`）；**不进** pre-commit |

桌面壳冒烟**不进** CI：它要 xvfb + 真实 WebKitGTK + 一份 serve，比页面 E2E 贵得多；本地 / 发版前用 `make smoke-desktop-shell`。

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
bash scripts/set-version.sh 0.5.1  # 一次改全部包版本 + 各自 Cargo.lock + 壳 tauri.conf.json（Tauri 打包版本以它为准）
```

`check-boundaries` 守护五件事：`crabmate-client-api` 纯度（禁 reqwest / tokio / tauri / web-sys / wasm-bindgen）、`crabmate-connect` 默认 feature 不含 Tauri、crates.io `crabmate` 契约钉形状唯一（`default-features = false, features = ["protocol"]`；禁旧包名 `crabmate-sse-protocol`）、全部包版本一致、壳 `tauri.conf.json` 顶层 `version` 与包版本一致（Tauri 打 .deb / APK 以它为准）。改版本必须走 `set-version.sh`，手改单个 Cargo.toml 或 tauri.conf.json 会被该检查拒绝。

## Playwright（浏览器 Web UI E2E）

权威目录：本仓 [`e2e/`](../e2e/)。一键（起纯 API `serve` + `crabmate-web` 托管 UI + 跑测）：

```bash
make frontend
./scripts/e2e-playwright.sh
# 或指定用例：./scripts/e2e-playwright.sh specs/mock-overlay-timing.spec.ts
```

`serve` 解析顺序：`CRABMATE_BIN` → `PATH` 的 `crabmate` → 同级 Server `target/{debug,release}/crabmate` → 同级仓 `cargo run`。正式 CI checkout `noisystreet/CrabMate` 钉 git tag **`v0.8.0`**（Server 最新产品 tag，早于当前契约 rev `817157e0`，故 E2E 覆盖不到 `context_budget`；见 [`contract_pin.md`](design/contract_pin.md)）。

**UI 托管**：Server 默认纯 API（脚本/CI **不传 `--with-web`**）；SPA 由客户端自托管 `crabmate-web`（本仓 `web-host/`，默认 `127.0.0.1:4173`，`--api-base` 指向纯 API serve）。页面经 `#cm_api_base=` hash 交接把 API 指向 serve；serve 须经 `CM_WEB_CORS_ALLOWED_ORIGINS` 放行 web Origin（脚本自动追加 `http://127.0.0.1:$CRABMATE_WEB_PORT`）。跨 Origin 直连 API 与 `crabmate-web --api-base` 的真实使用路径一致。

真实 LLM 规格仅本地：本机钥匙串/E2E 注入已有 `client_llm` 时可不必 `API_KEY`；启用 Web Bearer 时设 `CM_WEB_API_BEARER_TOKEN`。  
`cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com npx playwright test specs/real-llm-*.spec.ts`  
（三轮滚动另需 `REAL_LLM_E2E=1`。）

## 桌面壳冒烟（scripts/smoke-desktop-shell.sh）

```bash
make smoke-desktop-shell          # 或 ./scripts/smoke-desktop-shell.sh
```

判据（按强度）：壳进程在观察窗内仍存活 → 日志无 `startup failed` / panic → 壳的**子进程**里出现 WebKit（`WebKitWebProcess` / `WebKitNetworkProcess`，= WebView 引擎真的初始化并开始加载页面）。

不查 X11 窗口：在无窗口管理器的 xvfb 里，主窗口先以不可见状态创建、页面加载完才 `show()`，而 debug 构建的 WASM 有 160+ MiB —— 实测到 43s 仍 `xwininfo -root -tree` 报 0 children。把窗口当判据只会得到一条稳定假红。

页面行为（分页 / 设置页 / 模型注册…）在 Playwright（`e2e/specs`，随 PR 门禁）；钥匙串往返在 `crates/crabmate-connect` 的单测。壳冒烟只回答「打完包的壳在本机能起来吗」。

`serve` 二进制解析顺序：`CM_DESKTOP_BACKEND_BIN` → `PATH` 中的 `crabmate` → 同级 `../crabmate_agent` / `../CrabMate` / `../server` 的 `target/{debug,release}/crabmate`。已有 serve 监听同端口时直接复用（不杀别人的）。壳以 `CM_DESKTOP_SKIP_CONNECT=1` + `CM_DESKTOP_SERVE_URL` 直接开包内 UI，启动 `serve` 时**不传** `--with-web`（纯 API 即可）。

## 人工壳冒烟

见 [`docs/design/shell_smoke_runbook.md`](design/shell_smoke_runbook.md)。

## Server / 协议

主仓：`docs/测试指南.md`、`crabmate e2e`（编排真 LLM）、`client-contract` CI。Playwright **在本仓** `e2e/`。

## 表格长备注
### 注 1 · `check-css-tokens`

设计 token 门禁（`frontend/styles` + `frontend/themes`）：`var(--x)` 引用的 token 必须在同范围有定义，`var()` 兜底禁止颜色字面量（兜底不生效，会掩盖主题回归），且**主题覆盖必须完整**——任一主题覆盖过的 token，其余主题（`themes/*.css`，排除 `*.example.css`）必须同样显式覆盖，否则该主题静默沿用默认深色值；主题身份差异（字体族 / 圆角档）登记 `css_tokens_check.py` 的 `THEME_IDENTITY_TOKENS`；运行时注入属性（Rust `setProperty` / Android IME / 内联 `style`）须在 `scripts/css_tokens_allowlist.txt` 登记并写理由，白名单失效（已定义 / 已无引用）同样失败（`scripts/check-css-tokens.sh`）
