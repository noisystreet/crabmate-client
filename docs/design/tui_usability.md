# TUI 易用性规划（对齐 Desktop 聊天体验）

> **状态**：Wave 1 已落地 W1.1 全屏 `/resume`、W1.2 忙时排队、W1.3 会话级草稿、W1.4 复制（OSC52 / 外部工具）、W1.6 多行粘贴（bracketed paste，`CM_TUI_PASTE=off` 可关）；**W1.5 会话列标题过滤已落地**（v1 走命令入口 `/conv filter <词>`；**左栏内逐字收窄仍未做**）。W2.1 历史回放已落地（`GET /conversation/messages` 最新页 + `/older` 按 `before_index` 前插更早页，贴顶保持 + PgUp 到顶自续翻）；W2.2 **删除**已落地（左栏 `Delete` 先确认 / `Shift+Delete` 直删，在途回合拒绝），**只缺重命名**（无 serve 端点）；W2.4 **会话导出**已落地（`/conv export [json|markdown|both] [--out <路径>] [--id <id>]`，repl 与全屏共用 `crabmate_client_api::chat_export` 的展示投影，全量靠逐页往回拉）；W3.1 `/` 补全菜单也已落地（`tui_mode/slash_menu.rs`：内建命令 + `GET /skills` 候选）。Wave 2 的 W2.3 回合重试 / 再生 / 编辑已落地（`/conv retry` / `/conv edit`，见下）；Wave 3 其余项仍为方案。逐波按本文落地；每波独立 PR，落地后必须同 PR 更新 [`client_capability_matrix.md`](./client_capability_matrix.md) 对应格（§4）。
> **范围**：`crabmate-tui`（全屏 `tui` 为主，`repl` 相应联动）；候选与取舍参照 Desktop / Web（`frontend/`）聊天体验，**语义对齐，不追求 UI 复刻**。
> **关联**：[`remote_cli_tui.md`](./remote_cli_tui.md)（TUI 母方案，P1–P5）、[`client_capability_matrix.md`](./client_capability_matrix.md)（TUI 行 L53–L81）、[`tui_settings_panel.md`](./tui_settings_panel.md)（设置面板，W3 遗留）、[`chat_ui_todo.md`](./chat_ui_todo.md)（Desktop 聊天 UI 已勾项 = “已做过”的参照，不要当缺口重做）、[`contract_pin.md`](./contract_pin.md)（契约钉点）、Server `docs/`（SSE / user-data / 会话权威）

---

## 1. 背景与现状

### 1.1 Desktop 聊天易用性骨架（参照物，均已勾选落地）

`frontend/`（Desktop/Web 共用 WASM）围绕会话侧栏、消息区、composer 三处铺开的易用性能力：会话 **排序（置顶/星标）+ 标题过滤 + 跨会话全文搜索命中跳转 + 右键/长按菜单（管理/星标/置顶/导出/删除）+ 键盘删除**（`sidebar_nav/`、`session_list_modal.rs`、`session_search.rs`）；消息区 **回合右键菜单（复制/编辑/重试/再生/分支）+ 就地编辑 + 流式排队 + 代码块复制按钮 + stick-to-bottom + thinking/工具行折叠**（`message_turn_menu.rs`、`user_message_edit.rs`、`composer_follow_up.rs`、`md_code_copy.rs`、`chat_ui_todo.md` P1/P2 全部勾完）；composer **每会话草稿 flush/恢复 + `/` 斜杠浮层（含 skills）+ 澄清表单**。

### 1.2 TUI 现状与缺口对照

`crabmate-tui` 已具备：全屏 transcript + 状态栏、左栏会话 use/new/refresh（**并有鼠标：单击会话行即选中并切换**，滚轮刻意不碰列表）、右栏工作区目录树 + 项目池、审批浮层、thinking 折叠、工具行摘要、会话内 `/find`、多行输入（Alt+Enter 换行；粘贴一段多行文本时整段落进输入框，不会被当成多次回车提交）、设置面板（模型/会话两分区，W1+W2 落地）、repl `/resume` 断线续传、**两侧栏列宽参数化 + 一列分隔线**（#407 P0，见下方补记）。与 Desktop 对照，下列**用户高频操作缺口**是本规划的对象：

