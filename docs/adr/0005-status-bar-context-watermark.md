# ADR-0005: 状态栏「会话上下文用量」芯片的水位语义

## Status

Accepted

## Context

状态栏的「会话上下文」芯片需要回答一个问题：**当前会话离撑爆模型窗口还有多远**。先前的实现有三个缺陷：

1. **口径不一致**：分子取服务端 `used_input_tokens`（含附件、工具 schema、vendor 开销），分母却是完整窗口上限；用户看到的「填充比例」与真实窗口占用对不上，且附件计入分子会把比例虚高。
2. **只涨不落**：若只记滚动峰值，服务端压缩 / 裁剪历史（含工具输出压缩、摘要）后水位永不回落，无法反映压缩后的真实占用。
3. **波动噪声**：L5 skill 每轮按用户消息 Top-K 重选，`system` 体量随之变化，`used` 会在相邻回合小幅上 / 下抖动；直接显示原始快照会让数字来回跳。

**代码事实（Client + 已钉 `crabmate` 0.6.0）**：

- 服务端 `used_input_tokens = message_tokens + tool_schema_tokens + attachment_tokens + vendor_overhead_tokens`。Server DTO **不含** `vendor_overhead_tokens`，故固定开销 `F = used − (message_tokens + attachment_tokens)` 只能**反推**。
- 服务端 `timeline_log { kind, title, detail }`；裁剪历史 / 工具输出压缩 / 摘要**共用唯一** `kind=context_trim`；`detail.after_tokens` = 压缩后的 `used_input_tokens`。
- 轴 `used` 的来源 `chat.conversation_prompt_tokens` **只在回合末**（`RUN_FINISHED` 帧）刷新；`context_trim` 在**回合中途**到达。

## Decision

采用「**同口径显示 + 会话内滚动峰值 + 压缩 / 裁剪后回落，回落延迟到回合末可见**」。

1. **同口径**：芯片显示同一口径的 `used / raw_cap`；meter 分两段——灰段 = 固定开销 `F`（相对窗口上限），brand 段 = 会话内容。
2. **会话内滚动峰值**：`ContextUsedWatermark { key, max_used }`，`key = 服务端 conversation_id`；`observe` 取滚动最大（单调不减），cid 变化即重置；施加时要求 `active cid == watermark.key()`，避免把别的会话的峰值施到当前会话。
3. **压缩 / 裁剪后回落**：收到 `kind=context_trim`、能解析出 `detail.after_tokens`、本轮流正被查看（`is_bound_session_active()`）、且存在服务端 cid 时，调 `ContextUsedWatermark::reset(key, after_tokens)` **无条件置位（允许回落）**——一次覆盖裁剪历史 / 工具输出压缩 / 摘要三类场景。
4. **判定抽为纯函数**：`context_trim_watermark_reset(bound_active, after, cid)`（`frontend/src/timeline_scan.rs`），与 `context_trim_after_used_tokens` 同处，便于单测；接线处只做取值与 `update`。

### 聊天主列的 trim 卡片形态

同一 `kind=context_trim` 事件还会在聊天主列渲染一张旁注卡（受 `show_turn_context_inject` 开关控制，默认隐藏，与 `context_inject` 共用展示路径）：折叠行显示已用 / 可用上下文（缺窗口字段时退化为已用 tokens），展开层为结构化字段 + 原始 JSON 二级折叠。本 ADR 只约束状态栏的水位语义，卡片形态见实现与 `CHANGELOG.md`。

### 已知时序特性（刻意保留，勿当 bug「修掉」）

`context_trim` 中途到达即把水位 reset 到 `after_tokens`，但显示分子 `axis.used` 仍是**上一回合末**的快照；`context_axis_apply_watermark` 做 `axis.used.max(watermark.used())`，故**中途显示维持旧峰值，回落要到本轮末 `RUN_FINISHED` 快照刷入后才可见**。

间隙内 `max()` 维持旧峰值是刻意的：`max` 的作用正是压制 L5 skill 波动带来的「非裁剪性下降」。**不得**为消除回落延迟而删掉 `max`——那会把波动噪声放回来。

## Consequences

**正面**：

- 填充比例可核对到真实窗口占用，附件不再虚高。
- 压缩 / 裁剪后能回落，反映当前真实占用。
- 相邻回合的 skill 波动被 `max` 压制，数字稳定。

**代价 / 约束**：

- 回落延迟到回合末快照才可见（见上「已知时序特性」）。
- 水位按会话**单槽**记忆：切换会话即重置（芯片 tooltip 已声明）；跨会话峰值不保留。
- `reset` 的归属取自 `server_conversation_id_for_tokens()`（绑定会话记录优先，回退全局 `session_sync` 槽）；apply 侧以 key 相等守卫，故即便回退到非活跃 cid 也不会误施到活跃会话。
- 契约权威在 Server：`context_trim` / `after_tokens` 形状若变，本仓随之调整；本仓**不**发明线协议字段。

## Alternatives Considered

| 方案 | 否决原因 |
|------|----------|
| **A. 只显示原始快照（不记水位）** | L5 skill 每轮重选导致数字上下抖动，误导 |
| **B（本决策）. 同口径 + 会话内滚动峰值 + 压缩/裁剪后回落** | 见上 |
| **C. trim 时立刻可见回落（apply 用 `min` / authoritative 标记）** | `after_tokens` 无拆分字段，中途灰段 `F` 无从重算，只能整条按 `F=0` 渲染 → 短暂视觉伪影；暂缓，待产品需要再评估 |
| **D. 每会话水位 Map（保留各会话峰值）** | 当前无「跨会话比较」需求，tooltip 已声明切换重置；按需再加 |

## 关联

- 状态栏实现：[`frontend/src/app/status_context.rs`](../../frontend/src/app/status_context.rs)
- 水位结构：[`frontend/src/chat_session_state.rs`](../../frontend/src/chat_session_state.rs)
- 解析与判定纯函数：[`frontend/src/timeline_scan.rs`](../../frontend/src/timeline_scan.rs)
- `context_trim` 接线：[`timeline_dispatch.rs`](../../frontend/src/app/chat/composer_stream/callbacks/builders/timeline_dispatch.rs)
- 旁注「显示每轮注入与窗口裁剪信息」开关（与 `context_inject` 共用展示路径）：[ADR-0003](./0003-chat-file-context-inject-cards.md)
- 线协议：Server [`SSE协议.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/SSE协议.md)；本仓只链接
