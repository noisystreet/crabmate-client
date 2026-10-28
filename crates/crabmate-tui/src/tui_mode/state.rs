//! 全屏 TUI 的 UI 状态：transcript 行 + 输入缓冲 + 回合/会话指示 +
//! 审批浮层 / 工具行 / 搜索 / thinking 折叠等交互状态。
//!
//! 工作区目录树的状态逻辑在 [`super::workspace_tree`]（拆分以控制单文件行数）。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use crate::serve::{ApprovalDecision, CommandApprovalData, SessionListRow, WorkspaceDirEntry};

use super::approve::ApprovalChoice;
use super::palette::Palette;
use super::serve_defaults::ServeDefaults;
use super::slash_menu::{SkillsCache, SkillsPayload, SlashItem, filtered_items, slash_prefix};
use super::tool_summary::{tool_end_text, tool_row_text};
use super::workspace_tree::{WsPickState, WsRow};

// 搜索的命中集合与跳转：拆出去是为了增量维护（旧实现每个流式增量全量重扫，见文件头）。
#[path = "state_search.rs"]
mod search;

// 粘贴落地（`insert_paste`）：子模块才能直接写私有字段 `input` / `cursor`，顺带压住本文件行数。
#[path = "state_paste.rs"]
mod paste;

// 上下文用量（`tiktokenPromptTokens` 快照 → 状态行 `ctx` 段）：同上，拆出去压 `state.rs` 行数。
#[path = "state_context.rs"]
mod context;

// 历史分页（`/older` 前插）：与回放同源，单独成文件压 `state.rs` 行数。
#[path = "state_history.rs"]
mod history_page;

pub use history_page::HistoryPage;

/// `/` 补全菜单的会话内状态。
#[derive(Debug, Default)]
pub struct SlashMenuState {
    /// 选中项下标（超界时由输入变化后的收敛逻辑拉回）。
    pub cursor: usize,
    /// 本次 `/` 前缀态里被 Esc 关掉（草稿离开前缀态后自动解除）。
    pub dismissed: bool,
}

/// transcript 行类别（决定前缀与配色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    User,
    Assistant,
    Thinking,
    Tool,
    System,
}

/// 键盘焦点：底栏输入框、左栏会话列表或左栏工作区目录树。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Input,
    /// 左栏会话列表。
    Sidebar,
    /// 左栏工作区目录树（`GET /workspace[?path=]`，仿 Desktop 工作区侧栏）。
    Workspace,
}

/// 审批浮层：serve 等待决策期间 SSE 回合任务阻塞在应答通道上，
/// UI 事件循环保持运行收按键（双向握手，Esc=n 拒绝 / Enter=y 一次 / a 始终）。
#[derive(Debug)]
pub struct ApprovalOverlay {
    pub command: String,
    pub args: String,
    pub allowlist_key: Option<String>,
    /// 决策回传通道（worker 的 gate 阻塞在对应 recv 上）。
    pub answer: Sender<ApprovalDecision>,
    /// 「看完整命令」展开态：收起时命令区只占 1 行（截断），展开时铺满可用高度。
    pub expanded: bool,
    /// 展开态下命令区的滚动起点（行号，渲染时再夹一次上界）。
    pub scroll: usize,
}

impl ApprovalOverlay {
    /// 构造决策前的展示文本（与 repl 提示一致）。
    #[must_use]
    pub fn preview(&self) -> String {
        let cmd = self.command.trim();
        let args = self.args.trim();
        if args.is_empty() {
            cmd.to_string()
        } else {
            format!("{cmd} {args}")
        }
    }
}

/// 工具行的状态（仅 `LineKind::Tool` 有意义；其余行恒为 [`ToolState::Finished(None)`]）。
///
/// 曾经用 `tool_ok: Option<bool>` 兼表三态：`None` 同时是「进行中」与「已结束但服务端
/// 没给成败」。渲染层只能靠文本里那句 `(done)` 去猜，而前缀与 spinner 仍按「进行中」
/// 画，于是结束的调用看起来还在跑。拆成显式状态后，两者不再互相冒充。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    /// 调用中：前缀带进度位，末行还会叠 spinner 动画。
    Running,
    /// 已结束。`None` = 服务端没给成败（不猜，标记显示 `?`）。
    Finished(Option<bool>),
}