| # | 缺口（TUI 现状） | Desktop 参照 | 影响 |
|---|---|---|---|
| G1 | ~~全屏 `tui` 无 `/resume`~~ → **已落地**：断线 / 取消未送达时保留断点，控制斜杠表已有 `/resume`（`tui_mode/resume.rs`、`controls.rs`） | in-app stream resume（矩阵 L62） | 远程断线/取消未送达时全屏只能重开一轮，丢上下文 |
| G2 | ~~回合进行中 Enter 仅提示并保留输入~~ → **已落地**：忙时 Enter 进排队槽，回合**正常**结束后自动发出；Esc 清队；取消 / 出错则退回输入框（`tui_mode/drafts.rs`） | 忙时排队下一句 + 排队 chip + 结束后自动发出（`composer_follow_up.rs`） | 用户意图被打断，长回合场景需干等或反复重试 |
| G3 | ~~composer 为全局单缓冲，不保存/不恢复各会话草稿~~ → **已落地**：按 `conversation_id` 换入换出，`/conv new` 起空白（`tui_mode/drafts.rs`） | 每次切会话 flush 当前草稿、回来恢复（`composer.rs`、`session_ops.rs::flush_active_composer_draft`） | 遗留草稿会“串”到下一个会话；切走再回来内容丢失 |
| G4 | ~~无任何复制路径~~ → **已落地**（`tui_mode/clipboard.rs`）：`/copy [assistant\|user]`（别名 `/yank`）与 `Ctrl+Y`。**残余**：粒度固定为「最近一条非空消息」，不能指定中间某条；鼠标捕获占用了终端原生拖选（需 Shift+拖） | 消息 copy + 代码块「复制」按钮（`md_code_copy.rs`、`tui_actions_bar.rs`） | 终端里想取走一段代码/回复只能手工选中或重打 |
| G5 | 会话列表：左栏 use/new/refresh + **删除**（`Delete` 先确认 / `Shift+Delete` 直删，`tui_mode/sidebar_keys.rs`）；`/conv new` 已改为**写**服务端会话索引（`serve/sessions.rs`：`GET` → 复用或插入空白行 → 全量 `PUT`，首轮结束回填 `server_conversation_id`，切走回收空白行，首条消息按 Web 规则起名），左栏与 Web/Desktop 同步可见；仍缺重命名（**无 serve 端点**）、标题过滤、跨会话搜索、导出 | 排序 + 标题过滤 + 全文搜索 + 重命名/删除/导出（`session_rail.rs`、`session_list_modal.rs`） | 会话一多靠翻，无过滤手段 |
| G6 | ~~切换会话后 transcript 从空开始~~ → **已落地**：切换 / 启动回放 `GET /conversation/messages` 的**最新一页**；`/older`（别名 `/history`，也在 `/` 补全菜单里）按 `before_index` 前插更早一页（80 条/页），并**同时平移**工具行映射 / 搜索命中 / 搜索锚点（`tui_mode/history.rs`、`tui_mode/state_history.rs`、`tui_mode/body_cache.rs`）。**残余**：视口未补偿（前插后画面整体下移一页高度），滚动锚定留待下一个 PR | 会话水合 + 顶部「加载更早」prepend（`session_hydrate.rs`、`tui_stream_view.rs`） | 续聊时看不到前文，须先切到别的壳回顾 |
| G7 | 无 retry / regen / branch / 编辑用户消息（`controls.rs` 无对应命令，`tui_mode` 里无 `regen`/`branch`） | 消息回合菜单（`message_row_actions.rs`、`user_message_edit.rs`） | 模型答错只能整轮重发或手动复制改写 |
| G8 | ~~审批浮层把命令截成一行且无法展开~~ → **已落地**：`Ctrl+E` 在 78×9 收起档与铺满主体区的展开档之间切换，展开档按显示宽折行（`text::wrap_display`）并支持 ↑/↓、PgUp/PgDn、Home/End 滚动，提示行显示「第 x-y / N 行」（`overlay.rs`） | 底部审批条可展开完整命令 + 失败保留重试（`approval_modal.rs`） | 长命令看不到全貌，易误判 |
| G9 | 澄清问卷（`clarification_questionnaire`）在 TUI 无交互 UI：`serve/chat_classify.rs` 只有 `clarify_trace` 钩子，没有表单 | 问卷表单（Web `frontend`） | 模型需要澄清时 TUI 这侧拿不到结构化提问 |
| G10 | 窄终端（< 120 列）下**当前工作区不可见**：左右栏按 `sidebar_visible` 隐藏（`layout.rs::panes`），而工作区名只画在右栏（`ws_sidebar.rs::workspace_title`），顶栏已删除（`tui_visual_polish.md` §11.1）；状态行（`status.rs`）与右栏状态栈都不带 workspace 字段 | 顶栏工作区名 + 面包屑 | 窄屏 / 分屏时不知道命令跑在哪个项目，得先 `/workspace` 查一次。**已落地（#425）**：窄屏底栏加 `ws <basename>` 段（未设置时 `ws 未设置`），宽屏不加（右栏标题已交代）；80 列起与 `perm` 共存，60 列是物理下限 |
| ~~G11~~ | ~~上下文用量水位压缩后不回落~~ **已修**：`serve/chat_classify.rs` 接上 `notice_timeline.on_timeline_log`，`kind=context_trim` 的 `detail.after_tokens` 经共享层 `context_trim_after_used_tokens` 解析后由 `UiState::reset_used` 落到 `ContextUsedWatermark::reset`（与 Web 端同一份口径） | ~~把 `context_trim` 的 `after_tokens` 接到 `ContextUsedWatermark::reset`~~ 已完成 | ~~压缩后 `ctx` 停在旧峰值~~ 已消除；回归用例：`context_trim_custom_event_yields_reset`（分类）+ `context_trim_reads_stringified_detail` / `context_trim_reset_needs_all_three_conditions`（口径） |

另有与 Desktop 同步暂缓/不做项，见 §2 非目标（勿误当缺口）。

> **2026-10-25 代码核对**：G1–G10 已逐条对着代码验过（`resume.rs` / `drafts.rs` / `clipboard.rs` / `sidebar_keys.rs` / `history.rs` / `overlay.rs` / `ws_sidebar.rs` / `status.rs`），表内状态即当时事实。G10 为本次核对新发现的缺口（顶栏删除后的遗留债）；G5 的「导出」与 G7 的 retry/regen/branch 仍依赖 serve 端点，不属可独立落地项。

> **2026-10-31 补记**：新增缺口 G12「一轮的执行过程把 transcript 撑长」已落地（issue #369）——一轮的 `Thinking` / 工具行在回合结束时折成一条 `Process` 摘要行，`/older` 回放与滚动回看同样折叠。折叠单元是「**连续过程行段**」而不是回合区间（回放没有回合边界，区间会被前插错位）；`Assistant` / `System` 行永不折叠。**展开是可逆的**：摘要行始终保留，`/fold [n|all|close]` 切换它的开合（前缀 `▸ ` 收起 / `▾ ` 已展开；无参=最近一条，`n` 从底部数，`all` 全展开，`close` 全折回），`sticky` 随开合双向设置，所以折回的段下一轮仍会被自动折叠。**也能用光标挑**（`Ctrl+G`，任何焦点都能进）：摘要之间 `↑/↓`、`Enter` 开合、`Esc` 退出，左键点摘要同义（并把焦点带回输入框，否则接着按 `↑↓` 走的是侧栏）；空格与 `Alt+Enter` **不**归导航（前者是可打印字符、后者是换行）。进导航 = 接管方向键与视口：`/find` 锚点与 `/` 补全菜单都必须让位，否则就是「↑↓ 在动、画面钉在命中行」；`/fold <n>` 在导航开着时把光标带到它刚切过的那条。`/older` 前插与回合结束插入摘要行时，光标按插入量重映射（与视口锚点同一份 `remap_after_fold`），否则「按了 Enter 之后 ↑↓ 好像卡住」就是它指到了别的段。落点见 `crates/crabmate-tui/src/tui_mode/state_fold.rs` 与 `CHANGELOG.md` 的 `[Unreleased]` 条目。

