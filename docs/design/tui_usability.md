# TUI 易用性规划（对齐 Desktop 聊天体验）

> **状态**：Wave 1 已落地 W1.1 全屏 `/resume`、W1.2 忙时排队、W1.3 会话级草稿、W1.4 复制（OSC52 / 外部工具）、W1.6 多行粘贴（bracketed paste，`CM_TUI_PASTE=off` 可关）；**W1.5 会话列输入过滤仍未落地**（`session_filter` 等在 `tui_mode` 无对应字段）。W2.1 历史回放已落地**最新页**（`GET /conversation/messages`，更早的分页未做）；W2.2 **删除**已落地（左栏 `Delete` 先确认 / `Shift+Delete` 直删，在途回合拒绝），**只缺重命名**（无 serve 端点）；W3.1 `/` 补全菜单也已落地（`tui_mode/slash_menu.rs`：内建命令 + `GET /skills` 候选）。Wave 2 的 W2.3 / W2.4 与 Wave 3 其余项仍为方案。逐波按本文落地；每波独立 PR，落地后必须同 PR 更新 [`client_capability_matrix.md`](./client_capability_matrix.md) 对应格（§4）。
> **范围**：`crabmate-tui`（全屏 `tui` 为主，`repl` 相应联动）；候选与取舍参照 Desktop / Web（`frontend/`）聊天体验，**语义对齐，不追求 UI 复刻**。
> **关联**：[`remote_cli_tui.md`](./remote_cli_tui.md)（TUI 母方案，P1–P5）、[`client_capability_matrix.md`](./client_capability_matrix.md)（TUI 行 L53–L81）、[`tui_settings_panel.md`](./tui_settings_panel.md)（设置面板，W3 遗留）、[`chat_ui_todo.md`](./chat_ui_todo.md)（Desktop 聊天 UI 已勾项 = “已做过”的参照，不要当缺口重做）、[`contract_pin.md`](./contract_pin.md)（契约钉点）、Server `docs/`（SSE / user-data / 会话权威）

---

## 1. 背景与现状

### 1.1 Desktop 聊天易用性骨架（参照物，均已勾选落地）

`frontend/`（Desktop/Web 共用 WASM）围绕会话侧栏、消息区、composer 三处铺开的易用性能力：会话 **排序（置顶/星标）+ 标题过滤 + 跨会话全文搜索命中跳转 + 右键/长按菜单（管理/星标/置顶/导出/删除）+ 键盘删除**（`sidebar_nav/`、`session_list_modal.rs`、`session_search.rs`）；消息区 **回合右键菜单（复制/编辑/重试/再生/分支）+ 就地编辑 + 流式排队 + 代码块复制按钮 + stick-to-bottom + thinking/工具行折叠**（`message_turn_menu.rs`、`user_message_edit.rs`、`composer_follow_up.rs`、`md_code_copy.rs`、`chat_ui_todo.md` P1/P2 全部勾完）；composer **每会话草稿 flush/恢复 + `/` 斜杠浮层（含 skills）+ 澄清表单**。

### 1.2 TUI 现状与缺口对照

`crabmate-tui` 已具备：全屏 transcript + 状态栏、左栏会话 use/new/refresh（**并有鼠标：单击会话行即选中并切换**，滚轮刻意不碰列表）、右栏工作区目录树 + 项目池、审批浮层、thinking 折叠、工具行摘要、会话内 `/find`、多行输入（Alt+Enter 换行；粘贴一段多行文本时整段落进输入框，不会被当成多次回车提交）、设置面板（模型/会话两分区，W1+W2 落地）、repl `/resume` 断线续传。与 Desktop 对照，下列**用户高频操作缺口**是本规划的对象：

