---
name: 功能建议 / Feature request
about: 描述需求与使用场景 / Describe the need and the use case
title: ""
labels: ["enhancement"]
---

<!-- 标题格式：<type>(<optional scope>): <中文简述> / <English brief>，如
     feat(git): 侧栏显示分支与脏状态 / show branch and dirty state in the sidebar
     提交前请删除本注释。 -->

## 背景 / 问题 / Background

<!-- 现在的做法哪里不够用、在什么场景下卡住。有对照产品（VSCode / Cline / Aider 等）请说明差异。 -->

## 期望能力 / Desired capability

<!-- 用户可见的结果，而不是实现方案。写清成功时「用户能看到 / 能做到什么」。 -->

## 涉及端 / Surfaces

<!-- 勾选需要覆盖的端；未勾选的端视为本次非目标，避免范围蔓延。 -->

- [ ] Desktop（`desktop-tauri` + `frontend`）
- [ ] Android（`mobile-tauri` + `frontend`）
- [ ] Web 浏览器（`web-host` + `frontend`）
- [ ] TUI（`crates/crabmate-tui`）
- [ ] 共享纯逻辑（`crates/crabmate-client-api` / `crates/crabmate-tool-card`）

## 是否需要 Server 侧改动 / Server dependency

<!-- 本仓只维护官方壳与业务 UI；HTTP / SSE / 错误码 / 工作区语义的权威在 Server 仓。 -->

- [ ] 不需要：纯 Client 侧（UI / 交互 / 本地逻辑）
- [ ] 需要：需先在 **Server** 仓开 issue 定义契约（路由 / 字段 / 错误码），打 `client-contract-v*` 后本仓升级 pin（见 `docs/design/contract_pin.md`）
- 需要的 Server 能力（用验收语言描述，不要在本仓写死字段名）：

## 可能的实现方向（可选）/ Possible approach

<!-- 落点模块、可复用的既有能力、已知约束（WASM 体积 / 窄屏锁死 / 契约未 pin 等）。 -->

## 非目标（可选）/ Non-goals

<!-- 明确划出去的相关能力，避免 PR 里悄悄扩大范围。 -->

## 是否愿意提交 PR / Willing to open a PR

- [ ] 是
- [ ] 否