> **2026-11-02 补记**：新增缺口 G13「左右两栏列宽写死在编译期」——`pane_width.rs` 的 `SIDEBAR_WIDTH = 26` / `WORKSPACE_PANE_WIDTH = 34`：窄终端下两栏挤掉正文，宽终端下两栏又白占几十列，而两栏**没有边框**（边界只是聊天列的留白，用户看不出边界在哪）。Desktop 的侧栏可拖拽（`frontend/src/app_prefs.rs`：`side_width` 默认 280px、区间 200..560）。**P0 已落地**（issue #407）：`PaneWidths { left, right }` + `MIN/MAX` 边界 + **幂等**的 `clamped(terminal_width)`（聊天列 `MIN_CHAT_COLS = 40` 保底；超预算时先压右栏、再压左栏，因为右栏挤窄只是把 `ctx` 行截断，左栏挤窄就读不出会话标题了）；`layout::panes_with_lanes` 改收 `PaneLayout { visible, widths }` 并在函数内再夹一次（幂等），于是它仍是**全部矩形**（两栏 / lane 区 / 状态栈 / 聊天列）的唯一来源 —— 渲染与鼠标命中不可能出现「画的是夹取后的、点的是夹取前的」。`chat_body_width` 与 `LaneApp::status_stack_rows` 同口径读**右栏内宽**：状态栈的行数取决于内宽，两处不同源的症状是「点状态栈点到别的段」，而不是画面上的错位。两栏各画一列 `Palette::border` 分隔线，并贯穿左栏下半的 lane 区与右栏底部的状态栈（上下两段不同列会变成两条半截线）。默认值仍是 26 / 34，所以 P0 的可见变化**只有那条竖线**。**P0.5 已落地**（issue #407）：按住任一栏的分隔线拖动即可改宽。要点：`worker::is_actionable_mouse` 放行左键 `Drag` / `Up`（悬停 `Moved` 仍然丢弃 —— 1002 模式下只有按住键才产生 `Drag`）、`shell::lane_at_click` 补 `MouseEventKind` 判定（否则拖动经过左栏下半会当场切 lane）、分隔线命中**抢在** `click_sessions` / `click_workspace` 之前（否则一次拖宽会顺带把会话切走）、同一列重复的拖拽事件不写状态（事件循环每条消息都整帧重绘，宽度没变时 transcript 缓存仍命中）。换算（命中哪条线、指针第几列 → 多宽）是 `pane_drag.rs` 的纯函数，事件接线与入口门禁在 `mouse_drag.rs`（`mouse.rs` 也已贴着 920 行门禁）；宽度**按指针列重算**而非累加增量 —— 累加会让「拖到边界被夹住、再拖回来」时指针与分隔线错位。拖拽态放 `LaneApp` 而不是 `UiState`：它是一次手势的中间态，进 `st` 会让「同尺寸同内容」的两帧不再相等；审批 / 设置浮层接管鼠标时当场清掉它。**P2 已落地**（issue #407）：列宽落**本机** `$XDG_STATE_HOME/crabmate/tui-panes.json`（无 XDG 时回退 `~/.local/state/crabmate/`；`CM_TUI_PANES_FILE` 可覆盖整个路径），文件里只有 `{"left":…,"right":…}` 两个字符列数 —— **不**复用 Desktop 的 `user-data/prefs.side_width`（像素 vs 字符列，同一个键会让两端互相覆盖），也**不**往 `user-data` 加新键（那要先在 Server 仓定义契约，而这件事没有跨端语义）。落在 `state` 而不是 `config`：宽度是「上次看成的样子」，文件丢了 / 手改坏了只该回到默认版面，那里不该出现一份要人维护的配置。写入时机是**拖拽松手**（`mouse_drag` 的 `Up`），且只在宽度真的变了时才写 —— 拖动途中一个字节都不写（终端每列都发事件，否则磁盘就成了帧缓冲），没拖过的 `Up`（点会话、点树都会有）同样不碰文件。写盘走临时文件 + `rename`：半截 JSON 比「没有文件」更糟，下次启动读到坏内容会静默回落默认，而用户以为宽度存下来了。读侧**不报错**（没有 HOME、文件不存在、字段缺失、JSON 坏了都回落默认；缺的那一栏回落它**自己**的默认值而不是 0 —— 0 会被夹到下限，看起来像宽度自己变窄了），写侧失败在 transcript 里提示**一次**（全屏 TUI 的 stderr 看不见，而完全不说是更坏的选择：用户会以为存下来了，下次启动却回到默认）。读盘只按**各自上下限**夹取（`clamped_to_ranges`，此刻还没测终端宽度），按终端让位留到绘制时的 `clamped(terminal_width)`；落盘目的地（`PaneStore`）在装配 lane 时解析一次、随 lane 持有，事件路径上不读环境变量 —— 单测因此可以把目的地指到临时文件，不会把宽度写进跑测试那个人的 `~/.local/state`。**磁盘上只有一份宽度，运行期每条 lane 各持一份**：拖哪条 lane 就记那条的宽度，重启后所有 lane 都用最近一次落盘的那份（单 lane 时两者没有区别，多 lane 时这是刻意选择 —— 宽度是「一份偏好」，按 serve 名分键会让 `/lane attach` 换个名字就丢）。写入时**只覆盖被拖的那一栏**：另一栏可能正被终端预算让位压着（`clamped(terminal_width)`），让位是显示层结论，把它写回文件就是「在小窗口里拖了一下左栏，回到大终端右栏永久变窄」；文件里留下的是「各栏各自被拖到多宽」。**未做**（按 #407 的分期，不是遗漏）：P1 `/panes left <n> | right <n> | reset`（`CM_TUI_MOUSE=off` 时的等价通道）与双击分隔线复位。另：`render.rs` / `state.rs` 都贴着 920 行/文件的 `fn-nloc-ratchet` 门禁，P0 的渲染侧改动因此拆到新模块 `pane_render.rs`，`pane_layout()` 取值器放在 `pane_width.rs` 的 `impl UiState` 里。

> **补记（视图切换开关）**：`/editor` 的视图开了以后，屏幕上只有视图自己的标题行能回答「我在文件里」——transcript 被整片盖住，而切回去只有 `/chat` 与「输入框为空时 `Esc`」两条要记住的路。这一轮补上**状态栏 `view` 段 + `Ctrl+T`**：段值 `chat` / `file`，与 `think` / `perm` 同为可点开关（宽屏点状态栈那一行、窄屏点底栏那几列），命中链与渲染同源（`status::segment_cols` / `status_stack::segment_row_index`，不各自重算排版）。`Ctrl+T` 挂在**与焦点无关**的热键表（`hotkeys::of`）而不是 `on_input_key`：后者开头就把滚动键交给编辑器视图，放在那里等于「视图开着时按不出来」，而那正是最需要它的时刻。两个方向都复用 `/editor` 与 `/chat` 的入口（`toggle_view` → `editor_reopen_current` / `editor_close`），所以「切回来重读当前文件」「没打开过文件就开占位视图」「焦点交回输入框」三件事只有一份实现。段**恒在**（`chat` 也算）：段数因此不随开关变化，纵版行数与鼠标命中才能共用同一份 `visible_segments`。**没有**为它抬 `MAX_STACK_ROWS`——10 行是 24 行终端的上限、`think` 那一轮已经把余量用掉了；全填充 10 段时按优先级先丢 `lanes`，安全相关的 `perm` 保住。落点：`status.rs`（`VIEW_LABEL` / `priority::VIEW_MODE` / `view_text`）、`status_stack.rs`（裁剪路径首次生效，含溢出用例）、`mouse.rs`（`hit_status_segment` 统一三段命中，免得每加一个开关就多付一份圈复杂度）、`editor_view.rs::toggle_view`、`hotkeys.rs`、`help.rs`。

