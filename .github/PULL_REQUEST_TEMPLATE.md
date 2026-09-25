<!--
PR 标题：Conventional Commits + 中英双语，例如：
  refactor(client-api): 下沉 chat body 可选块取值规则至共享层 / move chat-body optional-block value rules to shared client-api
提交前请删除本注释。
-->

## 概要 / Summary

<!-- 这个 PR 做了什么、为什么这么做；关联 issue / 设计文档请附链接。 -->

## 改动面 / Scope

<!-- 主要文件或模块；跨端（frontend / tui / connect / desktop / mobile / web-host）请标明。 -->

## 行为变化 / Behavior changes

<!-- 用户可见行为变化写在这里；无行为变化写「无 / none」。 -->

## 验证 / Verification

<!-- 贴关键证据：通过的检查、测试数量、手动冒烟步骤等。 -->

- [ ] `make check`（含 frontend `wasm32` clippy）通过
- [ ] `make test` 通过
- [ ] `bash scripts/check-boundaries.sh` 通过
- [ ] 涉及复杂度 / 规模时：`scripts/lizard-rust.sh`、`scripts/fn-param-ratchet.sh`、`scripts/fn-nloc-ratchet.sh` 通过
- [ ] 涉及浏览器 / 壳交互时：`./scripts/e2e-playwright.sh` 或 `./scripts/victauri-e2e.sh all` 通过
- [ ] 手动冒烟步骤（如适用，见 `docs/design/shell_smoke_runbook.md`）：

## 自检 / Checklist

**边界与契约 / Boundaries & contracts**

- [ ] 未新增通往 Server 仓的 `path` 依赖；未捆绑 / 拉起 `crabmate serve` sidecar
- [ ] 契约钉点未动（crates.io `crabmate` 0.5.2 + `default-features = false, features = ["protocol"]`）；确需改动时已同步 `docs/design/contract_pin.md`
- [ ] 未手改单个 `Cargo.toml` 版本号（版本锁步走 `scripts/set-version.sh`）
- [ ] 新增 Rust 包已登记到 `scripts/rust-pkg-dirs.txt`（如有）
- [ ] `crabmate-client-api` 仍为纯逻辑（无 Tauri / `web-sys` / `reqwest` / `tokio`）；`crabmate-connect` 默认 features 仍无 Tauri

**前端 CSS 门禁 / Frontend CSS gates**

- [ ] 响应式断点仍与 `MOBILE_LAYOUT_BREAKPOINT_PX` 单一来源一致（`scripts/check-css-breakpoints.sh`）
- [ ] 新增 / 修改的类名在同 scope 有 CSS 规则，或登记到 `scripts/css_contract_allowlist.txt`（`scripts/check-css-contract.sh`）
- [ ] `var(--x)` 的 token 已定义、`var()` 兜底无颜色字面量、各主题覆盖齐备（`scripts/check-css-tokens.sh`）
- [ ] 字号 / 颜色字面量预算已同步下调（`scripts/css_literals_budget.txt`，`scripts/check-css-literals.sh`）

**文档 / Docs**

- [ ] 用户可见的壳行为变化：`README.md` ↔ `README.zh-CN.md` 已同改动同步（含对应的 `desktop-tauri/` / `mobile-tauri/` README）
- [ ] 能力变化：`docs/design/client_capability_matrix.md` 对应单元格已更新
- [ ] `CHANGELOG.md` 的 `[Unreleased]` 已补英文条目（面向用户 / 维护者，非逐条罗列 commit）
- [ ] 设计 / 架构 / 测试等文档已按 `AGENTS.md` 的文档规则更新

**提交 / Commits**

- [ ] 提交前已跑 `pre-commit run --all-files`（或 `bash scripts/check.sh`）并通过
- [ ] 未使用 `git commit --no-verify`
- [ ] commit subject 为 Conventional Commits + 中英双语
