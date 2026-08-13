# ADR-0001: 流式进行中允许切换会话（后台流）

## Status

Proposed

## Context

**Issue #28（P1）**：流式回复进行中点击其它会话没有门闸。SSE 仍绑定旧会话写入，而当前 UI 的全局 session sync 会切到新会话，可能造成后台半成品、状态错位与后续分支/再生异常。

**现状架构事实**（`frontend/`）：

- **单流模型**：[`ChatStreamTransport`](file:///home/gzz/crabmate/crabmate-client/frontend/src/chat_session_state.rs#L43-L99) 是单个 `RwSignal`，同一时刻至多一个 `/chat/stream` attach；`TurnLifecycleState`、`stream_text_overlay`、`session_sync` 均为全局单例。
- **写回不串会话**：SSE 回调按 attach 时绑定的 `bound_stream_session_id` 定位 `sessions` 记录写入（[`stream_session_access.rs`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer_stream/callbacks/stream_session_access.rs)），正文本身不会写进错误会话。
- **水合被推迟**：流式或 overlay 未收尾时 `defers_conversation_hydration_untracked()` 为真，切换会话不会与水合竞态。
- **新工作有全局门闸**：发送 / 分支 / 再生在忙时均被拦截，流式期间其它会话不能发起新 attach。

**根因（代码事实）**：

1. [`apply_shell_after_active_session_changed`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer.rs#L36-L66) 在每次 `active_id` 变更时**无条件调用 `clear_stream_resume_handles()`**，把 Bound 车道打回 Idle 并清零 SSE 序号。后果：页面后台化后回前台时 [`spawn_foreground_stream_resume`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer_stream/foreground_resume.rs#L50-L123) 读不到 `stream_bound_resume_handles_untracked()` → 流无法软续传，后台半成品、loading 卡死；同时破坏「Bound 会话 == attach 快照」调试不变量。
2. [`on_cid` / `on_conv_rev`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer_stream/callbacks/assemble.rs#L57-L95) 总是写入全局 `session_sync` 槽。用户切到 B 后，A 的流仍把 A 的 `conversation_id`/`revision` 写进全局槽；若 B 为纯本地会话，流结束后无人重置，B 的下一次发送会在 attach 处读到 **A 的 conversation_id**（[`composer_stream/mod.rs`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer_stream/mod.rs#L80)）→ 错写 A 的服务器会话，即「状态错位 / 分支再生异常」。
3. 侧栏没有任何「哪个会话仍在生成」的指示。

**约束**：

- 维持单流模型，不做多并发流（每会话独立 transport/overlay/busy 属更大重构，超出本 issue 范围）。
- 不破坏既有写入、水合、门闸不变量。
- 验收标准：切换不丢/不错写 SSE 内容；UI 明确指示生成中会话；分支/再生使用正确 conversation id/revision；新增流式中切换并切回的 E2E。

## Decision

采用「**正式支持后台流**」而非「流式期间禁用切换」，共 5 项改动：

1. **切换保留后台流重连句柄**：[`apply_shell_after_active_session_changed`](file:///home/gzz/crabmate/crabmate-client/frontend/src/app/chat/composer.rs#L36-L66) 仅当 `stream_bound_resume_handles_untracked().is_none()` 时才调用 `clear_stream_resume_handles()`；Bound 期间保留 `job_id`、SSE 序号与 overlay。流结束时由 `on_stream_ended` / `on_error` 自行清 lane，不残留状态。
2. **全局 `session_sync` 槽与活跃会话隔离**：`on_cid` / `on_conv_rev` 总是写绑定会话记录（`server_conversation_id` / `server_revision`），**仅当 `bound_stream_session_id == active_id`** 时才同步写全局槽（`ChatStreamCallbackCtx::is_bound_session_active()`）；切回时由切换 Effect 从会话记录重推导全局槽，语义自洽。
3. **侧栏「生成中」指示**：`session_row_item_class` 增加 `streaming` 参数 → 追加 `is-streaming` class；`nav_session_row_button` 在 `stream_transport.bound_session_id() == Some(row.id)` 时渲染 spinner + 「生成中…」badge（带 `data-testid` 供 E2E）。
4. **删除会话守卫**：拒绝删除仍被 Bound 的会话（否则 SSE 的 `find(|s| s.id == sid)` 找不到写入目标，内容静默丢失）。
5. **测试**：纯函数单测（`should_clear_resume_handles`、`sync_global_when_active`、`session_row_item_class` 新分支）+ E2E（mock 分块 SSE：流式中切到 B → 后台完成 → 切回 A 断言完整正文；断言 `POST /chat/branch` 携带 A 的 cid/rev）。

## Consequences

**正面**：

- 满足 issue 全部验收标准。
- 保留重连能力：桌面 / Android 切后台后回前台，后台流可软续传，不再丢流。
- 分支 / 再生永远使用正确 conversation id / revision。
- 用户体验：生成期间可查看其它会话。

**代价 / 约束**：

- `session_sync` 语义收窄为「活跃会话快照」；后续代码不得再把后台流的同步状态写入全局槽，必须走会话记录。
- 删除 Bound 会话被拒绝，用户需先停止或等待流结束。
- 仍是单流模型：流式期间其它会话不能发起新发送（与现状一致，非回归）。
- 后台流的增量在非活跃会话视图不可见（overlay 按 `parent_session_id` 过滤展示），由侧栏指示补偿。

## Alternatives Considered

| Option | Pros | Cons |
|--------|------|------|
| **方案 A：流式期间禁用切换并提示** | 改动极小、风险最低、与现有全局忙门闸一致 | 生成期间无法查看/操作其它会话；不满足「查看后台进度」诉求；未修 `clear_stream_resume_handles` 潜在误用 |
| **方案 B（本决策）：后台流** | 保留查看与切换能力；重连句柄不再被误毁；同步语义清晰可单测 | 改动中等；需严格守卫全局 sync 写入语义 |
| **方案 B'：多并发流**（每会话独立 transport / overlay / busy / sync） | 能力最强 | 需重构单流全局模型（`stream_transport` / `stream_text_overlay` / `TurnLifecycleState` 均单例），风险与成本高，超出本 issue 范围 |

## 实施计划

### 阶段 0 — 前置确认

- `session_row_item_class` 仅 1 处调用（`frontend/src/app/sidebar_nav/session_rail.rs`），签名扩展无连锁影响。
- i18n `pub use sidebar::*`，新增函数可直接以 `crate::i18n::*` 暴露。
- `clear_stream_resume_handles` 的其余调用点（`status_agent_role_menu.rs`、`status_session_mode_seg.rs`、`session_workspace_partition.rs`）均为「更换角色 / 模式 / 工作区」的有意中止，保持现状。

### 阶段 1 — 切换保留后台流重连句柄

`frontend/src/app/chat/composer.rs` `apply_shell_after_active_session_changed`：

- 在 `ChatStreamTransport`（`frontend/src/chat_session_state.rs`）新增判定方法：

```rust
/// 仅当无在途流（Idle）时才允许清空重连句柄；Bound 期间清空会丢失
/// job_id / SSE 序号 / overlay，导致后台流切后台后无法软续传。
#[must_use]
pub(crate) fn resume_handles_clear_allowed(&self) -> bool {
    self.bound_session_id().is_none()
}
```

- `apply_shell_after_active_session_changed` 改为 `if chat.stream_transport.get_untracked().resume_handles_clear_allowed() { chat.clear_stream_resume_handles(); }`。
- 覆盖所有切换路径：侧栏、搜索命中、新建会话、删除会话自动跳转（都经 `active_id` 变更触发此 Effect）。
- 单测：Idle → true；Bound（含 / 不含 job_id）→ false。

### 阶段 2 — 全局 `session_sync` 槽与活跃会话隔离

- `frontend/src/app/chat/composer_stream/context.rs`：新增纯门闸函数 + `ChatStreamCallbackCtx::is_bound_session_active()`：

```rust
/// 纯判定：`session_sync` 全局槽是否允许写入。
#[must_use]
pub(super) fn session_sync_global_gate(stale: bool, active_id: &str, bound_session_id: &str) -> bool {
    !stale && active_id == bound_session_id
}
```

```rust
/// 本轮 SSE 是否正被用户查看（即 UI active_id 仍是绑定会话且未过期）。
#[inline]
pub(super) fn is_bound_session_active(&self) -> bool {
    session_sync_global_gate(
        self.is_stale(),
        self.chat.active_id.get_untracked().as_str(),
        self.bound_stream_session_id.as_str(),
    )
}
```

- 为控制 `assemble::build_chat_stream_callbacks` 的 CCN（提取后回到上限内），`on_cid` / `on_conv_rev` 的闭包工厂拆至**新增文件** `frontend/src/app/chat/composer_stream/callbacks/builders/stream_sync_callbacks.rs`（`make_on_conversation_id_builder` / `make_on_conversation_revision_builder`）：全局槽更新包上 `if stream_ctx.is_bound_session_active()`；会话记录写入（`update_bound_session`）保持无条件。
- 单测：`session_sync_global_gate` 三分支（未过期且绑定 == 活跃 → 写；绑定 ≠ 活跃 → 不写；过期 → 不写）。

**配套修复（审查发现）**：回前台 resume 的 `conversation_id` 修正。后台流期间全局 `session_sync` 槽已被切换 Effect 重推导为其它会话，原 `spawn_foreground_stream_resume` 从全局槽取 `conversation_id` 会把绑定会话的流续到错误会话。改为 `ChatStreamCallbackCtx::bound_session_server_conversation_id()` 优先取**绑定会话记录**（`context.rs`），全局槽仅作回退（`foreground_resume.rs`）。

### 阶段 3 — 侧栏「生成中」指示

- `session_row_press.rs` `session_row_item_class(active, is_pinned, is_starred, streaming)` 追加 ` is-streaming`。
- `session_rail.rs` `nav_session_row_button`：class 闭包内 `let streaming = chat.stream_transport.get().bound_session_id() == Some(session_id_class.as_str())`；`nav-session-meta` 行渲染「生成中…」badge（`.nav-session-streaming-badge`），`data-testid="nav-session-streaming"` 供 E2E。
- `i18n/sidebar.rs` 新增 `session_row_streaming_label`（zh: 生成中… / en: Generating…）。
- `styles/shell-ds.css`：`.nav-session-item.is-streaming` + badge 脉冲动画。
- 单测：`session_row_item_class` 四参数组合。

### 阶段 4 — 删除 Bound 会话守卫

- `apply_delete_session` 当前签名**无 `stream_transport`**，需新增参数 `stream_transport: RwSignal<ChatStreamTransport>`；`delete_session_after_confirm` / `delete_session_immediate` 同步加参。
- 3 个调用点更新（均有 `chat: ChatSessionSignals`，传 `chat.stream_transport`）：`session_modal_row.rs`（管理会话弹窗）、`session_delete_hotkey.rs`（Delete / Shift+Delete）、`sidebar_nav/context_menus.rs`（右键菜单）。
- 守卫前置：`delete_session_after_confirm` 在弹确认框**之前**检查 `stream_transport.bound_session_id() == Some(id)` 并提示；`apply_delete_session` 内部保留防御性检查（拒绝并返回）。
- i18n 新增 `delete_session_streaming_blocked`（zh: 会话正在生成中，请先停止或等待完成）。

### 阶段 5 — E2E（新增 `e2e/specs/mock-stream-switch-background.spec.ts`）

沿用 `installDelayedMockSse` 分块 SSE + `getByTestId('nav-session-*')`（参考 `mock-mobile-shell.spec.ts`）：

1. seed A（active）与 B 两个会话；A 上发送，mock 分块 SSE 慢速输出（delay ≈ 150ms）。
2. 流式中断言 A 行含 `is-streaming` / `nav-session-streaming`。
3. 点击 B → 断言切换成功（B 行 `is-active`），不被阻塞。
4. 等待 A 流后台完成 → A 行 `is-streaming` 消失、状态栏「就绪」。
5. 切回 A → 断言完整正文可见、无 loading 残留（空助手泡数 = 0）。
6. **验收标准 3**：`installDelayedMockSse` 不暴露请求体，第 2 次发送改用**自定义 `page.route`** 拦截 `/chat/stream`，记录 `route.request().postDataJSON().conversation_id`，断言 == `"e2e-conv-A"`（字段名见 `frontend/src/api/chat_stream/http_request.rs`）。
7. 扩展断言（可选）：后台切 tab 再回前台 → A 流 resume（`x-stream-job-id` + `stream_resume_after_seq` 请求）。

### 阶段 6 — 回归验证

- `make check`（含前端 wasm、lint、单测）。
- 手动跑新增 E2E：`./scripts/e2e-playwright.sh` 过滤新 spec。

## 关联

- Issue: [#28](https://github.com/noisystreet/crabmate-client/issues/28)
- 主要改动文件：`frontend/src/app/chat/composer.rs`、`frontend/src/chat_session_state.rs`、`frontend/src/app/chat/composer_stream/context.rs`、`frontend/src/app/chat/composer_stream/callbacks/builders/stream_sync_callbacks.rs`（新增）、`frontend/src/app/sidebar_nav/session_rail.rs`、`frontend/src/app/sidebar_nav/session_row_press.rs`、`frontend/src/session_ops.rs`、`frontend/src/app/sidebar_nav/context_menus.rs`、`frontend/src/app/app_shell_effects/session_delete_hotkey.rs`、`frontend/src/session_modal_row.rs`、`frontend/src/i18n/`、`frontend/styles/shell-ds.css`