| # | 缺口（TUI 现状） | Desktop 参照 | 影响 |
|---|---|---|---|
| G1 | ~~全屏 `tui` 无 `/resume`~~ → **已落地**：断线 / 取消未送达时保留断点，控制斜杠表已有 `/resume`（`tui_mode/resume.rs`、`controls.rs`） | in-app stream resume（矩阵 L62） | 远程断线/取消未送达时全屏只能重开一轮，丢上下文 |
| G2 | ~~回合进行中 Enter 仅提示并保留输入~~ → **已落地**：忙时 Enter 进排队槽，回合**正常**结束后自动发出；Esc 清队；取消 / 出错则退回输入框（`tui_mode/drafts.rs`） | 忙时排队下一句 + 排队 chip + 结束后自动发出（`composer_follow_up.rs`） | 用户意图被打断，长回合场景需干等或反复重试 |
| G3 | ~~composer 为全局单缓冲，不保存/不恢复各会话草稿~~ → **已落地**：按 `conversation_id` 换入换出，`/conv new` 起空白（`tui_mode/drafts.rs`） | 每次切会话 flush 当前草稿、回来恢复（`composer.rs`、`session_ops.rs::flush_active_composer_draft`） | 遗留草稿会“串”到下一个会话；切走再回来内容丢失 |
| G4 | ~~无任何复制路径~~ → **已落地**（`tui_mode/clipboard.rs`）：`/copy [assistant\|user]`（别名 `/yank`）与 `Ctrl+Y`。**残余**：粒度固定为「最近一条非空消息」，不能指定中间某条；鼠标捕获占用了终端原生拖选（需 Shift+拖） | 消息 copy + 代码块「复制」按钮（`md_code_copy.rs`、`tui_actions_bar.rs`） | 终端里想取走一段代码/回复只能手工选中或重打 |
| G5 | 会话列表：左栏 use/new/refresh + **删除**（`Delete` 先确认 / `Shift+Delete` 直删，`tui_mode/sidebar_keys.rs`）；`/conv new` 已改为**写**服务端会话索引（`serve/sessions.rs`：`GET` → 复用或插入空白行 → 全量 `PUT`，首轮结束回填 `server_conversation_id`，切走回收空白行，首条消息按 Web 规则起名），左栏与 Web/Desktop 同步可见；仍缺重命名（**无 serve 端点**）、标题过滤、跨会话搜索、导出 | 排序 + 标题过滤 + 全文搜索 + 重命名/删除/导出（`session_rail.rs`、`session_list_modal.rs`） | 会话一多靠翻，无过滤手段 |
| G6 | ~~切换会话后 transcript 从空开始~~ → **部分落地**：切换 / 启动回放 `GET /conversation/messages` 的**最新一页**；`has_older` 时只提示「更早的消息暂不回放」，**没有「加载更早」**（`tui_mode/history.rs`） | 会话水合 + 顶部「加载更早」prepend（`session_hydrate.rs`、`tui_stream_view.rs`） | 续聊时看不到前文，须先切到别的壳回顾 |
| G7 | 无 retry / regen / branch / 编辑用户消息（`controls.rs` 无对应命令，`tui_mode` 里无 `regen`/`branch`） | 消息回合菜单（`message_row_actions.rs`、`user_message_edit.rs`） | 模型答错只能整轮重发或手动复制改写 |
| G8 | ~~审批浮层把命令截成一行且无法展开~~ → **已落地**：`Ctrl+E` 在 78×9 收起档与铺满主体区的展开档之间切换，展开档按显示宽折行（`text::wrap_display`）并支持 ↑/↓、PgUp/PgDn、Home/End 滚动，提示行显示「第 x-y / N 行」（`overlay.rs`） | 底部审批条可展开完整命令 + 失败保留重试（`approval_modal.rs`） | 长命令看不到全貌，易误判 |
| G9 | 澄清问卷（`clarification_questionnaire`）在 TUI 无交互 UI：`serve/chat_classify.rs` 只有 `clarify_trace` 钩子，没有表单 | 问卷表单（Web `frontend`） | 模型需要澄清时 TUI 这侧拿不到结构化提问 |
| G10 | 窄终端（< 120 列）下**当前工作区不可见**：左右栏按 `sidebar_visible` 隐藏（`layout.rs::panes`），而工作区名只画在右栏（`ws_sidebar.rs::workspace_title`），顶栏已删除（`tui_visual_polish.md` §11.1）；状态行（`status.rs`）与右栏状态栈都不带 workspace 字段 | 顶栏工作区名 + 面包屑 | 窄屏 / 分屏时不知道命令跑在哪个项目，得先 `/workspace` 查一次 |
| G11 | 上下文用量水位**压缩后不回落**：TUI 不解析 `timeline_log` `kind=context_trim`（`serve/chat_classify.rs` 只有正文 / 工具 / 审批钩子，`notice_timeline` 未接），而 `ContextUsedWatermark` 的 `observe` 只取滚动最大；Web 侧由 `reset` 借 `after_tokens` 把水位置回压缩后的值 | 把 `context_trim` 的 `after_tokens` 接到 `ContextUsedWatermark::reset` | 真发生压缩 / 裁剪后，状态行 `ctx` 仍停在旧峰值，直到切会话或重发流换 cid；用户会以为「压缩没生效」 |

