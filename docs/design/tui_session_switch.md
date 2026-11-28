# 运行中切换会话（TUI）

> **状态**：已落地。**范围**：`crates/crabmate-tui` 的全屏 `tui` 模式。
> **不改** Server 契约（SSE / 审批 / 工作区语义）；Android / Web 不涉及。
> **关联**：[`client_capability_matrix.md`](./client_capability_matrix.md) 注 14、[`tui_usability.md`](./tui_usability.md)、
> [`tui_multi_server.md`](./tui_multi_server.md)（多 lane；本文是**一条 lane 内**的会话切换）。

## 1. 问题

切会话就是换一份 transcript。TUI 的 transcript 是 `UiState` 上的单个 `Vec<LogLine>`，于是「回合进行中切走」
原本被直接拒绝（`回合进行中：结束后再切换会话`）：不走这一步，在途 SSE 增量只能写进「当前会话的屏幕」，
也就是把 A 的回答画到 B 上。

Desktop 没有这个问题，因为它有三件 TUI 缺的东西：

| Desktop | 位置 | TUI 原有对应物 |
|---|---|---|
| 每条会话自带 `messages`（`sessions: Vec<ChatSession>`） | `frontend/src/storage.rs` | 无（`UiState.lines` 单份） |
| 流绑定会话：`ChatStreamTransportLane::Bound { session_id }` | `frontend/src/chat_session_state.rs` | 无（事件只带 `LaneId`） |
| 流式旁路缓冲带归属：`StreamTextOverlay { session_id, message_id }` | `frontend/src/stream_text_overlay.rs` | 无（增量直接 `push_line`） |

Desktop 的口径可以一句话概括：**切会话不打断在途回合；在另一个会话发新回合会 abort 旧的那条流**
（`reset_abort_state_for_new_attach`：`ac.abort()` + 抬 `attach_generation`）。也就是说它并**不**并行两轮，
只是把「收哪条流」与「看哪一份 transcript」解耦。

## 2. 做法（三个部件）

1. **事件带归属**：`UiEvent::Turn { session, ev }`（`tui_mode/lane_route.rs`）。包装变体而不是拆 `LaneEvent`，
   因为 `UiEvent` 有 25+ 变体、70+ 处构造与匹配点 —— 与 `Inbound::FromLane(LaneId, …)` 同一理由。
   归属由 `LaneTx::with_session` 在 `run_turn_job` 一处注入，回合代码一行都不用改。
   `TurnSession` 是**枚举**（`Global` / `Of(Option<String>)`）而不是 `Option<String>`：
   「不是回合事件」与「回合但首轮未绑定会话」两边都是 `None`，压成一个 `Option` 会让后者写进别人的 transcript。
2. **停泊槽**：`tui_mode/parked.rs`。切走时把整条会话（`st` / `body` / `cancel` / `resume` / `sent_turn` /
   `turn_started`）整体挪进 `parked: HashMap<Option<String>, ParkedSession>`，切回来直接接上，
   **不**重发 `GET /conversation/messages`（槽里就是最新内容，含正在流式的正文）。
   后台会话的增量走「换进 → 既有分派 → 换回」（`apply_to_parked`），因此 70 余处分支一行都没复制。
   进槽与换回都走「取 / 放」两个原语，而不是「park 到 map、再从 map 取回」：两条**未绑定**会话的键
   都是 `None`，后者会把正要处理的那一条覆盖掉。视图级的三样（输入框与它的**光标位置**、`palette`、
   会话列表与它的光标）**不**进槽 —— 尤其输入框：早期实现用 `set_input` 搬它，`set_input` 会把光标
   弹到行尾，于是后台每来一批增量就把用户正在打的字打断一次。
3. **发新回合先停旧回合**：`abandon_parked_turns`（`dispatch_turn` 开头）。对齐 Desktop 的 abort 语义：
   一个连接同时只跑一轮。TUI 的 worker 是串行的，不停它的话新回合只会排在后面，而屏幕上看起来「已经在跑」。
   同一个函数也挂在收尾入口 `wind_down` 上（Ctrl+C / `/quit` / `/lane detach`）—— 不收停泊槽里的回合，
   就是「屏幕上说停了、实际还在跑」，serve 侧白跑一轮、worker 也要等整轮跑完才退出。
   与 `wind_down` 的另一半分工：它动连接级 `JobCancel`（拉取也算它的账），这里只按回合取消。
