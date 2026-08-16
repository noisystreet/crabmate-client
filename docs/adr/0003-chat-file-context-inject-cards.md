# ADR-0003: 聊天区展示「从文件注入」的上下文卡片

## Status

Proposed

## Context

用户希望在对话里看见 **skill / 上下文注入** 用了什么，而不是只有工具卡。本仓路径 A 薄壳不变：执行与组装权威在已运行的 `crabmate serve`；本仓不得发明线协议字段并当作契约。

**代码事实（Client + 已钉 `crabmate` 0.4.0）**：

- Skill **不是** `tool_call`。L2 把 skills 目录编成索引写入 `system`；L5 按本轮用户消息 Top-K（或 `/<skill-id>` 强制）把 skill **正文**叠进 `system`。聊天 SSE（[Server `SSE协议.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/SSE协议.md)）无 `skill_*` 事件。
- 用户显式 `/<id> [任务]`：存盘仍是斜杠原文；展示侧已有用户气泡内联 chip（[`skill_slash_display.rs`](../../frontend/src/message_format/skill_slash_display.rs)、[`tui_transcript_sync.rs`](../../frontend/src/app/chat/tui_transcript_sync.rs) `skill_slash_body_chunks`）。Composer `/` 浮层来自 `GET /skills`（含 `id` / `description` / `path`）。
- 首轮工作区上下文（活文档、备忘文件、项目画像、依赖摘要）合成 **一条** `role=user`，`name=crabmate_first_turn_workspace_context`（Server `cm_types`）。其中 **从文件读** 的是：
  - `.crabmate/living_docs/` 下 `SUMMARY.md` / `map.md` / `pitfalls.md` / `build.md`（可改相对目录）
  - `agent_memory_file` 指向的备忘文件
  - 项目画像与依赖摘要是 **扫描生成**（工作区 walk / `cargo metadata` / `package.json`），不是「把某个用户文档整篇注入」
- `GET /conversation/messages` 经 `filter_messages_for_web_client_snapshot` **不向 Web 下发** 上述 `user.name` 注入条（与长期记忆、变更集、规划纠偏同类）。本仓水合若见到 `crabmate_first_turn_workspace_context` 会 **跳过**（[`conversation_hydrate.rs`](../../frontend/src/conversation_hydrate.rs)）。
- 另一套隐藏规则是正文前缀启发式：[`cm_display_rules::user_message_should_hide_for_chat_display`](https://docs.rs/crabmate/0.4.0/crabmate/cm_display_rules/)（规划拒绝、步级反馈、plan rewrite、LTM 前缀、编排纠偏）。**不含** L6 文件块（L6 靠 `name` 过滤，根本到不了 UI）。
- 变更集已有侧栏 Markdown 弹窗（`GET /workspace/changelog`），不是消息卡。调试台 `thinking_trace.context_snapshot` 是工具前后摘要，不是 L5/L6 文件清单。

**要回答的问题**：聊天区要不要、以及如何展示「从文件注入」的上下文，而不把注入全文伪装成用户发言、不在本仓发明 SSE 形状。

## Decision

采用「**压缩卡片 + 只列文件来源；全文默认不进气泡；线协议由 Server 定**」。

### 产品范围（本 ADR 要显示的）

只把 **读盘注入** 做成对话里的上下文卡：

| 显示 | 不显示（继续藏或走现有入口） |
|------|------------------------------|
| 本轮 / 本会话首轮读过的 living_docs 相对路径 | 规划拒绝、编排纠偏、plan rewrite 注入 user |
| agent 备忘文件相对路径 | 长期记忆 **召回** 条（不是工作区备忘文件） |
| 用户 `/<skill-id>`：chip 可附 `GET /skills` 的 `path` | Top-K skill **自动**叠进 `system`（无事件则不上卡） |
| （可选，须标「扫描」）项目画像 / 依赖摘要 | 把 L6 **全文**当用户/助手气泡 |
| | 用工具卡冒充 skill（无 `invoke_skill` 工具） |

卡片形态：助手时间线或回合元信息条上一张 **只读压缩卡**（例如「已从文件注入：`a.md` · `b.md`」），展开最多给标题/截断摘要，默认不渲染整篇 Markdown。不得看起来像用户说的话。

### 契约与依赖方向

1. **禁止**在本仓把 living_docs 标题启发式（如 `### 摘要（SUMMARY.md）`）写成 HTTP/SSE 契约。解析只可作 **过渡期展示**，Server 一旦下发 `{ kind, path }[]` 必须改吃结构化字段。
2. **禁止** Client 因工作区里「有这些文件」就画卡（存在 ≠ 本轮注入）。
3. L6 / 自动 L5 清单的权威形状归 **Server**（snapshot 旁路、SSE 事件或 OpenAPI DTO）。未 pin 前 Client **不上**「本轮自动注入了哪些 skill/文件」按钮式假数据。
4. 若 Server 选择把 `crabmate_first_turn_workspace_context` 放进 Web 快照：本仓停止「静默 skip」；按 `name` 识别后渲染上下文卡，**仍不**用普通 user 气泡展示正文。

### 本仓可先做（不改 Server）

**阶段 A — 用户斜杠 skill 文件来源**

- 保留现有 chip；若 `GET /skills` 缓存命中该 `id`，在 chip/`title` 或展开行显示 `path`。
- 未知 id（未进列表）只显示 id，不编造路径。
- 验收：`/foo 任务` 气泡仍是用户消息；chip 表示 skill；有 path 则可见相对路径。

### 须 Server 配合后再做

**阶段 B — 首轮文件注入卡**

Server 须提供下列之一（本仓不拍板字段名）：

- 快照中保留带 `name=crabmate_first_turn_workspace_context` 的消息（或等价 `display_*`），或
- 单独元数据：本轮/本会话已注入的 **相对路径列表**（可含 `kind`: living_docs / memory_file），**不必**下发全文。

Client：按列表画卡；无列表则不画。水合测试从「skip 该 name」改为「映射为上下文卡」。

**阶段 C — 自动 Top-K skill（可选）**

仅当 Server 下发本轮选中的 skill id/path。不得从 `system` 正文正则抠。

### 明确不做

- 在本仓新增 SSE event type 或 OpenAPI path 并当作权威。
- 把 L6 全文当用户消息或工具卡。
- 为对齐 Desktop 在 Android 上另做一套注入调试台。
- 用 Client 再读一遍 living_docs 目录来「同步」注入内容（与 Server 预算/截断会漂）。

## Consequences

**正面**：

- 「模型为什么知道这些」可核对到具体文件，而不污染用户气泡。
- 与 display-rules / snapshot 过滤的目标一致：注入不是用户发言。
- 阶段 A 可独立交付；阶段 B/C 等待 pin，避免假卡片。

**代价 / 约束**：

- 未做 Server 工作前，首轮 living_docs **仍然不可见**；文档不得声称已支持。
- 过渡期解析 Markdown 文件名会在 Server 改文案时失效，必须在 pin 升级时改掉。
- 卡片若展开摘要，仍可能带工作区内容；默认折叠；日志/E2E 不要 dump 全文。
- 实现用户可见行为时：`CHANGELOG.md` `[Unreleased]`；若改变启动/能力描述则同步 `README.md` + `README.zh-CN.md`。本 ADR 落地前只记设计。

## Alternatives Considered

| 方案 | 否决原因 |
|------|----------|
| **A. 维持全藏（现状）** | 用户无法确认文件是否进上下文 |
| **B（本决策）. 压缩卡 + 只列文件；A 可先做 slash path；B/C 等 Server 元数据** | 见上 |
| **C. 快照原样展示注入 user** | 长文伪装成用户；与 `is_message_visible_in_chat_transcript` 冲突 |
| **D. Client 扫描 `.crabmate/living_docs` 自行画卡** | 存在 ≠ 注入；截断预算与 Server 不一致 |
| **E. 把 skill 做成假 `tool_call` 卡** | 无此工具；会误导审批/打开文件等 Wave 2 路径 |
| **F. 本仓发明 `context_inject` SSE** | 契约权威在 Server；薄壳禁止先行定字段 |

## 实施计划

### 阶段 A（仅 Client）

- chip 旁 path：`composer_slash` skills 缓存 + `skill_slash_body_chunks`。
- 单测：有/无 cache 命中；`/skills` 保留词仍不走 skill chip。

### 阶段 B（双仓）

- Server 先合元数据或放行 named user；再打 tag / 升本仓 pin。
- Client：水合映射为上下文卡；禁止当 plain user。
- 窄屏：列表+折叠即可，不解锁 IDE。

### 阶段 C（可选）

- Top-K skill 列表事件 → 同一套上下文卡组件（kind=skill）。

### 回归

- `cd frontend && cargo test --lib`（hydrate / skill slash）。
- 手测：新会话有 living_docs 时，**在 Server 放行前**不应出现假卡；放行后只出现文件路径卡。

## 关联

- 编程 Agent 审查面：[`docs/design/coding_agent_client.md`](../design/coding_agent_client.md)（不替代本 ADR；还原/changelog JSON 仍归 Server）
- 展示隐藏规则：Server `cm_display_rules`；本仓 [`session_merge.rs`](../../frontend/src/app/chat/session_merge.rs)、[`message_ex/parts.rs`](../../frontend/src/message_format/display/message_ex/parts.rs)
- 水合 skip：[`conversation_hydrate.rs`](../../frontend/src/conversation_hydrate.rs)
- 首轮合并（Server 0.4.0）：`cm_internal::context_bootstrap::first_turn_inject` / `living_docs`
- 线协议：Server [`SSE协议.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/SSE协议.md)；本仓只链接