另有与 Desktop 同步暂缓/不做项，见 §2 非目标（勿误当缺口）。

> **2026-10-25 代码核对**：G1–G10 已逐条对着代码验过（`resume.rs` / `drafts.rs` / `clipboard.rs` / `sidebar_keys.rs` / `history.rs` / `overlay.rs` / `ws_sidebar.rs` / `status.rs`），表内状态即当时事实。G10 为本次核对新发现的缺口（顶栏删除后的遗留债）；G5 的「导出」与 G7 的 retry/regen/branch 仍依赖 serve 端点，不属可独立落地项。

---

## 2. 目标与非目标

### 目标

1. 按「使用频率 × 实现成本」补齐 G1–G8 中的高频缺口，交互语义与 Desktop 对齐（键盘为主，遵循现有 focus 模型）。
2. **Wave 1 全部纯客户端**（无新 serve 契约、不动协议 crate），与现有 `running` gate、Ctrl+C 取消语义兼容演进。
3. 保持现有斜杠/按键兼容：新增一律是新命令或新按键，不重绑已有快捷键。
4. 落地即同步能力矩阵（§4），避免矩阵漂移。

### 非目标

- **不**做终端无意义或矩阵已 `no` 的 Desktop 项（不反向“对齐”）：图片附件/灯箱/拖放（矩阵 L58）、IDE 标签/编辑器/文件保存与重命名/文件夹下载/git clone/changelog 弹窗（L76–L82）、GitHub Device Flow（L48）。
- **不**做代码块语法高亮：Desktop P3 未勾（`chat_ui_todo.md` L45），TUI 围栏保持纯文本（矩阵 L53）。
- **不**复刻 Desktop 的 CSS 渲染类能力与文案细节；repl 不做全屏面板（保持入口引导语义）。
- 不碰设置面板 W3 遗留（MCP 全局开关/`tool_timeout_secs`、Session SQLite 开关）——归属 [`tui_settings_panel.md`](./tui_settings_panel.md)，本规划只引用不重复立项。
- Wave 1–3 的取舍是**建议顺序**，不是一次性全部交付的承诺。

---

## 3. 分期方案

> 验收统一口径：改动需在 `crates/crabmate-tui` 有对应单测（沿用 `render_tests.rs` / `settings_*_tests.rs` 等现有测试风格），并在真实 TTY 手测；无浏览器 E2E。涉及能力矩阵的项见 §4。

### Wave 1 · 纯客户端（无新契约，推荐先做）

#### W1.1 全屏断线续传 `/resume`（G1）— 已落地（`tui_mode/resume.rs`）

- **参照**：repl `/resume`（`main.rs:499-594` 断点记录 + `stream_resume:{job_id,after_seq}` 重挂）；Desktop in-app resume（矩阵 L62）。
- **方案**：把「断点记录」从 repl 提升为 turn 层通用状态——全屏回合意外中断（网络 `InterruptedStream` / 取消未送达）时在 `state.rs` 记录 `{text, conversation_id, job_id, after_seq}`，状态栏/系统行提示「断线：输入 /resume 续传」；新增 `/resume` 控制斜杠（`controls.rs` 解析 → `mod.rs` 以 `stream_resume` 重新 attach）。已确认取消/正常结束的回合不可续，与 repl 一致。
- **落点**：`tui_mode/controls.rs`、`tui_mode/mod.rs`（`start_turn`）、`tui_mode/state.rs`、`tui_mode/worker.rs`。
- **风险**：与 Ctrl+C「第一次取消回合」现有语义的边界（repl 已定义，沿用即可）。

#### W1.2 忙时排队发送（G2）— 已落地（`tui_mode/drafts.rs`）

> **实现**：队列只留最后一条（连按 Enter = 改主意），Esc 清队（仅在输入框焦点下；侧栏 / 工作区树里 Esc 先用于返回输入框）。Ctrl+C 仍是「取消回合」，语义不变。取消 / 出错的回合**不**自动发出，而是把文本退回输入框（`queued_outcome`）—— 用户在那时按 Ctrl+C 的意图通常是「停下来」，替他再发一句意外消息比多问一次更糟。回合进行中输入框照常可编辑，只有按 Enter 才入队。排队状态目前以 transcript 的一行系统提示呈现（`已排队：回合结束后自动发送（Esc 清队）`），常驻 chip / 状态行指示尚未做。