4. **停泊会话的两个「看不见」要补说一句**：审批浮层画在它自己的 `st` 上，与跨 lane 的情况同一条理由
   （issue #337）——`announce_parked_approval` 在当前屏留一行提示，否则就是最长 600 s 的静默停摆；
   回合结束时也**不**自动发它排队的消息（`drafts::flush_queued_after_turn` 看 `in_borrow`），
   否则会在用户看不见的会话里开一轮新回合、抢占串行 worker。

## 3. 有意不随会话走的东西

| 项 | 为什么 |
|---|---|
| `st.sessions` / `st.selected` | 会话列表是**连接**级事实（刷新按 `serve` 拉，同 serve 各 lane 共用一份）；进槽会让侧栏切一次就空 |
| `palette` | `/settings` 选的外观，进程级；跟着会话走会「切一次退回默认配色」 |
| 输入框内容**与光标位置** | 属于「正在编辑的这一句」，草稿本来就由 `drafts` 按会话保存 / 载入；走 `set_input` 搬它会把光标弹到行尾 |
| 排队消息 | 属于会话（在 `st` 里，跟着槽走），但**不会**在借位态自动发出（见 §2 第 4 条） |
| `overrides`（`/model` `/mode` `/role`） | 本进程表达，与 `tui_multi_server.md` §4.1 的口径一致 |
| `index`（`/conv new` 在途状态） | 连接级（它描述的是「列表里那一条空行」） |

## 4. 非目标

- **同一会话两轮并发**：Server 侧同 `conversation_id` 无互斥（`tui_multi_server.md` §3 事实 6），Desktop 也不做。
- **跨会话两轮并发**：Desktop 发新回合即 abort 旧流，TUI 对齐它（见 §2 第 3 条）。
- **把 worker 改成并行**：上面两条都不需要，串行 worker 的语义保持不变。
- **会话级的 lane 行读数**（哪条**会话**在跑）：lane 行仍只描述连接；这是可见性增强，另开 Issue。

## 5. 验收口径（与 Desktop 对照）

| 场景 | 期望 |
|---|---|
| A 跑到一半切到 B | 视图换 B，A 继续收增量（进停泊槽，不写 B 的屏） |
| 在 B 滚动 / 搜索 / 看历史 | 正常；B 的 transcript 不受 A 影响；**输入框与光标不被打断** |
| 切回 A | 看到完整实时正文（含切走之后的部分），不重发历史请求 |
| 在 B 发新回合 | 先停 A（撤审批浮层 + `StreamCancel`），B 开跑 |
| A 在后台弹审批 | 当前屏留一行提示（`announce_parked_approval`），不静默等到 600 s 超时 |
| A 在后台跑完且有排队消息 | **不**自动发出；切回 A 才发 |
| Ctrl+C | 只停**当前**会话的回合 |
| `/quit` / `/lane detach` | 逐 lane 收尾；停泊槽里的回合一起被取消 |

单测落点：`tui_mode/parked.rs` 的 `tests`（增量不串台、接回不重发历史、未知会话丢弃、侧栏不进槽、
发新回合让位、**输入框与光标不被打断**、未绑定首轮不写进新空会话、收尾连停泊回合一起停、借位态不发排队消息）。

## 6. 未做 / 已知缺口

- 侧栏没有「这条会话正在生成」的标记：切走之后只有切回去才看得到进度。lane 行已有同形读数（`●`），
  会话行复用它是下一步。
- 未绑定会话（`/conv new` 之后的首轮）切走后**不能主动切回**：侧栏切会话只认
  `server_conversation_id`（`use_selected` 对还没绑定的行是拒绝的），所以只能等那一轮结束、绑定 id 后在列表里
  切回 —— 那时按 id 找不到槽，走历史回放补上（实时尾部由服务端历史补齐，不丢内容）。这也正是
  `on_turn_event` 直派条件要带 `st.running` 的原因：两条未绑定会话的归属都是 `None`。
- 停泊槽没有容量上限：`MAX_LANES` 是连接数，会话数是它 × 每个连接切过的会话数。极端情况下内存随切会话次数增长
  （每条会话一份 transcript），与 Desktop「所有会话都在内存里」同阶，暂不设上限。