> **2026-11-03 补记**：`perm` 段与 `/perm <level>` 在**回合进行中一律拒绝换档**，与 `/workspace <路径>` / 项目池切换同一条守卫（`st.running`，见 `workspace_cmd.rs`）。理由与本轮 `/chat` 的快照一致：档位在 `start_turn` 时已随请求体发出（`turn_send.rs`），飞行中的工具按哪一档判定不该在一轮内前后不一致。**不做**「先改意图、等回合结束再落盘」——那正是旧行为的症状：意图立刻变、生效投影要等落盘回包，而 worker 是单线程串行消费（回合 `block_on` 占着它），于是状态行整个回合都挂着 `unrestricted→approved`，读起来像「服务端把我钳制了」，实际只是请求还排在队里没出站。拒绝时给一句原因（「a turn is running — switch after it finishes」）；点 `perm` 段同样只给这句、不开菜单——菜单是纯写入入口，选不动的菜单比直接说明更困惑。**Desktop 同源**：底栏 chip 与设置页选择器读 `ChatShellLeptosContext::perm_turn_in_flight`（与「停止」门闩同一谓词 `turn_lifecycle_stream_turn_busy`），忙时同样只写就地提示、不动意图信号。**未覆盖**：`WritePermission` / `RefreshStatus` 这类轻任务仍会被回合挡住，要让它们在回合中真正并发得改 worker 的调度模型（另议，见任务单）。

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

> **实现**：草稿按 `conversation_id` 存在表里，切会话 / `/conv new` 时换入换出；全空白视为无草稿。退出时仍由 `exit_flush` 把当前会话那一份写回服务端索引。**跨进程保留已补**（issue #430）：切走的会话也落本机 `$XDG_STATE_HOME/crabmate/tui-drafts.json`（`CM_TUI_DRAFTS_FILE` 可覆盖），装配 lane 时读回。两个设计点：**分区按 `api_base`**——两个 serve 的同名 `conversation_id` 不会互相串（扁平存会静默串）；**不做每键写盘**——只在切会话 / 退出写，所以异常退出最多丢「最后一次切会话之后敲的字」。本机文件与服务端索引那份草稿互不覆盖：本机代表「这台机器上打到一半的字」，不把服务端值回灌进输入框。装配 lane 时**未绑定槽**（还没发过言的那段输入）直接回到输入框 —— 「打到一半就退出」是最常见形态，等一次 `/conv new` 才出现等于没恢复。

- **参照**：Desktop `flush_active_composer_draft` / 切会话恢复。
- **方案**：composer 文本按当前 `conversation_id` 缓存：切会话时保存旧缓冲并载入目标会话草稿，`/conv new` 新建时清空草稿，发送成功后清空该会话草稿。杜绝草稿“串会话”（现状见 §1.2 G3）。
- **落点**：`state.rs`（`HashMap<conversation_id, String>` 草稿表）、`mod.rs`（`switch_to_conv`/`new_session`/发送成功回调）。
- **风险**：草稿仅本进程内存（repl 的 `/model` 等偏好同样进程级），不落盘——文档注明即可。

#### W1.4 复制（OSC52 + 外部剪贴板回退）（G4）— 已落地（`tui_mode/clipboard.rs`）

> **实现**：`/copy [assistant|user]`（别名 `/yank`）与 **Ctrl+Y** —— 后者在顶层热键表里，输入框 / 会话栏 / 工作区树三种焦点下都可用。复制目标是**最近一条非空消息**的**原始 Markdown 正文**（不是渲染后的文本）。投递按环境自动选路：SSH 会话（`SSH_CONNECTION` / `SSH_TTY` / `SSH_CLIENT`）走 **OSC52**（内容由本地终端执行；客户端跑在远端时只有这条路能把正文送回用户手边）；本地会话优先 `wl-copy` → `xclip` → `xsel`（能拿到退出码），都没有时回落 OSC52；tmux 下才加 passthrough 包裹（`ESC Ptmux;` + 内层 ESC 双写，服务端需 `allow-passthrough on`）；**screen 不转发 OSC52**，在那里只走外部工具、没工具就直接报错。OSC52 不能分块也没有回执，base64 超过 64 KiB 时结果行会标注「可能不完整」；外部工具存在但跑不通（无 `DISPLAY` / 无 wl 服务）时回落 OSC52 一次。`CM_TUI_CLIPBOARD=off|osc52|tool` 可强制路径，认不出的值按未设置处理。结果写一行系统提示（`已复制助手回复（12 行，经 终端剪贴板）` / `没有可复制的助手回复` / `复制失败：…`）。**与原方案的偏差**：按键从 `c` 改为 **Ctrl+Y**（输入框焦点下 `c` 就是打字，不能占用）；目标是「整条消息」，**代码围栏块**走 `/copy code [n]`（倒数第 n 块，缺省 1 = 最后一块；回执写明语言标签与块序，见 issue #428 —— 扫描在**规范化后的原始 markdown** 上做，所以拿到的是源码本身，不带渲染态的竖条 / padding / `…` 截断，粘连围栏与流式未闭合围栏也照抄到此刻为止），也**不复制** thinking 行。

- **参照**：Desktop 消息 copy / 代码块复制按钮。
- **方案**：新增按键：当前「锚定/最近」对象复制——定义：`c` 复制**当前光标所在/最近一条助手消息**正文（纯文本，含已折叠 thinking 之外的部分）、代码围栏块聚焦时复制整块；复制协议优先 **OSC52**（kitty/wezterm/screen/tmux `set-clipboard` 支持），不支持且环境有 `wl-copy`/`xsel`/`xclip` 时回退调用，否则系统行提示「终端不支持剪贴板」。Esc 不干扰。
- **落点**：新增 `tui_mode/clipboard.rs`（纯序列化 + 探测），`render.rs`/`mod.rs` 接线；`md.rs` 暴露围栏边界。
- **风险**：OSC52 需终端显式允许（tmux `set-clipboard on`）；SSH 到远端时剪贴板属本地终端，语义正确无需处理。密钥/敏感内容复制是用户主动行为，与 Desktop 同语义。