- **参照**：Desktop `ComposerStreamFollowUp::QueuedUserMessage` + 排队 chip（`composer_follow_up.rs`、`column.rs`）。
- **方案**：回合进行中按 Enter → 文本进入「排队」槽并显示状态行指示（如 `○ 排队中：将自动发送`），回合结束自动 `start_turn` 发出；队列非空时再按 Enter 覆盖或 Esc/Ctrl+C 清队。切换会话/新建时若队列存在先落为草稿（联动 W1.3）再清。
- **落点**：`state.rs`（新增 `queued_text`）、`mod.rs`（`on_submit`/回合结束回调）、`render.rs`（composer 上方一行指示）。
- **风险**：与「回合在跑拒绝切会话」（`mod.rs:176-204`）的一致性——队列随会话切换落地草稿即可。

#### W1.3 会话级草稿保留/恢复（G3）— 已落地（`tui_mode/drafts.rs`）

> **实现**：草稿按 `conversation_id` 存在内存表里，切会话 / `/conv new` 时换入换出；全空白视为无草稿。退出时仍由 `exit_flush` 把当前会话那一份写回服务端索引（本条只补「切换不丢」这一段）。**局限**：草稿不跨进程——切走的会话只在内存里有，退出后不保留。

- **参照**：Desktop `flush_active_composer_draft` / 切会话恢复。
- **方案**：composer 文本按当前 `conversation_id` 缓存：切会话时保存旧缓冲并载入目标会话草稿，`/conv new` 新建时清空草稿，发送成功后清空该会话草稿。杜绝草稿“串会话”（现状见 §1.2 G3）。
- **落点**：`state.rs`（`HashMap<conversation_id, String>` 草稿表）、`mod.rs`（`switch_to_conv`/`new_session`/发送成功回调）。
- **风险**：草稿仅本进程内存（repl 的 `/model` 等偏好同样进程级），不落盘——文档注明即可。

#### W1.4 复制（OSC52 + 外部剪贴板回退）（G4）— 已落地（`tui_mode/clipboard.rs`）

> **实现**：`/copy [assistant|user]`（别名 `/yank`）与 **Ctrl+Y** —— 后者在顶层热键表里，输入框 / 会话栏 / 工作区树三种焦点下都可用。复制目标是**最近一条非空消息**的**原始 Markdown 正文**（不是渲染后的文本）。投递按环境自动选路：SSH 会话（`SSH_CONNECTION` / `SSH_TTY` / `SSH_CLIENT`）走 **OSC52**（内容由本地终端执行；客户端跑在远端时只有这条路能把正文送回用户手边）；本地会话优先 `wl-copy` → `xclip` → `xsel`（能拿到退出码），都没有时回落 OSC52；tmux 下才加 passthrough 包裹（`ESC Ptmux;` + 内层 ESC 双写，服务端需 `allow-passthrough on`）；**screen 不转发 OSC52**，在那里只走外部工具、没工具就直接报错。OSC52 不能分块也没有回执，base64 超过 64 KiB 时结果行会标注「可能不完整」；外部工具存在但跑不通（无 `DISPLAY` / 无 wl 服务）时回落 OSC52 一次。`CM_TUI_CLIPBOARD=off|osc52|tool` 可强制路径，认不出的值按未设置处理。结果写一行系统提示（`已复制助手回复（12 行，经 终端剪贴板）` / `没有可复制的助手回复` / `复制失败：…`）。**与原方案的偏差**：按键从 `c` 改为 **Ctrl+Y**（输入框焦点下 `c` 就是打字，不能占用）；目标只做「整条消息」，**未做**「代码围栏块聚焦复制」（围栏边界尚未抽到 `md.rs`），也**不复制** thinking 行。

- **参照**：Desktop 消息 copy / 代码块复制按钮。
- **方案**：新增按键：当前「锚定/最近」对象复制——定义：`c` 复制**当前光标所在/最近一条助手消息**正文（纯文本，含已折叠 thinking 之外的部分）、代码围栏块聚焦时复制整块；复制协议优先 **OSC52**（kitty/wezterm/screen/tmux `set-clipboard` 支持），不支持且环境有 `wl-copy`/`xsel`/`xclip` 时回退调用，否则系统行提示「终端不支持剪贴板」。Esc 不干扰。
- **落点**：新增 `tui_mode/clipboard.rs`（纯序列化 + 探测），`render.rs`/`mod.rs` 接线；`md.rs` 暴露围栏边界。
- **风险**：OSC52 需终端显式允许（tmux `set-clipboard on`）；SSH 到远端时剪贴板属本地终端，语义正确无需处理。密钥/敏感内容复制是用户主动行为，与 Desktop 同语义。