impl ToolState {
    /// 成败（仅 `Finished` 有值）。
    #[must_use]
    pub fn ok(self) -> Option<bool> {
        match self {
            Self::Running => None,
            Self::Finished(ok) => ok,
        }
    }

    #[must_use]
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// 行版本号的来源：全局单调递增。
///
/// 用全局计数器、而不是「每行自己 +1」：后者下，两条内容不同但都刚构造出来的行
/// 版本都是 0，行级缓存分不开 —— 把整批行换成另一批等长的行时就会漏判。
/// 全局计数保证「任何一次构造 / 修改」都拿到互不相同的值。
static NEXT_LINE_REV: AtomicU64 = AtomicU64::new(1);

fn next_rev() -> u64 {
    NEXT_LINE_REV.fetch_add(1, Ordering::Relaxed)
}

/// 一条 transcript 记录。
///
/// 字段私有、改内容只走下面几个方法：body 行级缓存只凭 [`LogLine::rev`] 判断
/// 「这一行变没变」，谁绕过方法直接改文本，缓存就会一直显示旧内容。
/// 收成私有等于让编译器替人记着这条规矩。
#[derive(Debug, Clone)]
pub struct LogLine {
    kind: LineKind,
    text: String,
    /// thinking 折叠态（默认折叠；Ctrl+E 切换全局展开）。
    collapsed: bool,
    /// 工具行状态；仅供渲染分色与进度位，不参与文本（文本仍是 `tool_end_text` 的完整串）。
    tool_state: ToolState,
    /// 行版本：内容 / 折叠 / 工具状态每变一次就换一个新值，供 body 行级缓存做指纹（O(1)）。
    /// 不用「文本长度」或逐行哈希：前者漏等长替换，后者每帧都要重算一遍全文。
    rev: u64,
}

impl LogLine {
    /// `collapsed` 由调用方按全局 thinking 展开态传入（thinking 默认折叠，
    /// 用户 Ctrl+E 展开后新建的 thinking 行也应保持展开）。
    fn new(kind: LineKind, text: &str, collapsed: bool) -> Self {
        Self {
            kind,
            text: text.to_string(),
            collapsed,
            // 非工具行没有状态可言；工具行由 `tool_start` 换成 `Running`。
            tool_state: ToolState::Finished(None),
            rev: next_rev(),
        }
    }

    /// 同上，但给定初始工具状态（回放的历史行 / 刚起的工具行）。
    fn with_tool_state(kind: LineKind, text: &str, collapsed: bool, tool_state: ToolState) -> Self {
        Self {
            tool_state,
            ..Self::new(kind, text, collapsed)
        }
    }

    /// 从落盘数据还原一行（历史回放）：文本与成败一次落定。
    pub(crate) fn replayed(
        kind: LineKind,
        text: &str,
        collapsed: bool,
        tool_ok: Option<bool>,
    ) -> Self {
        Self::with_tool_state(kind, text, collapsed, ToolState::Finished(tool_ok))
    }

    #[must_use]
    pub(crate) fn kind(&self) -> LineKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub(crate) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    #[must_use]
    pub(crate) fn tool_state(&self) -> ToolState {
        self.tool_state
    }

    /// 行版本 —— body 行级缓存的指纹。
    #[must_use]
    pub(crate) fn rev(&self) -> u64 {
        self.rev
    }

    fn touch(&mut self) {
        self.rev = next_rev();
    }

    fn append(&mut self, delta: &str) {
        self.text.push_str(delta);
        self.touch();
    }

    /// 工具结果回填：文本与状态一起换。
    fn finish(&mut self, text: String, ok: Option<bool>) {
        self.text = text;
        self.tool_state = ToolState::Finished(ok);
        self.touch();
    }