#### W1.5 会话列标题过滤（G5 的一部分）— 已落地（v1）

- **参照**：Desktop 侧栏搜索面板标题过滤（`search_panel.rs`）。
- **v1 已落地（命令入口）**：`/conv filter <词>`（`/conv list <词>` / `ls` / `show` 同义）按标题做大小写不敏感子串过滤，空标题按 `(untitled)` 参与匹配；空参或 `off`/`none`/`clear` 清除。左栏标题写成 `会话 [可见/总数] · filter: 词`，无匹配给 `（无匹配）` 占位行；`↑/↓` 只在可见行间走、`Enter` 使用，鼠标点击同样过可见映射。过滤生效时侧栏提示换成「`/conv filter off` 清过滤」。切会话（`/conv use`）与新建会话会清掉过滤 —— 否则会出现「切过去了但左栏看不见」。
- **为什么不做「左栏内逐字收窄」**：左栏的裸字母键已占用（`n` 新建 / `r` 刷新 / `w` 跳右栏），字母即过滤会让它们随上下文变义且不可发现。状态模型（`session_filter` + 可见映射）与它完全兼容，将来只是多一层按键分流（见 issue #423 的评论）。
- **落点**：`tui_mode/state_session_filter.rs`（过滤词 / 可见映射 / 选中吸附 / `move_selection`）、`controls.rs`（`/conv filter` 解析）、`conv_cmd.rs`（落地 + 回执）、`render.rs`（标题与占位行）、`ws_sidebar.rs`（光标口径 + 提示）、`mouse.rs`（点击过可见映射）。
- **风险**：小。过滤只影响「显示 / 移动 / 鼠标命中」，`selected` 仍是全量下标，删除与切换链路未变；窄屏左栏不可见时过滤无副作用。

#### W1.6 多行粘贴（bracketed paste）— 已落地（`tui_mode/paste.rs`、`tui_mode/state_paste.rs`）

> **实现**：启动时下发 `EnableBracketedPaste`（`ScreenGuard` 退出时对称关闭；`CM_TUI_PASTE=off` 可关），键盘线程把 `Event::Paste` 转成 `UiEvent::Paste`，`on_paste` 一次性写进输入框（**不提交**）。归一化在 `UiState::insert_paste`：`\r\n` / 裸 `\r` 统一成 `\n`（多行判定只认 `\n`）、`\t` 展开四空格、其余 C0 控制符丢弃（裸 `\x1b` 会被渲染层当成未闭合转义序列）、单次上限 32K 字符（超出截断并在 transcript 提示）。落地前的问题：终端把粘贴当逐字符按键，换行变成 `KeyCode::Enter` 被当成提交，多行内容只发出第一行、剩下的落进新草稿；夹带的 `Tab` 还会被当成焦点切换。**未做**：图片/文件粘贴、多行粘贴后自动折叠成「已粘贴 N 行」占位。

- **参照**：Desktop composer 的粘贴（浏览器原生 `paste` 事件，整段进 textarea）。
- **落点**：`tui_mode/mod.rs`（开关 + `ScreenGuard`）、`tui_mode/event_loop.rs`（`paste_capture_enabled` / `enable_captures`）、`tui_mode/worker.rs`（事件转发）、`tui_mode/paste.rs`、`tui_mode/state_paste.rs`。
- **风险**：小。关掉括号粘贴即回到改造前行为（逐字符按键）。

### Wave 2 · 需确认 serve 会话/消息契约

> 前置：核对 serve 是否提供「会话消息分页读取」「会话删除/改名」端点（TUI `/conv list` 已证明会话列表存在；Desktop 水合数据源在 serve 侧）。未 pin 前不落地，禁止用猜测端点写代码。

#### W2.1 会话历史回放 + 「加载更早」（G6）— 已落地（最新页 + `/older` 前插）

> **实现**：切换 / 启动回放照旧拉 `GET /conversation/messages` 的**最新一页**，回包里的 `window_start_index` / `has_older` 落进 `UiState::history_page`；`/older`（别名 `/history`）把 `window_start` 当 `before_index` 再请求一页，回包走 `prepend_replayed` **前插**（不清屏、不动会话绑定）。**前插要同时处理三处行号状态**：`tool_pending`（工具行映射）与 `search_cursor`（搜索锚点）跟着整体平移，`search_hits` 改为**整表重扫**（只平移会漏掉更早页里的命中，`/find` 于是计数偏小、跳不到它们；翻页低频，O(行数) 付得起），游标本身换成回包的 `window_start_index`。漏掉任何一处都会让后续事件落到错误的行上（工具结果改写历史消息、`/find` 跳到隔壁行）。进度点 `loading` 兼作「这次回包是不是我这次翻页」的判据；切走会话或开跑新回合后到达的回包直接丢弃并复位标记；**切换会话的回放还在途时 `/older` 一律拒绝** —— 那时 `conversation_id` 已改绑新会话而游标仍属于上一个会话，放行会把别的会话的一页前插进来。`window_start_index == 0` 视作「已到最早一条」，不认服务端同时给出的 `has_older`，避免反复请求同一页。顶部提示是 `body_cache` 生成的 `log_index: None` 物理行（「↑ 还有更早的消息：/older 加载」/「↑ 正在加载更早…」），因此缓存与全量重建逐行一致（`assert_same_as_full_rebuild` 覆盖），搜索锚点与 spinner 定位天然跳过它。**视口**：`view_offset` 是「距底部行数」，头部插入不改变底部对齐，所以滚动在中间/底部时前插**天然不移动视线**（白拿的锚定）；唯一要补的是「已在顶」——那时贴顶等于「看着最早那条」，`render_body` 每帧把 `body_at_top` 回写给状态层（只有渲染层知道 `max_back`），`prepend_replayed` 据此把 offset 重新打到 `SCROLL_TOP` 哨兵，于是新页出现后仍贴顶、继续往上读。**PgUp 到顶自动续翻**：已在顶再按 PgUp 就是「我要看更早的」，走 `load_older_on_top`（与 `/older` 同一个门控 `gate_older`，但**拒绝时静默**——在顶连按是高频动作，不该每按一次就往 transcript 写一行提示）。
>
> **依赖修正**：原方案写「依赖 serve 消息分页端点」——核对后该端点与 `before_index` 参数**早已就绪**（`serve/conversation.rs` 一直传 `CONVERSATION_MESSAGES_PAGE_LIMIT`，`ConversationMessagesResponse` 已带 `window_start_index`/`has_older`），TUI 只是永远传 `None`，所以本项属纯客户端改动，无新契约。