#### W1.5 会话列输入过滤（G5 的一部分）

- **参照**：Desktop 侧栏搜索面板标题过滤（`search_panel.rs`）。
- **方案**：左栏聚焦时输入普通字符进入过滤词（顶部或底行显示 `filter:`），`↑/↓` 在过滤结果间移动、Enter 使用、Esc/Backspace 清空退出；`/conv list` 同理可加尾参过滤。
- **落点**：`state.rs`（`session_filter`）、`mod.rs`（左栏按键）、`render.rs`（左栏过滤态行渲染）。
- **风险**：小。窄屏左栏不可见时过滤不生效（保持现状）。

#### W1.6 多行粘贴（bracketed paste）— 已落地（`tui_mode/paste.rs`、`tui_mode/state_paste.rs`）

> **实现**：启动时下发 `EnableBracketedPaste`（`ScreenGuard` 退出时对称关闭；`CM_TUI_PASTE=off` 可关），键盘线程把 `Event::Paste` 转成 `UiEvent::Paste`，`on_paste` 一次性写进输入框（**不提交**）。归一化在 `UiState::insert_paste`：`\r\n` / 裸 `\r` 统一成 `\n`（多行判定只认 `\n`）、`\t` 展开四空格、其余 C0 控制符丢弃（裸 `\x1b` 会被渲染层当成未闭合转义序列）、单次上限 32K 字符（超出截断并在 transcript 提示）。落地前的问题：终端把粘贴当逐字符按键，换行变成 `KeyCode::Enter` 被当成提交，多行内容只发出第一行、剩下的落进新草稿；夹带的 `Tab` 还会被当成焦点切换。**未做**：图片/文件粘贴、多行粘贴后自动折叠成「已粘贴 N 行」占位。

- **参照**：Desktop composer 的粘贴（浏览器原生 `paste` 事件，整段进 textarea）。
- **落点**：`tui_mode/mod.rs`（开关 + `ScreenGuard`）、`tui_mode/event_loop.rs`（`paste_capture_enabled` / `enable_captures`）、`tui_mode/worker.rs`（事件转发）、`tui_mode/paste.rs`、`tui_mode/state_paste.rs`。
- **风险**：小。关掉括号粘贴即回到改造前行为（逐字符按键）。

### Wave 2 · 需确认 serve 会话/消息契约

> 前置：核对 serve 是否提供「会话消息分页读取」「会话删除/改名」端点（TUI `/conv list` 已证明会话列表存在；Desktop 水合数据源在 serve 侧）。未 pin 前不落地，禁止用猜测端点写代码。

#### W2.1 会话历史回放 + 「加载更早」（G6）— 最新页已落地，分页未做

- **参照**：Desktop `session_hydrate.rs` 水合 + 顶部「加载更早」prepend 且滚动锚定不跳（`scroll_shell.rs::compensate_after_prepend`）。
- **方案**：切换会话（Enter/`/conv use`）时按需拉最近一页历史渲染（复用行模型，thinking/工具行转摘要）；顶部「↑ 加载更早」翻页；prepend 时视口补偿。更新矩阵 L53 描述。
- **落点**：`state.rs`、`mod.rs`、`worker.rs`、`render.rs`；契约侧 tui `serve/` 模块（原 `crabmate-tui-core`）。
- **依赖**：serve 消息分页端点；行模型需能承载多页（`lines` 追加 + `prepend` 区）。

#### W2.2 会话重命名/删除（G5 的一部分）— 删除已落地，重命名待端点