    /// 折叠态变化（没变就不动版本，免得白白让缓存重建）。
    fn set_collapsed(&mut self, collapsed: bool) {
        if self.collapsed != collapsed {
            self.collapsed = collapsed;
            self.touch();
        }
    }
}

/// 在途的历史回放请求（`GET /conversation/messages`）。
///
/// 携带请求前的绑定与标题：回包要用它们判断「这次回包还算不算数」（用户可能已经
/// 切走或已经开聊），并在失败时把绑定回滚——否则会拿一个可能已被服务端删掉的 id 继续聊。
#[derive(Debug, Clone)]
pub struct HistoryRequest {
    pub conversation_id: String,
    /// 请求前的绑定（`None` = 新会话）。
    pub prev_conversation_id: Option<String>,
    /// 会话标题（提示文案用；没有就是 `(by id)`）。
    pub title: String,
}

/// 「贴到内容顶」的哨兵偏移：远大于任何真实行数，渲染时被 clamp 到 `max_back`。
///
/// 用哨兵而不是 `max_back` 本身，是因为状态层不知道物理行数与视口高度 ——
/// 每次渲染都会重算 clamp，哨兵自然贴到**当前**内容顶（内容变长后也一样）。
pub const SCROLL_TOP: usize = usize::MAX / 4;

/// 全屏 UI 状态；仅由事件循环触碰（单线程）。
#[derive(Debug, Default)]
pub struct UiState {
    pub lines: Vec<LogLine>,
    /// 终端配色板（随 `UserPrefsDto.theme`；启动时解析一次，不随帧变）。
    pub palette: Palette,
    /// 输入缓冲（Vec<char> 便于光标插入；含 `\n` 即多行编辑）。
    input: Vec<char>,
    /// 输入光标位置（字符下标）。
    cursor: usize,
    pub conversation_id: Option<String>,
    /// 在途的历史回放请求（`None` = 没有正在等的回放）。
    ///
    /// 用户一发送消息或新建会话就清空：那时的回包已经没有落点，清屏会丢真实内容。
    pub history_pending: Option<HistoryRequest>,
    /// 历史分页游标（`/older`）；`None` = 当前 transcript 不能往前翻。
    pub history_page: Option<HistoryPage>,
    /// 是否有回合在跑（SSE 任务未结束）。
    pub running: bool,
    /// 当前回合是否已发过取消（第二次 Ctrl+C = 强退）。
    pub cancel_sent: bool,
    /// 回合进行中按 Enter 排队的下一句（回合**正常**结束后自动发出；Esc 清队）。
    ///
    /// 只保留最后一条：与 Desktop 的排队 chip 同语义，连按 Enter 视为「改主意」。
    /// 取消 / 出错的回合不自动发出，见 `drafts::queued_outcome`。
    pub queued: Option<String>,
    /// 从底部回看的物理行数（0 = 跟随最新；`SCROLL_TOP` = 贴到内容顶）。
    pub view_offset: usize,
    /// transcript 视口是否已贴到内容顶（`render_body` 每帧回写）。
    ///
    /// 只有渲染层知道 `max_back`（物理行数 − 视口高度），所以「在顶」由它判定；
    /// `/older` 前插据此决定是否继续贴顶（否则刚加载的那页会落在视口上方看不见）。
    pub body_at_top: bool,
    /// PgUp/PgDn 页步（由事件循环按终端高度刷新）。
    pub page_rows: usize,
    /// 键盘焦点（输入框 / 左栏会话）。
    pub focus: Focus,
    /// 左栏会话列表（`fetch_web_sessions` 快照，未经本地转换）。
    pub sessions: Vec<SessionListRow>,
    /// 左栏选中项下标。
    pub selected: usize,
    /// 待确认删除的会话行 id（`Delete` 第一次按下；`Shift+Delete` / 确认后清空）。
    ///
    /// 放 `UiState` 而不是 `LaneApp`：侧栏底部的提示行要按它切换成确认文案。
    pub confirm_delete: Option<String>,
    /// serve 默认偏好（`/status` 拉取）。
    pub serve_defaults: Option<ServeDefaults>,
    /// 上下文用量（ADE 0005 同口径）：会话内水位 + 最近一次 `tiktokenPromptTokens` 快照。
    pub context: context::ContextUsage,
    /// serve 侧当前活跃会话（左栏 `*` 标记）。
    pub active_session_id: Option<String>,
    /// 当前是否宽到显示左栏（由渲染层每帧刷新）。
    pub sidebar_visible: bool,
    /// 当前工作区路径（`GET /workspace`；右栏标题显示，未获取为 None）。
    pub workspace_path: Option<String>,
    /// 工作区目录树：根列表是否已成功加载（false 时渲染占位）。
    pub ws_ready: bool,
    /// 是否有根目录拉取在途（避免重复入队）。
    pub ws_root_pending: bool,
    /// 根目录最近一次拉取失败文案（侧栏占位提示；成功后清空）。
    pub ws_root_err: Option<String>,
    /// 目录相对路径 → 已加载子条目缓存（`""` = 根）。
    pub ws_dir_cache: HashMap<String, Vec<WorkspaceDirEntry>>,
    /// 工作区代数：每次**重置**树（首次/切换工作区）自增。
    ///
    /// 读请求现在是并发发出的，回包顺序不再与发起顺序一致。带上发起时的代数，
    /// 回包时代数不符就说明中途切走了 —— 那条结果属于旧工作区，必须丢弃，
    /// 否则会把 A 的目录内容写进 B 的缓存。
    pub ws_generation: u64,
    /// 缓存已过期、展开时该后台重拉的路径。
    ///
    /// 刷新不再清空缓存（否则整棵树会短暂塌成 `…`），改为标记过期：旧值继续显示，
    /// 折叠目录下次展开时补拉一次，这样刷新也能覆盖到当时没展开的那些。
    pub ws_dir_stale: HashSet<String>,
    /// 已展开的目录相对路径。
    pub ws_expanded: HashSet<String>,
    /// 子目录列表在途的路径（防重复请求）。
    pub ws_loading: HashSet<String>,
    /// 扁平化后的可见树行（数据/展开态变化后重算）。
    pub ws_rows: Vec<WsRow>,
    /// 树光标（`ws_rows` 下标，超界时渲染前收敛）。
    pub ws_cursor: usize,
    /// 刷新时目标行在展开目录内、需等子列表重载后才能定位的 rel（best-effort）。
    pub ws_pending_cursor: Option<String>,
    /// 项目池「选择/切换工作区」子视图（`Some` 时工作区列改列候选项目）。
    pub ws_pick: Option<WsPickState>,
    /// 审批浮层（回合暂停等待决策）。
    pub approval: Option<ApprovalOverlay>,
    /// 审批浮层里方向键选中的那一项（浮层关闭后保留，下次打开时复位）。
    pub approval_choice: ApprovalChoice,
    /// 格子背景采样表（`/background` 在无像素协议的终端上退到这一档；默认 `None`）。
    ///
    /// 放 `UiState` 而不是 `LaneApp`：只有渲染层读它，这样 `draw` 的签名不必多一个参数
    /// （那会波及 `render_tests` 里几十处调用）。图像本身仍由 `background::Background` 持有。
    pub bg_cells: Option<super::background::BgCells>,
    /// 工具调用 id → transcript 行号（结果到达时原位更新摘要行）。
    tool_pending: HashMap<String, usize>,
    /// 全局 thinking 展开开关（默认折叠）。
    thinking_visible: bool,
    /// 搜索词（/find；lowercase，trim）。
    search: Option<String>,
    /// 当前搜索锚定的逻辑行下标（None = 无锚点，随手动滚动释放）。
    pub search_cursor: Option<usize>,
    /// 当前搜索命中的逻辑行数（状态行提示用）。
    pub search_total: usize,
    /// 当前搜索命中的逻辑行下标（升序、无重复；`search_total` 恒等于其长度）。
    ///
    /// 独立于 `lines` 的扫描结果存在，是为了流式期只重判被改动的那一行。
    pub(super) search_hits: Vec<usize>,
    /// `/` 补全菜单（输入期候选提示，不影响提交语义）。
    pub slash: SlashMenuState,
    /// serve 的 skill 目录缓存（`GET /skills`；`None` = 还没拉过）。
    pub skills: Option<SkillsCache>,
    /// skill 目录拉取在途（防重复请求）。
    pub skills_loading: bool,
    /// 右栏底部状态栈是否折叠（`F3`）：`false` = 展开；窄屏无状态栈，此开关无效果。
    pub status_stack_collapsed: bool,
    /// 权限写入代数（连选两档只认最后一次回包）与 `perm` 段菜单（`None` = 未打开）。
    pub permission_generation: u64,
    pub permission_menu: Option<crate::tui_mode::permission_intent::PermissionMenu>,
}

impl UiState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_rows: 12,
            // 拿到 user-data 前先落默认（透明）主题；prefs 到达后由 serve 的主题覆盖。
            palette: Palette::resolve(None),
            ..Self::default()
        }
    }

    pub fn push_line(&mut self, kind: LineKind, text: &str) {
        // thinking 默认折叠；但"全局展开"开启时新建的 thinking 行也应保持展开。
        let collapsed = kind == LineKind::Thinking && !self.thinking_visible;
        self.lines.push(LogLine::new(kind, text, collapsed));
        self.search_on_line_added();
    }

    /// 流式增量：若最后一条同类（Assistant/Thinking）则续接，否则开新行。
    pub fn stream_delta(&mut self, kind: LineKind, delta: &str) {
        if delta.is_empty() {
            return;
        }
        let mergeable = matches!(kind, LineKind::Assistant | LineKind::Thinking);
        if mergeable && self.lines.last().is_some_and(|l| l.kind() == kind) {
            self.lines.last_mut().expect("checked last").append(delta);
        } else {
            self.push_line(kind, delta);
        }
        // 流式续接只改了最后一行：只重判它，不重扫全表。
        self.search_on_last_line_changed();
    }

    /// 最后一行是什么类型（`/resume` 判断续流正文该接上原块还是另起一块）。
    #[must_use]
    pub fn last_kind(&self) -> Option<LineKind> {
        self.lines.last().map(|l| l.kind())
    }

    /// 应用配色板（启动拉到 user-data 后一次、设置面板改主题时；主题是渲染输入，
    /// body 缓存自己按它失效，这里只需落到状态上）。
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    // ── 输入缓冲（单行/多行统一） ─────────────────────────────

    pub fn current_input(&self) -> String {
        self.input.iter().collect()
    }

    /// 输入缓冲是否多行（含 `\n`）。
    #[must_use]
    pub fn is_multiline(&self) -> bool {
        self.input.contains(&'\n')
    }

    /// 取走输入并清空（发送后调用）。
    pub fn take_input(&mut self) -> String {
        let s: String = self.input.drain(..).collect();
        self.cursor = 0;
        s
    }

    // ── 忙时排队（回合结束后自动发送） ─────────────────────────

    /// 排入「回合结束后自动发送」的下一句（覆盖前一条）。
    pub fn queue_message(&mut self, text: &str) {
        self.queued = Some(text.to_string());
    }

    /// 是否有排队消息（回合结束时据此决定要不要动输入框）。
    #[must_use]
    pub fn has_queued(&self) -> bool {
        self.queued.is_some()
    }

    /// 取走排队消息（回合结束时调用）。
    pub fn take_queued(&mut self) -> Option<String> {
        self.queued.take()
    }

    /// 清掉排队消息（Esc）；返回是否确有内容被清掉。
    pub fn clear_queued(&mut self) -> bool {
        self.queued.take().is_some()
    }

    /// 输入框当前全文（不动光标、不清空）：退出前把草稿写回会话行用。
    #[must_use]
    pub fn input_snapshot(&self) -> String {
        self.input.iter().collect()
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor >= self.input.len() {
            self.input.push(c);
            self.cursor = self.input.len();
        } else {
            self.input.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// 光标处插入换行（Alt+Enter 多行编辑）。
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// 工作区树里回车选中文件：把相对路径插入输入框。
    ///
    /// 两侧都只在会粘住相邻文字时补空格；光标在末尾时右侧固定补一个空格，便于接着写问题。
    pub fn insert_workspace_path(&mut self, rel: &str) {
        if rel.is_empty() {
            return;
        }
        let glued_left = self.cursor > 0 && !self.input[self.cursor - 1].is_whitespace();
        if glued_left {
            self.insert_char(' ');
        }
        for c in rel.chars() {
            self.insert_char(c);
        }
        let at_end = self.cursor == self.input.len();
        let glued_right = !at_end && !self.input[self.cursor].is_whitespace();
        if at_end || glued_right {
            self.insert_char(' ');
        }
    }

    /// 整串替换输入缓冲（`/` 菜单接受候选时用；光标落到末尾）。
    pub fn set_input(&mut self, text: &str) {
        self.input = text.chars().collect();
        self.cursor = self.input.len();
    }

    // ── `/` 补全菜单（输入期候选提示，与提交时的控制斜杠解析无关） ──────

    /// 草稿处于候选态、且没被 Esc 关掉。
    #[must_use]
    pub fn slash_menu_open(&self) -> bool {
        !self.slash.dismissed && slash_prefix(&self.current_input()).is_some()
    }

    /// 当前候选；菜单关闭 / 非前缀态时为空。
    #[must_use]
    pub fn slash_items(&self) -> Vec<SlashItem> {
        if self.slash.dismissed {
            return Vec::new();
        }
        let draft = self.current_input();
        let Some(prefix) = slash_prefix(&draft) else {
            return Vec::new();
        };
        filtered_items(prefix, self.skills.as_ref())
    }

    pub fn slash_dismiss(&mut self) {
        self.slash.dismissed = true;
    }

    /// 环形步进选择（`len == 0` 时不动）。
    pub fn slash_step(&mut self, len: usize, delta: i32) {
        if len == 0 {
            return;
        }
        self.slash.cursor = ((self.slash.cursor as i32 + delta).rem_euclid(len as i32)) as usize;
    }

    /// 接受光标项：整串替换草稿并收起菜单；无候选时返回 `false`（放行原本的按键语义）。
    pub fn slash_accept(&mut self) -> bool {
        let items = self.slash_items();
        let Some(item) = items.get(self.slash.cursor) else {
            return false;
        };
        let insert = item.insert.clone();
        self.set_input(&insert);
        // 接受后立刻收起：`/help` 这类无参命令接受后草稿仍是前缀态，否则马上又弹回来。
        self.slash.dismissed = true;
        self.slash.cursor = 0;
        true
    }

    /// 输入变化后统一收敛：离开前缀态就解除 Esc 关闭；光标钳到合法范围。
    pub fn slash_after_input(&mut self) {
        if slash_prefix(&self.current_input()).is_none() {
            self.slash.dismissed = false;
            self.slash.cursor = 0;
        }
        let n = if self.slash.dismissed {
            0
        } else {
            self.slash_items().len()
        };
        if n == 0 {
            self.slash.cursor = 0;
        } else if self.slash.cursor >= n {
            self.slash.cursor = n - 1;
        }
    }

    /// skill 目录拉取结果落地。失败也缓存：每次打开菜单都重试同一个错只会刷屏。
    pub fn set_skills(&mut self, result: Result<SkillsPayload, String>) {
        self.skills_loading = false;
        self.skills = Some(match result {
            Ok(p) => SkillsCache::from_payload(p),
            Err(e) => SkillsCache {
                enabled: false,
                skills: Vec::new(),
                err: Some(e),
            },
        });
    }

    /// 切换工作区后 skill 目录可能整套换掉：清缓存并复位菜单态。
    pub fn clear_skills(&mut self) {
        self.skills = None;
        self.skills_loading = false;
        self.slash = SlashMenuState::default();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let idx = self.cursor - 1;
        self.input.remove(idx);
        self.cursor = idx;
    }

    pub fn cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Home：当前行首（单行输入等价整串开头）。
    pub fn cursor_home(&mut self) {
        if let Some(line_start) = self.cursor_line_start() {
            self.cursor = line_start;
        } else {
            self.cursor = 0;
        }
    }

    /// End：当前行尾（不含行尾 `\n`）。
    pub fn cursor_end(&mut self) {
        if let Some(line_end) = self.cursor_line_end() {
            self.cursor = line_end;
        } else {
            self.cursor = self.input.len();
        }
    }

    /// 光标行起始字符下标。
    fn cursor_line_start(&self) -> Option<usize> {
        self.input[..self.cursor.min(self.input.len())]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(Some(0), |nl| Some(nl + 1))
    }

    /// 光标行结束字符下标（不含行尾 `\n`）。
    fn cursor_line_end(&self) -> Option<usize> {
        let start = self.cursor_line_start()?;
        let rest = &self.input[start.min(self.input.len())..];
        rest.iter()
            .position(|&c| c == '\n')
            .map_or_else(|| Some(self.input.len()), |nl| Some(start + nl))
    }

    /// 光标所在行号与行内字符列。
    #[must_use]
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, &c) in self.input.iter().enumerate() {
            if i >= self.cursor {
                break;
            }
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// 每行行首字符下标（含空串行）。
    fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0usize];
        for (i, &c) in self.input.iter().enumerate() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    /// 按 `\n` 拆分输入为各行（不含换行符）。
    #[must_use]
    pub fn input_lines(&self) -> Vec<String> {
        self.input
            .split(|&c| c == '\n')
            .map(|s| s.iter().collect())
            .collect()
    }

    /// 输入总行数。
    #[must_use]
    pub fn input_line_count(&self) -> usize {
        self.input.iter().filter(|&&c| c == '\n').count() + 1
    }

    /// 上/下行移动光标（列尽量保持，超出 clamp 到行尾）。
    pub fn move_cursor_vert(&mut self, up: bool) {
        let (line, col) = self.cursor_line_col();
        let count = self.input_line_count();
        if (up && line == 0) || (!up && line + 1 >= count) {
            return;
        }
        let starts = self.line_starts();
        let target = if up { line - 1 } else { line + 1 };
        let start = starts[target];
        let end = starts.get(target + 1).copied().unwrap_or(self.input.len());
        let line_cols = self.input[start..end.min(self.input.len())]
            .iter()
            .take_while(|&&c| c != '\n')
            .count();
        self.cursor = start + col.min(line_cols);
    }

    /// 替换会话列表并 clamp 选中项。
    pub fn replace_sessions(&mut self, sessions: Vec<SessionListRow>) {
        self.sessions = sessions;
        self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
    }

    /// 上/下移动左栏选中（空列表时不变）。
    pub fn move_selection(&mut self, up: bool) {
        if self.sessions.is_empty() {
            return;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        } else if self.selected + 1 < self.sessions.len() {
            self.selected += 1;
        }
    }

    /// 当前选中会话的可续聊 server conversation_id（无则 `None`）。
    #[must_use]
    pub fn selected_resume(&self) -> Option<String> {
        self.sessions.get(self.selected).and_then(|s| {
            s.server_conversation_id
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_string)
        })
    }

    /// 会话行是否正是当前 `conversation_id`（左栏标记 `>`）。
    #[must_use]
    pub fn row_in_use(&self, row: &SessionListRow) -> bool {
        self.conversation_id
            .as_deref()
            .is_some_and(|c| row.server_conversation_id.as_deref() == Some(c))
    }

    // ── 审批浮层 ─────────────────────────────────────────────

    /// 收到命令审批请求：记录浮层（供渲染 + 按键应答）。
    pub fn begin_approval(&mut self, req: &CommandApprovalData, answer: Sender<ApprovalDecision>) {
        self.approval = Some(ApprovalOverlay {
            command: req.command.clone(),
            args: req.args.clone(),
            allowlist_key: req.allowlist_key.clone(),
            answer,
            // 每次新请求都从未展开、未滚动开始：上一条审批展开到第 40 行，不该让
            // 下一条一打开就停在半截命令上。
            expanded: false,
            scroll: 0,
        });
        // 每次新请求都把光标复位到默认项：上一条审批停在哪一项，不该影响下一次的
        // 默认回答（否则「上次点了始终允许」会让下一条审批一回车就自动放行）。
        self.approval_choice = ApprovalChoice::default();
    }

    // ── 工具行摘要 ───────────────────────────────────────────

    /// 工具开始：记一个「进行中」摘要行，RESULT 到达时原位补标记与状态。
    ///
    /// `note` 是事件里已经给出的「要执行什么」（`TOOL_CALL_START` 的 summary /
    /// argsPreview）——一开始就显示，别等结果回来才让用户看到跑的是什么命令。
    pub fn tool_start(&mut self, tool_call_id: &str, name: &str, note: Option<&str>) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let idx = self.lines.len();
        self.lines.push(LogLine::with_tool_state(
            LineKind::Tool,
            &tool_row_text(display, note),
            false,
            ToolState::Running,
        ));
        // 没有 id 就无法按 id 找回这一行；结果到达时会另起一行，本行就此停在
        // `Running`——与「id 缺失等于放弃了原位更新」的既有语义一致。
        if !tool_call_id.is_empty() {
            self.tool_pending.insert(tool_call_id.to_string(), idx);
        }
        self.search_on_line_added();
    }

    /// 工具结果：更新对应摘要行（note 与工具名重复时省略前缀，如 `✓ read file: hello.cpp`）；
    /// 找不到 start（如续流中段）则补一行收尾。
    pub fn tool_end(
        &mut self,
        tool_call_id: &str,
        name: &str,
        ok: Option<bool>,
        note: Option<&str>,
    ) {
        let display = if name.trim().is_empty() { "tool" } else { name };
        let text = tool_end_text(display, ok, note);
        let idx = self.tool_pending.remove(tool_call_id);
        match idx {
            Some(i)
                if self
                    .lines
                    .get(i)
                    .is_some_and(|l| l.kind() == LineKind::Tool) =>
            {
                let line = &mut self.lines[i];
                line.finish(text, ok);
                // 原位替换了文本：命中集合要跟着改（可能是中间行，不是末尾）。
                self.search_on_line_changed(i);
            }
            _ => {
                // 没有对应的 start 行（续流中段）：直接补一行「已结束」。
                self.lines.push(LogLine::with_tool_state(
                    LineKind::Tool,
                    &text,
                    false,
                    ToolState::Finished(ok),
                ));
                self.search_on_line_added();
            }
        }
    }

    /// 回合开始时清理上轮工具行映射。
    pub fn reset_run_tools(&mut self) {
        self.tool_pending.clear();
    }

    // ── thinking 折叠 ────────────────────────────────────────

    /// 全局展开 / 折叠所有 thinking 行（桌面语义默认折叠）。
    pub fn set_thinking_visible(&mut self, visible: bool) {
        self.thinking_visible = visible;
        for line in &mut self.lines {
            if line.kind() == LineKind::Thinking {
                line.set_collapsed(!visible);
            }
        }
    }

    pub fn thinking_visible(&self) -> bool {
        self.thinking_visible
    }

    /// Ctrl+E：切换 thinking 展开/折叠。
    pub fn toggle_thinking(&mut self) {
        self.set_thinking_visible(!self.thinking_visible);
    }

    /// Ctrl+E 的完整动作：切换后再回报一行提示。
    ///
    /// 与 `toggle_thinking` 分开纯为复杂度：`on_input_key` 的 match 已贴着
    /// lizard 的 CCN 门禁（≤ 10），内联的 if/else + 提示格式化把它顶到了 11。
    pub fn toggle_thinking_with_notice(&mut self) {
        self.toggle_thinking();
        let state = if self.thinking_visible() {
            "展开"
        } else {
            "折叠"
        };
        self.push_line(
            LineKind::System,
            &format!("思考内容已{state}（Ctrl+E 切换）"),
        );
    }

    // ── 右栏状态栈折叠（F3） ────────────────────────────────

    /// 切换右栏底部状态栈的展开 / 折叠。
    ///
    /// 窄屏没有状态栈（`layout::panes` 的 `right_status` 为 `None`），此时开关
    /// **无效果但不报错** —— 不为它再加一处「窄屏改文案」的分支。
    pub fn toggle_status_stack(&mut self) {
        self.status_stack_collapsed = !self.status_stack_collapsed;
    }

    // 搜索（`/find`）的状态与跳转见 `state_search.rs`：命中集合增量维护，
    // 不再在 `stream_delta` 里全量重扫。

    pub fn scroll_page(&mut self, up: bool) {
        let step = self.page_rows.max(1);
        if up {
            self.view_offset = self.view_offset.saturating_add(step);
        } else {
            self.view_offset = self.view_offset.saturating_sub(step);
        }
        self.release_search_anchor();
    }

    pub fn scroll_top(&mut self) {
        self.view_offset = SCROLL_TOP;
        self.release_search_anchor();
    }

    pub fn scroll_bottom(&mut self) {
        self.view_offset = 0;
        self.release_search_anchor();
    }

    pub fn scroll_lines(&mut self, up: bool, by: usize) {
        if up {
            self.view_offset = self.view_offset.saturating_add(by);
        } else {
            self.view_offset = self.view_offset.saturating_sub(by);
        }
        self.release_search_anchor();
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