- **参照**：Desktop `session_hydrate.rs` 水合 + 顶部「加载更早」prepend 且滚动锚定不跳（`scroll_shell.rs::compensate_after_prepend`）。
- **落点**：`tui_mode/history.rs`（门控 / 前插回包）、`tui_mode/state_history.rs`（游标 + `prepend_replayed`，压 `state.rs` 行数）、`tui_mode/body_cache.rs`（顶部提示行）、`tui_mode/controls.rs` + `slash_menu.rs`（`/older`）、`tui_mode/worker.rs`（回包带 `before_index`）。

#### W2.2 会话重命名/删除（G5 的一部分）— 删除已落地，重命名待端点

- **参照**：Desktop 行内重命名 + 删除确认（`session_modal_row.rs`、`session_ops.rs`）。
- **已落地**：左栏 `Delete` 进入确认态（`y` 确认、其它键取消）、`Shift+Delete` 免确认直删，在途回合拒绝删除——对齐 Desktop 的两级快捷键（`tui_mode/sidebar_keys.rs`，纯逻辑有单测）。删除走契约钉的 `DELETE /conversation/{conversation_id}`（幂等 204）；repl 另有 `/conv delete [<id>|current]`（`slash.rs`），只删服务端记录、不动 Web 侧栏索引。
- **未做**：行内重命名——**仍缺 serve 端点**，客户端不能自造 `PUT`；左栏也没有过滤/搜索（见 G5 / W1.5）。
- **依赖**：重命名需 Server 先提供端点并在 `contract_pin.md` 钉住；不得猜路径。

#### W2.3 回合 重试 / 再生 / 编辑用户消息（G7）— 已落地（`/conv retry` / `/conv edit`）

- **参照**：Desktop `message_row_actions.rs`（regen/branch 优先 `POST /chat/branch`，NotFound/Conflict 走本地截断重试）、`user_message_edit.rs`。
- **已落地**：`/conv retry` 重跑上一轮 —— 服务端 `POST /chat/branch` 先把会话截到该轮用户消息之前，本地 `truncate_to_last_user_turn` 截到同一位置，再重发同一句；`/conv edit` 只把该句放回输入框，**提交时**才 branch，所以「打开来改又不改」不会动服务端历史。序号（`before_user_ordinal`）在客户端按分页历史现数并**跳过首轮工作区上下文注入条**；有归档回合时拒绝而不是猜（猜错会把会话截到别处）。`NotFound` 解开 `conversation_id` 绑定，`Conflict` 只提示先刷新、不动本地 transcript；流式中 / 有排队消息 / 上一发还在路上都拒绝并说明原因。
- **落点**：`crabmate-client-api::chat_branch`（分类 + 序号）、`serve/branch.rs`（HTTP）、`state_truncate.rs`（截断原语）、`conv_cmd.rs`（命令与回包）、`worker_jobs::job_conv_branch`。
- **不做**：回合段数据结构（本 issue 只到「最近一个回合」，回扫 + 截断已够；任意历史回合的重跑是另一件事）、分支树浏览、编辑 assistant 消息。

#### W2.4 会话导出（Markdown / JSON）— **已落地**

- **参照**：Desktop `session_export.rs`（`cm_chat_export projection=display`；Markdown 与 CLI 同规则）。
- **命令**：`/conv export [json|markdown|both] [--out <路径>] [--id <id>]`（repl 与全屏 `tui` 同一套；格式词与 Web `/export` 同名同义，缺省 `json`）。
- **口径**：信封与 Markdown 骨架直接来自契约 crate（`DisplayChatSessionFile` / `markdown_from_role_bodies`），逐条过滤沿用 `conversation_messages::replay_rows`（跳过 `system` 卡片与首轮工作区注入、工具名从调用侧 `tool_calls` 回填、工具正文折成一行 ≤200 字摘要）；**思考并进紧随其后的助手正文**（展示投影没有 thinking role）。
- **全量**：端点 80 条/页，故导出是「逐页往回拉到 `window_start_index == 0`」，上限 `EXPORT_MAX_PAGES = 200`（≈16k 条），超限**报错**而不截断。客户端翻页 vs 服务端全量端点这一项按**客户端翻页**结案：不改 Server 契约。
- **出口**：repl 不给 `--out` 写 stdout；全屏 `tui` 不给 `--out` 落到当前目录 `chat_export_<UTC 时间戳>.md/.json`（alt-screen 里打几万字既没人读也会刷掉对话）。`--out` 给目录 → 目录 + 默认文件名；给文件名且 `both` → 只换后缀。
- **落点**：`crabmate-client-api/src/chat_export.rs`（纯逻辑）、`crates/crabmate-tui/src/conv_export.rs`（解析 / 渲染 / 落盘）、`tui_mode/export.rs`（门控 + 报告）、`tui_mode/conv_cmd.rs`（`/conv` 子命令落地）、`serve::fetch_all_pages`、`slash.rs`（repl）；`help.rs` 更新文案。
- **门控**：回合在跑拒绝（会读到半截会话）、会话未绑定且无 `--id` 拒绝、已有一次导出在途拒绝；回包按 `export_pending` 比对，期间切走则**不写盘**。导出不写会话索引、不动 transcript、不改绑定。
- **依赖**：会话历史读取端点（与 W2.1 同源）。
- **未做**：不做 PDF / HTML、不导出图片附件（TUI 的 Markdown 图片本来就降级为 `[图] alt`）、不做导出后上传；不接 `context_inject` 旁注开关（TUI 没有该设置，服务端那两条 `system` 卡片天然被跳过 —— 与 Desktop 的已知差异）。

### Wave 3 · 锦上添花 / 开放决策