- **参照**：Desktop 行内重命名 + 删除确认（`session_modal_row.rs`、`session_ops.rs`）。
- **已落地**：左栏 `Delete` 进入确认态（`y` 确认、其它键取消）、`Shift+Delete` 免确认直删，在途回合拒绝删除——对齐 Desktop 的两级快捷键（`tui_mode/sidebar_keys.rs`，纯逻辑有单测）。删除走契约钉的 `DELETE /conversation/{conversation_id}`（幂等 204）；repl 另有 `/conv delete [<id>|current]`（`slash.rs`），只删服务端记录、不动 Web 侧栏索引。
- **未做**：行内重命名——**仍缺 serve 端点**，客户端不能自造 `PUT`；左栏也没有过滤/搜索（见 G5 / W1.5）。
- **依赖**：重命名需 Server 先提供端点并在 `contract_pin.md` 钉住；不得猜路径。

#### W2.3 回合 重试 / 再生 / 编辑用户消息（G7）

- **参照**：Desktop `message_row_actions.rs`（regen/branch 优先 `POST /chat/branch`，NotFound/Conflict 走本地截断重试）、`user_message_edit.rs`。
- **方案**：transcript 引入轻量「回合边界」（assistant 块 ↔ 触发它的 user 行）；新增 `/regen`（重发最后 user 消息再生该回合）、`/edit`（编辑最后 user 消息后重跑）；失败助手消息 `/retry`。流式中全部禁用。
- **落点**：`state.rs`（回合索引）、`mod.rs`、`worker.rs`；`POST /chat/branch` 语义复用（契约已 pin，无需新端点）。
- **风险**：行模型改「回合感知」是本规划**回归面最大**的一项（render/scroll/find/折叠均遍历 `lines`），需先抽出回合段数据结构再动 UI。

#### W2.4 会话导出（Markdown / JSON）

- **参照**：Desktop `session_export.rs`（`cm_chat_export projection=display`；Markdown 与 CLI 同规则）。
- **方案**：repl `/conv export [json|markdown] <id>` → 写 stdout/文件；与 Desktop 导出的过滤规则子集对齐（跳过 system、工具带摘要）。
- **依赖**：会话历史读取端点（与 W2.1 同源）。

### Wave 3 · 锦上添花 / 开放决策

- **W3.1 斜杠补全列表** — **已落地**（`tui_mode/slash_menu.rs`）：composer 输入 `/` 前缀弹出候选（内建命令 `controls.rs` 表 + `GET /skills` 的 skill），↑↓ 选择 · Tab/Enter 采纳 · Esc 关闭；候选为空时底部说明 skill 为何缺失。
- **W3.2 审批浮层看完整命令** — **已落地**（G8）：`Ctrl+E` 展开/收起，展开档按宽度折行 + 可滚动，提示行给出位置；收起档的单行预览也把语义换行压成空格（不再因为 `Line` 里夹 `\n` 把提示行顶掉）。
- **W3.3 阅读增强**：OSC8 超链接（wezterm/kitty 可点击 URL）、工具行/thinking 详情整段展开（受 SSE 负载约束，先验证负载是否携带完整文本）。
- **W3.4 设置面板补齐 W3 遗留**：MCP 全局开关与 `tool_timeout_secs`、Session SQLite 开关——实现归属 [`tui_settings_panel.md`](./tui_settings_panel.md) §7，本规划仅列入口不展开。

---

## 4. 需随落地同步的能力矩阵格

| 落地项 | 矩阵位置（`client_capability_matrix.md`） | 变更 |
|---|---|---|
| W1.1 | L62「In-app stream resume after background」TUI 列/Notes | TUI 全屏补 `/resume`，Notes 补充（原仅 repl） |
| W1.2/W1.3/W1.4/W1.5 | L53「Full-screen TUI」Notes / L60「Control slashes」 | Notes 追加排队/草稿/复制/过滤描述 |
| W1.6 | L53「Full-screen TUI」Notes | 多行输入一栏补「整段粘贴经括号粘贴一次落地，`CM_TUI_PASTE=off` 可关」 |
| W2.1 | L53 Notes「switching sessions starts a fresh transcript」 | 改为「切换可翻历史」 |
| W2.2 | L62「Web session list + resume…」TUI 列/Notes | 删除已补（`Delete` / `Shift+Delete`）；重命名待 serve 端点 |
| W2.3 | L59「Ask/Plan/Act…」附近或新增行 | 视方案形态在 chat/tools 段新增或并入 Notes |
| W2.4 | L61 Notes | 补导出能力 |
| W3.1 | L60「Control slashes」TUI 列 | 已同步（TUI 有 `/` 补全菜单，含 skills 候选） |
| W3.2 | L55「`POST /chat/stream` + command approval」TUI Notes | 补 `Ctrl+E` 展开看完整命令（折行 + 滚动）、收起档单行截断 |
| W3.3 | L53「Full-screen TUI」Notes | 补「< 120 列时左右栏隐藏，当前工作区无指示」（G10，待落地） |
| W3.4 | L54「Full-screen TUI」Notes | 补状态栏 `ctx` 上下文用量段（`tiktokenPromptTokens`（`RUN_FINISHED` / `conversation_saved`） + 会话内水位；口径来自 S11 `client-api::context_axis`；段恒在，无实测时占位 `—`）——已同步 |
| W3.5 | 同格 Notes | 待补：TUI 不消费 `timeline_log` `kind=context_trim`，故压缩 / 裁剪后水位**不会回落**（只增不减，直到切会话或换 cid）。Web 侧由 `ContextUsedWatermark::reset` 处理，见 G11 |

