# ADR-0004: 多端客户端仓库的维护形态（不合并单一 Cargo workspace）

## Status

Proposed

## Context

本仓是**多端薄壳仓**：Desktop Tauri、Android Tauri、Web（`crabmate-web` 回环托管 Leptos CSR）、远程 `crabmate-tui` 四种交付形态，由 9 个 Rust 包组成（`desktop-tauri/src-tauri`、`mobile-tauri/src-tauri`、`crates/{crabmate-client-api,crabmate-tool-card,crabmate-connect,crabmate-tui-core,crabmate-tui,crabmate-web-host}`、`frontend`）。

历史上每个包都是**独立单包 workspace**（各有自己的 `[workspace]` 与 `Cargo.lock`），这带来真实收益：

- **依赖解析隔离**：`frontend` 是 wasm32 工具链、`mobile-tauri` 有 Android 侧依赖；合并 workspace 会让所有成员共享一次解析结果，任何成员升依赖都可能牵动其余 8 个包的 `Cargo.lock`（本仓曾为统一 reqwest 0.13 手工刷新过 9 份 lockfile，合并 workspace 后这种事会更频繁、更全局）。
- **构建/缓存边界清晰**：CI 的 rust-cache、`test-*` 分组、deb 打包各自按包取用；单包 workspace 让 `cargo` 命令的影响面 == 包的影响面。
- **与 Server 仓解耦**：契约钉 crates.io `crabmate`（`default-features = false, features = ["protocol"]`），本仓无任何 path 回 Server 树；独立 workspace 天然阻止「顺手引用主仓 crate」。

但独立 workspace 也有维护税，且此前只靠文档约束（AGENTS.md Hard Constraints），没有机械守护：

1. **包列表多处手写枚举**：Makefile `fmt` / `clippy` / `clean`、`scripts/check.sh`、pre-commit `cargo-fmt` 钩子各自硬编码 9 个目录；新增/改名包要改 4+ 处，漏改即静默跳过检查。
2. **边界规则无 CI 守护**：client-api 纯度（禁 reqwest / tokio / tauri / web-sys / wasm-bindgen）、connect 默认 feature 不含 Tauri、契约钉形状、9 包版本一致——全靠人肉 review。
3. **版本同步靠手改**：9 份 Cargo.toml + 9 份 Cargo.lock 逐个改，易漂移。
4. **常量双侧漂移**：hash 交接键（`cm_api_base` / `cm_web_api_bearer`）在 Rust 常量与 Playwright fixture 中各写一份，改名时易漏一侧。

## Decision

**维持多包独立 workspace 的形态，不为合并而合并；把维护税转为四件机械化的小事**：

1. **包列表单一来源 `scripts/rust-pkg-dirs.txt`**：`#` 注释行格式的目录清单；Makefile `fmt` / `clippy` / `clean`、`scripts/check.sh`、`scripts/check-boundaries.sh` 都循环消费它，新增包只改这一处（frontend 的 wasm32 clippy 因工具链不同仍在循环外单列）。
2. **边界规则脚本化 `scripts/check-boundaries.sh`**：作为 pre-commit 钩子 + `check.sh` 一环，机械检查 client-api 纯度、connect 默认 feature、契约钉形状（全仓 `crabmate = { version, default-features = false, features = ["protocol"] }` 唯一形状且版本一致；禁旧包名 `crabmate-sse-protocol`）、全部包版本一致。
3. **版本统一改版 `scripts/set-version.sh <semver>`**：一次改 9 份 Cargo.toml 并同步各自 Cargo.lock（`cargo update -w` 只动 workspace 成员，不碰依赖钉版本）。
4. **交接常量 golden 化**：`crabmate-client-api/tests/golden/handoff_keys.json` 为键名唯一来源，Rust 单测断言常量与 golden 一致，Playwright `homeUrlWithOptionalWebBearer` 改读同一份文件。

### 备选方案：合并为单一根 workspace

优点：一次 `cargo` 命令覆盖全部；`cargo fmt --all` / `clippy --workspace` 原生循环；依赖版本天然统一。

否决理由：

- **锁文件全局耦合**：任意成员改依赖都重排根 lockfile，跨端 diff 噪声大；wasm32 与 Android 目标的解析差异会把条件依赖堆进同一份清单。
- **打破现有 CI / 打包边界**：rust-cache key、`test-*` 分组、三个 deb 打包 job、依赖安全 workflow 都按 workspace 划分，合并需整体重做且收益有限。
- **削弱与 Server 的隔离姿态**：根 workspace 会把 path 依赖解析变成「加一行就行」，违背本仓「契约只走 crates.io 钉版本」的硬约束。
- **契约钉是跨仓约定**：`crabmate` 版本由 Server 发布节奏决定，本仓 9 包版本 lockstep 已由 `set-version.sh` + `check-boundaries` 低成本解决，无需 workspace 继承机制。

### 后续维护约定

- 新增 Rust 包：加入 `scripts/rust-pkg-dirs.txt`，按需补 Makefile `test-*` 分组与 CI 缓存；边界脚本自动纳入版本一致性检查。
- 改版本：只用 `scripts/set-version.sh`，不手改单个 Cargo.toml。
- 改交接键名：改 Rust 常量 + golden JSON（同一次提交），Rust 单测与 Playwright 共读 golden 防漂移。

## Consequences

- 新增包的成本从「改 4+ 处枚举」降为「改 1 处列表 + 按需补分组」。
- 边界违规（纯度破坏 / 默认 feature 拉入 Tauri / 钉形状漂移 / 版本不一致）在 pre-commit 与 CI 即被拒绝，不再依赖 reviewer 记忆。
- `rust-pkg-dirs.txt` 成为关键文件：被删除或格式破坏会让 fmt/clippy/clean 静默空转——`check-boundaries` 对它做存在性校验，`check.sh` 消费前也校验。
- 若未来包数量显著增长、或出现真正的根级共享代码需求，可重新评估根 workspace；本 ADR 的边界脚本与列表文件在两种形态下均可复用。