- **W3.1 斜杠补全列表** — **已落地**（`tui_mode/slash_menu.rs`）：composer 输入 `/` 前缀弹出候选（内建命令 `controls.rs` 表 + `GET /skills` 的 skill），↑↓ 选择 · Tab/Enter 采纳 · Esc 关闭；候选为空时底部说明 skill 为何缺失。
- **W3.2 审批浮层看完整命令** — **已落地**（G8）：`Ctrl+E` 展开/收起，展开档按宽度折行 + 可滚动，提示行给出位置；收起档的单行预览也把语义换行压成空格（不再因为 `Line` 里夹 `\n` 把提示行顶掉）。
- **W3.3 阅读增强** — **链接部分已落地（应用内，不是 OSC 8）**：正文里的 `[文字](url)` 与裸 `http(s)://…` 都能打开（**`Ctrl+单击`** 命中链接文字即拉起系统浏览器（`xdg-open` → `gio open` → `sensible-browser` → `open`），普通单击仍是「回到输入框」；裸地址按 `[url](url)` 登记 —— 显示文本就是地址本身，屏幕上逐字不变，尾随标点会剥掉，行内代码与代码块里的地址保持字面量）。**OSC 8 明确不做**：本程序开着鼠标上报（`shell.rs` 的 1000/1002/1006），点击会被送进本进程、终端自己的链接处理器收不到，OSC 8 在这套捕获模式下收益为零；应用内命中表不依赖终端能力，长 URL 折行后逐段都能点。落点：`tui_mode/links.rs`（URL 校验 / 打开器 / `click_link`）、`body_click.rs`（链接 → 折叠 → 「回输入框」的分派）、`body_row.rs`（`BodyRow` + 热区）、`md_links.rs`（`PartChars` + 折行换算列）；`render_body` 每帧回写 `UiState::body_links`（与 `body_rows_log` 逐行对齐，鼠标侧只做减法）。边界：只放行 `http(s)`（拒 `javascript:` / `data:` / `file:` / 控制字符 / >2048 字节）；SSH 会话降级为「已复制链接 + 说明」（复用 `clipboard::in_ssh_session` 与 OSC52），无打开器或拉起失败同样回落复制；表格单元格内的链接**也可点**（issue #428：`md_table` 的 `render_row` 保留单元格 `PartChars` 的链接区间 —— 「按列截断没有稳定列」这个理由不成立，屏幕列从来不是这一层算的；`pad_cell` 的左填充经 `absorb` 右移区间，`truncate_cell` 的 `clip_links` 把越过 `…` 的段裁掉，列换算仍由 `wrap_styled_rows` 统一做，热区因此与看到的字对齐）；**每种结果都写一行回执**，因为静默失败就是「点了没反应」。**工具详情整段展开已落地**（issue #428 第 3 条）：`/detail [n]` 把某次工具调用的输出全文铺成只读浮层（`↑↓`/`PgUp`/`PgDn`/`Home`/`End` 滚动、`Esc` / `q` 关闭）。前提是 `ToolResultInfo.output` 本来就带全量（上界是 Server 的工具输出长度上限，不是 SSE 流式截断）—— 卡点在 `chat_classify::tool_result_action` 把它压成一行；现在 `output` 随结果帧一起下发，由 `state_tools::tool_end` 写进「行号 → 全文」侧表（单条 64K 字符、总量 4M 字符，超出按行号从旧到新淘汰），`/older` 前插时与工具行映射一起平移。刻意**不复用** `/fold` 的折叠状态机：折叠管「哪些行可见」，详情只读一行现成文本，关掉即无痕。边界：只有**本进程跑过**的工具带结构化输出，回放出来的历史 tool 行没有 —— 那时命令写一行说明而不是开空壳；thinking 详情不在此列（同一份逻辑没有对应字段，另议）。
- **W3.4 设置面板补齐 W3 遗留**：MCP 全局开关与 `tool_timeout_secs`、Session SQLite 开关——实现归属 [`tui_settings_panel.md`](./tui_settings_panel.md) §7，本规划仅列入口不展开。
- **W3.7 工作区树按文件类型着色与标记** — **已落地**（#508；后继细分了色板）：右栏每个条目按类型取色（目录 / 代码 / 配置 / 文档 / 图片 / 其它）并在名字前带 2 列标记（`{` `=` `"` `*` `·`）；目录仍用 `▸/▾` 折叠箭头——一列不承载两个语义。分类器是纯函数（`ws_file_kind.rs`），**按名优先于扩展名**（`CMakeLists.txt` 是构建配置，不是 `.txt` 文档；`.bashrc` 的点开头不算扩展名）；色板随后细分成十五档语言 / 资产槽（Rust / Python / JS / TS / Go / JVM / C 系 / 脚本 / Web / 配置 / 锁文件 / 数据 / 文档 / 图片 / 字体音视频），每档一深一浅两套值（色相是文件身份，不该随主题漂），**均为终端专用色阶**（不绑 CSS token，所以 Desktop 主题零改动）；标记刻意用单列宽 ASCII 而不是 Nerd Font —— 字体装没装探测不到，且部分终端把图标算成 2 列会直接撞坏右栏对齐（点击按行命中，列错位看不出来但点得到）；`CM_TUI_FILE_ICONS=off` 退回纯上色，`NO_COLOR` 下标记仍在（它不依赖配色）。
- **W3.8 编辑器源码视图语法高亮** — **已落地**（#510，两步）：`editor_hl` 是纯词法器（注释 / 字符串 / 数字 / 关键字 / 类型 / 函数 + 跨行状态），覆盖 Rust / C++ / Python / JSON / TOML / Markdown 六种，其余扩展名回正文色。两种语言走行级特例（TOML 的键名与表头、Markdown 的标题/围栏/列表标记）。P2 把 `Palette` 的六个 `syn_*` 槽接进 `source_body`：**终端专用色阶**（不绑 token，Desktop 主题零改动，与 W3.7 的 `file_*` 同一取舍），`Palette::syntax_style` 是唯一出口，因此 `editor_hl` 顶部那条模块级 `allow(dead_code)` 已删。两个实现要点留在这里，因为都属于「看着能省一步、其实会错」：**逐帧重扫全文不行** —— 可见窗口算不出起始态（块注释 / raw string / 围栏可能从任意前面的行跨过来），而 2 万行 × 每行一次 `Vec<char>` 分配会直接掉帧，所以按 `text_rev` 缓存「语言 + 每行起始态」，与 `md_cache` 同一口径；**裁切与分词必须同一次遍历** —— `HlSpan` 的区间单位是 char，行号栏却按显示列算，先 `clip()` 再切会因尾部追加的 `…` 整体错位，先切段再各自裁会算出互相矛盾的边界。正文预算按 `gutter + 3` 扣（行号 + 分隔空格 + `│ `），少算一格就会让「恰好超一个字」的行向右溢出、被 `Paragraph` 裁掉最右一列；宽度也统一按逐字符累加（`UnicodeWidthStr::width` 把 ZWJ 连字 emoji 当一个簇，两者混用会让「判定不裁」与「实际超宽」同时成立）。叠加顺序固定为「语法色 → `md::highlight_override`」：查找命中压过语法色（正在找的东西优先于装饰），`NO_COLOR` 下六个槽一并 `Reset`。`.jsonc` 仍映射到 `Language::Json`（JSON spec 无注释标记，故其 `//` 与 `/* */` 不上色）—— 为一个注释多开一档 `Language` 不划算，口径写在这里而不是留给猜测。刻意不引 syntect / tree-sitter：只读浏览视图，零依赖让颜色留在 `Palette` 里，而不是再映射一层第三方主题（对比见 issue #510 决策 1）。