> 矩阵维护规则：改了用户可见能力就必须在同 PR 更新对应格（§顶注），不允许“下个 PR 再补”。

---

## 5. 验证

- **单测**：Wave 1 以 `state.rs` / `controls.rs` 纯逻辑为主（断点记录、队列状态机、草稿表换入换出、过滤词匹配、OSC52 序列转义），沿用 `render_tests.rs` / `settings_*_tests.rs` 的写法。
- **手测**（需真实 TTY + 本地 `crabmate serve`）：
  - W1.1：`tui` 中 `Ctrl+C` 未送达取消（或断网）→ `/resume` 续上；已确认取消则不可续。
  - W1.2：长回合中输入第二句 → 出现「已排队」系统行 → 回合**正常**结束才自动发出；取消 / 出错应退回输入框；输入框焦点下 Esc 清队（在侧栏 / 工作区树里要先 Esc 回输入框）。
  - W1.3：会话 A 输入半句 → 切 B 再切回 A → 半句仍在；`/conv new` 后为空。**已知局限**：草稿只在内存，退出进程即丢（`exit_flush` 只写当前会话那一份），切走的那几个会话的草稿不跨进程保留。
  - W1.4：kitty/wezterm 下复制助手消息/代码块 → 粘贴到编辑器；不支持终端给出提示。
  - W1.5：左栏输入字母过滤 → 选中使用；清空恢复全量。
  - W1.6：从编辑器复制一段多行文本粘进 composer → 整段落进输入框（含换行，光标在末尾）、**不会**被当成交付；粘贴内容夹 `Tab` 不切换焦点；单次超 32K 字符时 transcript 提示截断。`CM_TUI_PASTE=off` 时回到逐字符按键的旧行为。
  - W2.2：左栏选中会话按 `Delete` → 出现确认提示，`y` 删除、其它键取消；回合进行中按 `Delete` 应被拒绝并说明原因；`Shift+Delete` 免确认直删。
  - W3.2（G8）：触发一条长命令的审批 → 收起档只一行（超宽以 `…` 收尾），按 `Ctrl+E` 展开到主体区、命令按键宽折行、提示行显示「第 x-y / N 行」；`↑/↓`、`PgUp/PgDn`、`Home/End` 能滚到首尾（`End` 之后 `↑` 仍能回退）；再按 `Ctrl+E` 收起；展开态下 `Enter`/`Esc` 仍能决策。
- **无浏览器 E2E**：TUI 不属 Playwright 范围（`remote_cli_tui.md`）。

---

## 6. 风险与开放问题

| 风险/问题 | 缓解/决策点 |
|---|---|
| serve 会话「消息分页/删除/改名」端点形态未 pin（W2 前置） | Wave 1 先行；Wave 2 开工前在 Server `docs/` 与 `contract_pin.md` 确认，禁止猜测端点 |
| transcript 行模型改「回合感知」回归面大（W2.3） | 独立 PR + 先抽回合段数据结构；Wave 1 的 5 项不触碰行模型核心 |
| OSC52 支持参差；剪贴板含敏感内容 | 探测 + 回退 + 提示；复制为显式用户动作，语义同 Desktop |
| 队列/草稿与 running gate、Ctrl+C、切会话竞态 | 草稿/队列为纯状态字段，切换/取消时按单一顺序处理并加单测 |
| 会话标题语义（Desktop 本地推导 vs serve 记录）影响 W2.2 重命名 | 确认标题落点后定重命名的持久化去向 |
| 范围膨胀 | 按 Wave 顺序交付，每波独立 PR；非目标清单 §2 作为驳回依据 |