---

## 4. 需随落地同步的能力矩阵格

| 落地项 | 矩阵位置（`client_capability_matrix.md`） | 变更 |
|---|---|---|
| W1.1 | L62「In-app stream resume after background」TUI 列/Notes | TUI 全屏补 `/resume`，Notes 补充（原仅 repl） |
| W1.2/W1.3/W1.4/W1.5 | L53「Full-screen TUI」Notes / L60「Control slashes」 | Notes 追加排队/草稿/复制/过滤描述 |
| W1.6 | L53「Full-screen TUI」Notes | 多行输入一栏补「整段粘贴经括号粘贴一次落地，`CM_TUI_PASTE=off` 可关」 |
| W2.1 | L53 Notes「switching sessions starts a fresh transcript」 | 已同步：切换回放最新页 + `/older` 经 `before_index` 前插（80 条/页），并平移工具行 / 搜索行号 |
| W2.2 | L62「Web session list + resume…」TUI 列/Notes | 删除已补（`Delete` / `Shift+Delete`）；重命名待 serve 端点 |
| W2.3 | L59「Ask/Plan/Act…」附近或新增行 | 视方案形态在 chat/tools 段新增或并入 Notes |
| W2.4 | L63「Control slashes」TUI 列/Notes | 已同步：TUI 补 `/conv export [json\|markdown\|both] [--out <路径>] [--id <id>]`（与 Desktop 同一份展示投影）；仍标 `reduced`（无系统文件对话框，路径由用户给） |
| W3.1 | L60「Control slashes」TUI 列 | 已同步（TUI 有 `/` 补全菜单，含 skills 候选） |
| W3.2 | L55「`POST /chat/stream` + command approval」TUI Notes | 补 `Ctrl+E` 展开看完整命令（折行 + 滚动）、收起档单行截断 |
| W3.3 | L53「Full-screen TUI」Notes | 补「< 120 列时左右栏隐藏，当前工作区无指示」（G10）**→ 已落地，#425**：窄屏底栏 `ws` 段 |
| W3.3（链接） | L57「Full-screen TUI」Notes | 已同步：`Ctrl+单击` 打开正文链接（应用内命中表，不依赖 OSC 8；SSH 降级为复制；只放行 http(s)） |
| W3.3（工具详情） | L57「Full-screen TUI」Notes | 已同步：`/detail [n]` 展开单次工具调用的输出全文（只读浮层、可滚动；侧表 64K 字符/条、4M 总量；只覆盖本进程跑过的工具） |
| W3.4 | L54「Full-screen TUI」Notes | 补状态栏 `ctx` 上下文用量段（`tiktokenPromptTokens`（`RUN_FINISHED` / `conversation_saved`） + 会话内水位；口径来自 S11 `client-api::context_axis`；段恒在，无实测时占位 `—`）——已同步 |
| W3.5 | 同格 Notes | 已修（见 G11）：`timeline_log` `kind=context_trim` 的 `after_tokens` 会把水位**回落**；另加两处清理 —— `/conv new` 与切换会话时 `UiState::clear_usage` 清空快照与水位，避免新会话仍显示上一个会话的数字；新会话（无 cid）回落 `/status` 的 `tiktoken_new_session_baseline_by_agent_role` 并显示 `~` 估算前缀（对齐 Desktop）；另：首轮结束绑上 cid 后若这一轮仍无实测，回合末补拉 `GET /conversation/messages` 的快照（同 Desktop 流收尾水合，回包按 cid 门控），不再从 `~12k` 掉回 `—` |
| W3.6 | L68「Per-turn inject / trim transcript notes」TUI 列 + Notes | 已落地（#478）：TUI 从 `no` → `partial` —— `context_trim` 每轮落**一行** `System`（`上下文已压缩：312.4k → 251.1k`；同轮第二次起改写该行并计次；无可信前置值时只报当前量，退化形态见 `serve::trim_line`）。`context_inject` 仍只有 skill 行，且**不加** Desktop 那个默认关闭的旁注开关。文本模式（`chat` / `repl`）同一句、无箭头。措辞三条准则随代码固化在 `trim_line.rs` 模块头：最小事实优先（差量是增强，缺前置不退化成沉默）、不陈述自己不知道的因（契约只给 `after_tokens`）、屏幕一行 / 细节走 `CM_TUI_SSE_DUMP`，不为一行提示加设置项 |
| W3.7（#508） | L57「Full-screen TUI」Notes | 已同步：按文件类型着色（目录 / 代码 / 配置 / 文档 / 图片 / 其它）+ 2 列标记；4 个终端专用色槽（不改 CSS / Desktop 主题）；`CM_TUI_FILE_ICONS=off` 关标记 |

> 矩阵维护规则：改了用户可见能力就必须在同 PR 更新对应格（§顶注），不允许“下个 PR 再补”。

---

## 5. 验证

- **单测**：Wave 1 以 `state.rs` / `controls.rs` 纯逻辑为主（断点记录、队列状态机、草稿表换入换出、过滤词匹配、OSC52 序列转义），沿用 `render_tests.rs` / `settings_*_tests.rs` 的写法。
- **手测**（需真实 TTY + 本地 `crabmate serve`）：
  - W1.1：`tui` 中 `Ctrl+C` 未送达取消（或断网）→ `/resume` 续上；已确认取消则不可续。
  - W1.2：长回合中输入第二句 → 出现「已排队」系统行 → 回合**正常**结束才自动发出；取消 / 出错应退回输入框；输入框焦点下 Esc 清队（在侧栏 / 工作区树里要先 Esc 回输入框）。
  - W1.3：会话 A 输入半句 → 切 B 再切回 A → 半句仍在；`/conv new` 后为空。退出进程再启动、切回 A → 半句仍在（本机 `tui-drafts.json`）；`cat` 该文件能看到按 `api_base` 分区的条目，手改成坏 JSON 后启动仍正常（静默回落空表）。
  - W1.4：kitty/wezterm 下复制助手消息/代码块 → 粘贴到编辑器；不支持终端给出提示。
  - W1.5：`/conv filter 关键词` → 左栏只剩匹配项、标题显示 `会话 [m/n] · filter: 关键词` → `Tab` 进左栏 `↑↓` 只在结果间走、`Enter` 使用；`/conv filter off`（或空参）恢复全量；无匹配时显示 `（无匹配）` 且 `Enter`/`Delete` 不误伤。
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
