//! 左栏下半的 lane 列表渲染：每条连接一行整行宽的底色条，自上而下堆叠。
//!
//! 单独成文件有两个理由：`render.rs` 贴着 920 行门禁；而这块逻辑与「怎么画一个
//! pane」无关，只依赖调色板与截断工具，拆开后两边都不必为对方让路。
//!
//! 版面口径全部来自 `layout`（`lane_shown_len` / `lane_index_at`）：
//! 一行对应哪条 lane 与鼠标命中必须是同一份算法，否则会出现「点第 2 行切到第 3 条」。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::layout::lane_shown_len;
use super::palette::Palette;
use super::pulse;
use super::status::reduced_motion;
use super::text::truncate_display;
use super::worker::{LaneId, UiEvent};

/// 名字至少留几列才值得画尾部读数：不足就整段尾部让位给名字。
const LANE_LABEL_MIN: usize = 4;

/// 外壳层算好的单条 lane 快照：只含**渲染需要**的字段。
///
/// 故意不导 lane 内部状态（transcript / 草稿 / 游标）：渲染层拿不到 `&mut`，
/// 也就不可能在某条 lane 的绘制里改到别人。
///
/// `Default` 是为了让测试只写关心的那几个字段 —— 这里的字段只会越加越多，
/// 每加一个就改一遍四个测试构造函数，迟早漏掉一处。
#[derive(Default)]
pub(super) struct LaneEntry {
    /// lane 名（连接名）。
    pub label: String,
    /// 是否当前 lane。
    pub active: bool,
    /// 这条 lane 此刻在做什么。
    pub status: LaneStatus,
    /// 后台 lane 产生了新输出、用户还没切过去看（切过去即清零）。
    pub unread: bool,
    /// 上下文用量短文本（`62%` / `~1%`）；没有实测也没有基线时 `None`。
    pub context: Option<String>,
    /// 在途回合已耗时（秒）；空闲时 `None`。
    pub busy_secs: Option<u64>,
    /// 生成中「呼吸」的相位档（见 [`pulse`]）。非生成中为 `None`。
    ///
    /// 字形不随相位变化（仍是 `LaneStatus::Running` 的 `●`）：宽度恒定，既不会挤动
    /// 折行点，也不必为一个动画多占一列。
    pub pulse: Option<u64>,
}

/// 一条 lane 当前处于什么阶段（渲染用的最小快照）。
///
/// 用枚举而不是「busy / 待审批」两个 bool：后者允许「空闲又在等审批」这种非法组合，
/// 分支也会随标志数扩散。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LaneStatus {
    /// 没有在跑的回合。
    #[default]
    Idle,
    /// 回合进行中（审批之外的工作）。
    Running,
    /// 有审批浮层在等决策。
    ///
    /// **后台 lane 的这个状态画不到屏幕上**（每帧只画当前 lane 的 `draw_frame`），
    /// 所以左栏这一列是用户唯一能看见它的地方 —— 否则那条 lane 会静默停摆到
    /// `APPROVAL_WAIT`（600 s）超时。见 issue #337。
    WaitingApproval,
    /// 已请求退出（`/quit` 在等这一轮结束，或 `exit` 已置位）。
    ///
    /// 与 [`Self::WaitingApproval`] 同一类问题：后台 lane 的那一行是**唯一**能看见
    /// 「这个客户端马上要走」的地方。没有它，用户在别的 lane 干活时会被一个不打招呼的
    /// 退出打断一遍 —— `/quit` 之后到真正退出之间可能隔着整轮生成（issue #338）。
    Quitting,
}

impl LaneStatus {
    /// 单列状态符号。
    ///
    /// 刻意都取**宽度确定是 1** 的字符：`⏸` 这类符号在部分终端算双列，会把整行撑破
    /// （`every_row_fills_the_width` 会当场发现）；而无色模式下符号本身就是状态的
    /// 唯一信息，不能省。
    fn mark(self) -> &'static str {
        match self {
            Self::Idle => "·",
            Self::Running => "●",
            Self::WaitingApproval => "▲",
            Self::Quitting => "×",
        }
    }
}

/// 当前 lane 的名字 —— **只在多 lane 时**给出；单 lane 返回 `None`。
///
/// 用途是「这是哪条连接」那一行（`/editor` 视图的第二行标题）：单 lane 时这个名字恒等于
/// 「当前这条」，写上去只占宽度；多 lane 时缺了它，同一个相对路径在两份文件里就看不出是哪份
/// （每条 lane 各绑各的 workspace）。
pub(super) fn current_label(lanes: &[LaneEntry]) -> Option<&str> {
    if lanes.len() < 2 {
        return None;
    }
    lanes
        .iter()
        .find(|l| l.active)
        .map(|l| l.label.as_str())
        .filter(|l| !l.is_empty())
}

/// 画出整块 lane 区：标题行 + 每条 lane 一行底色条，行与行之间不留空行。
///
/// 色条左右各内缩 1 列（与左右侧栏的内容区同口径）；条数少于可用行时下方留空
/// （不拉伸），多于可用行时按行截断并在标题补 `↓n`。待就绪的 lane 排在真连接之后。
pub(super) fn render_lane_list(
    frame: &mut Frame,
    area: Rect,
    lanes: &[LaneEntry],
    pending: &[super::pending_lane::PendingLane],
    palette: &Palette,
) {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = lane_list_rows(lanes, pending, inner.width as usize, inner.height, palette);
    frame.render_widget(Paragraph::new(rows), inner);
}

/// lane 区内容行：首行标题，其后每条 lane 一行。
///
/// `lanes` 是外壳层算好的快照 —— lane 的状态各自住在自己的 `LaneApp` 里，
/// 渲染不该为了画一行去借遍所有 lane（那会把借用冲突引到渲染路径上）。
fn lane_list_rows(
    lanes: &[LaneEntry],
    pending: &[super::pending_lane::PendingLane],
    width: usize,
    height: u16,
    palette: &Palette,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    // 标题固定 1 行，其余每行一条 lane（与 `layout::lane_index_at` 同一口径）。
    let avail = height.saturating_sub(1);
    let shown = lane_shown_len(avail, lanes.len());
    let mut rows = Vec::with_capacity(height as usize);
    rows.push(Line::from(Span::styled(
        truncate_display(&lane_title(LaneTally::from_entries(lanes), shown), width),
        Style::new().fg(palette.fg).add_modifier(Modifier::BOLD),
    )));
    for entry in lanes.iter().take(shown) {
        rows.push(lane_row(entry, width, palette));
    }
    // 待就绪行**排在真连接之后**：`layout::lane_index_at` 只把前 `lanes.len()` 行映射成
    // 下标，于是它们点不到 —— 点一行却切到某条真 lane，比点不动糟得多。
    let room = avail.saturating_sub(shown as u16) as usize;
    for item in pending.iter().take(room) {
        rows.push(pending_row(item, width, palette));
    }
    rows
}

/// 标题：`连接 [n/8] ▲k`，装不下时补 `↓m`（被截掉的条数）。
///
/// 容量（`/MAX_LANES`）与全局待审批数（`▲k`）原先长在状态栈的 `lanes` 段里（右下角）。
/// 那一行已撤掉：这两项的**操作对象**都在左栏这一块（`/lane attach` 加不了就超了上限、
/// `F4` 跳的正是 `▲` 那条），留在右下角只是同一件事的第二份读数。
///
/// `↓m` 与 `▲k` 并存：前者说「这里没画全」，后者说「有条目在等人」，两件事互不替代。
fn lane_title(tally: LaneTally, shown: usize) -> String {
    let mut text = format!("连接 [{}/{}]", tally.connected, super::shell::MAX_LANES);
    if shown < tally.connected {
        text.push_str(&format!(" ↓{}", tally.connected - shown));
    }
    if tally.pending_approvals > 0 {
        text.push_str(&format!(" ▲{}", tally.pending_approvals));
    }
    text
}

/// 一条 lane 的一行：整行宽的底色条，文字贴左。
///
/// 恒一行而不是「均分可用行」的色块：两行块在真终端上读起来像一个按钮，文字贴首行
/// 之后第二行只剩一条没有信息的色带，还会把切换 lane 的文字基线撑得跟上面的会话
/// 列表对不齐。一行一条与左侧会话列表同一行距，才读得出是「列表项」。
///
/// 整行补空格到 `width` —— 只给文字上底色的话，条会缩成「文字那么宽」，看起来仍是
/// 一行高亮文字，而不是列表里的一行。
///
/// 头两列是「当前 lane 标记 + 状态符号」：`▎` 指明聚焦，符号（`·` / `●` / `▲` / `×`）
/// 说明这条 lane 在做什么。两列都要 —— 后台 lane 在等审批时屏幕上看不到浮层，
/// 「切过去」与「它卡住了」全靠这一列（见 issue #337）。
///
/// 生成中**不换字形**：`●` 只做亮度呼吸（见 [`pulse`]）。行宽因此恒定，也不必为
/// 「在动」多留一列；后台 lane 在不在动，仍然不必切过去才看得见。
///
/// 只画名字，不带工作区末段：这一行首先回答「哪条 lane」，而末段会把名字挤掉
/// （工作区在右栏目录树的标题里，`/lane` 也列出全部）。
fn lane_row(entry: &LaneEntry, width: usize, palette: &Palette) -> Line<'static> {
    let base = row_style(entry, palette);
    let cursor = if entry.active { "▎" } else { " " };
    let mark = entry.status.mark();
    // 只有相位存在时才换色：`pulse` 由 `lane_entries` 决定（在跑 + 没关动效）。
    // 换色只染**状态符号**：`▎` 是「当前聚焦」的静态标记，让它跟着亮暗会读成整行在跳，
    // 而那一列本来也不表示「在动」（它只回答「我在哪条 lane」）。
    let mark_style = entry
        .pulse
        .map_or(base, |step| base.fg(pulse::color_at(palette.accent, step)));
    // 「标记 + 符号」恒 2 列（两者都被测试钉死单列）；宽度不足时先截它自己，
    // 免得余下那段算出负预算。
    let head = truncate_display(&format!("{cursor}{mark}"), width);
    let head_w = UnicodeWidthStr::width(head.as_str());
    let rest_w = width.saturating_sub(head_w);
    // 尾部读数最多占半行：这一行首先是「哪条 lane」，其次才是读数。
    let tail_room = width
        .saturating_sub(head_w + 1 + LANE_LABEL_MIN)
        .min(width / 2);
    let tail = lane_tail(entry, tail_room);
    let tail_w = UnicodeWidthStr::width(tail.as_str());
    // 名字 + 尾部读数占满余下宽度；装不下时 `truncate_display` 从右切（名字优先）。
    let mut rest = format!(" {}", entry.label);
    if !tail.is_empty() {
        let used = UnicodeWidthStr::width(rest.as_str());
        rest.push_str(&" ".repeat(rest_w.saturating_sub(used + tail_w)));
        rest.push_str(&tail);
    }
    let rest = pad_display(&truncate_display(&rest, rest_w), rest_w);
    // 三段 span：`▎`（静态）+ 状态符号（呼吸）+ 余下整行（底色条）。
    // 拆开是为了让呼吸只落在符号那一列 —— 见上面 `mark_style` 的注释。
    let mut head_chars = head.chars();
    let cursor_cell = head_chars.next().map(String::from).unwrap_or_default();
    let mark_cell: String = head_chars.collect();
    Line::from(vec![
        Span::styled(cursor_cell, base),
        Span::styled(mark_cell, mark_style),
        Span::styled(rest, base),
    ])
}

/// 一条待就绪 lane 的一行：`◌ name`（启动中 / 超时）或 `✗ name`（起不来）。
///
/// 与 [`lane_row`] 一样整行补空格到 `width` —— 只给文字上底色的话，条会缩成「文字那么
/// 宽」，看起来像一行普通文字而不是列表项。但它**没有** `▎` 与读数：这条还不在
/// `lanes` 里，既不能聚焦，也没有上下文 / 耗时 / 未读点。
///
/// 失败用错误色：那是一条需要用户动手的信号。启动中 / 超时用淡色 —— 它自己会变，
/// 不该抢真连接的注意力。
fn pending_row(
    item: &super::pending_lane::PendingLane,
    width: usize,
    palette: &Palette,
) -> Line<'static> {
    use super::pending_lane::PendingState;

    let base = Style::new().fg(palette.muted).bg(palette.overlay_bg);
    let head_style = match item.state {
        PendingState::Failed(_) => base.fg(palette.err),
        PendingState::Starting | PendingState::TimedOut => base,
    };
    let head = item.state.mark();
    let head_w = UnicodeWidthStr::width(head);
    let rest_w = width.saturating_sub(head_w);
    let text = pending_text(item, rest_w);
    Line::from(vec![
        Span::styled(head, head_style),
        Span::styled(pad_display(&truncate_display(&text, rest_w), rest_w), base),
    ])
}

/// `名字` + 右对齐尾注（`超时` / 失败原文）；装不下时从右截断，名字优先。
fn pending_text(item: &super::pending_lane::PendingLane, width: usize) -> String {
    let mut text = format!(" {}", item.name);
    let note = item.state.note();
    if note.is_empty() {
        return text;
    }
    // 尾注最多占半行：这一行的主语是名字。
    let used = UnicodeWidthStr::width(text.as_str());
    let room = width.saturating_sub(used + 1).min(width / 2);
    if room == 0 {
        return text;
    }
    let note = truncate_display(note, room);
    let gap = width.saturating_sub(used + UnicodeWidthStr::width(note.as_str()));
    text.push_str(&" ".repeat(gap));
    text.push_str(&note);
    text
}

/// 行尾读数：`62% 12s •`，按重要性**从右往左**排 —— 最右的未读点最重要（它就是
/// 「切过去看看」的理由），其次是耗时，再次是上下文占比。
///
/// 宽度不够时从**左边**丢：先丢 `ctx`，再丢耗时，未读点留到最后。宁可少一个数字，
/// 也不能把「这条 lane 有话要说」丢掉。
fn lane_tail(entry: &LaneEntry, room: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ctx) = entry.context.as_deref() {
        parts.push(ctx.to_string());
    }
    if let Some(secs) = entry.busy_secs {
        parts.push(duration_text(secs));
    }
    if entry.unread {
        parts.push("•".to_string());
    }
    while !parts.is_empty() && tail_width(&parts) > room {
        parts.remove(0);
    }
    parts.join(" ")
}

/// 尾部文本占的列数（含分隔空格）。
fn tail_width(parts: &[String]) -> usize {
    if parts.is_empty() {
        return 0;
    }
    let glyphs: usize = parts
        .iter()
        .map(|p| UnicodeWidthStr::width(p.as_str()))
        .sum();
    glyphs + parts.len() - 1
}

/// 回合耗时短文本：`45s` / `2m05s` / `1h02m`。
///
/// 定宽优先于精确：60 秒后跳到 `m` 而不是继续累秒，否则 3 分钟就是四位数宽度，
/// 在这一行里会把名字挤掉。
fn duration_text(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// 底色条样式：当前 lane 走 `palette.selection`（无色模式自动退回反色），其余用
/// 「比条带底色亮一档」的 `overlay_bg`，这样条与 lane 区底色仍分得开。
///
/// 等审批的那条非当前 lane 用告警色前景：它不会自己前进，用户得知道该切过去
/// （无色模式下颜色退回 `Reset`，仍有 `▲` 这个符号兜底）。
fn row_style(entry: &LaneEntry, palette: &Palette) -> Style {
    if entry.active {
        return palette
            .selection(palette.accent)
            .add_modifier(Modifier::BOLD);
    }
    match entry.status {
        // 退出请求和等审批一样「不会自己前进」，都用告警色；靠符号与 BOLD 区分两者。
        LaneStatus::Quitting => Style::new()
            .fg(palette.err)
            .bg(palette.overlay_bg)
            .add_modifier(Modifier::BOLD),
        LaneStatus::WaitingApproval => Style::new().fg(palette.err).bg(palette.overlay_bg),
        LaneStatus::Idle | LaneStatus::Running => {
            Style::new().fg(palette.fg).bg(palette.overlay_bg)
        }
    }
}

/// 补空格到 `width` 个显示列（CJK 按 2 列计）；已够宽则原样返回。
fn pad_display(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    let mut out = String::with_capacity(text.len() + width.saturating_sub(used));
    out.push_str(text);
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

/// 有名字与状态点了，还缺两组计数：连了几条（`连接 [2/8]`）、几条在等审批（`▲k`）。
///
/// 合成一个值而不是两个字段，是为了让「两处不得各算一份」成为类型事实：左栏标题拿到
/// 这一个值，就没机会自己再数一遍 `lanes.len()`。
///
/// 这两项原本由状态栈的 `lanes` 段显示（右下角），现已并入左栏标题 —— 同一条信息在
/// 两个角落各画一遍，久了必然对不上，而左栏才是「加一条 / 跳过去」的操作位置。
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LaneTally {
    /// 已连接数。与左栏标题 `连接 [n/8]` 是**同一个数字**。
    pub connected: usize,
    /// 正在等审批的 lane 数（0 = 标题不显示 `▲` 后缀）。
    ///
    /// 判定复用 [`lane_status`]，也就是左栏 `▲` 与 `F4` 的 `next_approval_index` 用的那一个：
    /// 各判一遍的症状是「左栏亮着 `▲`、标题说没有人在等」。
    pub pending_approvals: usize,
}

impl LaneEntry {
    /// 这条 lane 现在在等审批（左栏 `▲`、`F4` 跳转与标题的 `▲k` **共用**这一判定）。
    ///
    /// `status` 本来就是 [`lane_status`] 的结果，这里只是给它一个名字：读 `▲` 的那几处
    /// 不必各自再写一遍 `== LaneStatus::WaitingApproval`。
    pub(super) fn is_waiting_approval(&self) -> bool {
        self.status == LaneStatus::WaitingApproval
    }
}

impl LaneTally {
    /// 按外壳交给渲染的那份 lane 快照数（`draw_frame` 的 `lanes` 参数就是它）。
    ///
    /// 用快照而不是再读一遍 `TuiApp::lanes`：左栏画的就是这批 entry，两处读不同来源迟早
    /// 出现「标题说两条、下面画了三条」。
    pub(super) fn from_entries(entries: &[LaneEntry]) -> Self {
        Self {
            connected: entries.len(),
            pending_approvals: entries.iter().filter(|e| e.is_waiting_approval()).count(),
        }
    }
}

/// 外壳侧的投影：把每条 lane 的读数压成渲染需要的字段。
///
/// 放在这个文件而不是 `shell.rs` 有两个理由：它的唯一用途就是「给 lane 行提供数据」；
/// 而 `shell.rs` 贴着 920 行门禁，读数只会越加越多。
///
/// 仍是**投影**而不是暴露 lane：这里读出 `UiState` 的几个数字就地拼成文本，渲染层
/// 拿到的依然是 `String` / `u64`，改不到任何 lane 的状态。
impl super::TuiApp<'_> {
    pub(super) fn lane_entries(&self) -> Vec<LaneEntry> {
        // 动效关掉时整块都不动：与状态行仪器条同一约束（录屏 / 远程终端 / 读屏）。
        let breathe = !reduced_motion();
        self.lanes
            .iter()
            .enumerate()
            .map(|(i, lane)| {
                let status = lane_status(lane);
                let persisted = lane.persisted.as_ref();
                // 耗时与呼吸相位都从同一个起点算：从**发出**起（见 `turn_elapsed`）。
                let elapsed = lane.turn_elapsed();
                LaneEntry {
                    label: lane.label.clone(),
                    active: i == self.active,
                    status,
                    unread: lane.unread,
                    // 与状态行 `ctx` 段同一条轴线（见 `UiState::context_pct_text`），
                    // 只是省成占比。
                    context: lane.st.context_pct_text(
                        persisted.and_then(|p| p.context_tokens.as_deref()),
                        lane.st.serve_defaults.as_ref().and_then(|d| {
                            d.baseline_prompt_tokens(persisted.and_then(|p| p.role.as_deref()))
                        }),
                    ),
                    busy_secs: lane.turn_started.map(|_| elapsed.as_secs()),
                    // 相位来自墙钟：后台 lane 不必被谁推进，切过去之前它也在呼吸。
                    pulse: (breathe && spinner_turning(lane)).then(|| pulse::step_at(elapsed)),
                }
            })
            .collect()
    }

    /// 非当前 lane 收到新输出时点亮未读点。
    ///
    /// 只认 [`UiEvent::Text`] 与 [`UiEvent::TurnDone`]：工具行的起止属于「过程」，
    /// 要人回来看的是「它说了什么」与「它答完了」。
    pub(super) fn mark_background_unread(&mut self, from: LaneId, ev: &UiEvent) {
        if !matches!(ev, UiEvent::Text(_) | UiEvent::TurnDone { .. }) {
            return;
        }
        let Some(idx) = self.lanes.iter().position(|l| l.id == from) else {
            return;
        };
        if idx != self.active {
            self.lanes[idx].unread = true;
        }
    }

    /// 当前 lane 已经摆在眼前，未读点该熄了。每帧调用。
    ///
    /// 不逐条写在切换路径上（`Alt+←/→`、点击、`F4`、`/lane attach` 各有出口）：漏一处的
    /// 表现是「切过去了那个点亮着不走」，而每帧改一个 bool 的代价是零。
    pub(super) fn clear_active_unread(&mut self) {
        if let Some(lane) = self.lanes.get_mut(self.active) {
            lane.unread = false;
        }
    }
}

/// lane 的渲染状态：**退出请求 > 审批 > 「运行中」** —— 等审批的 lane 一定也在跑，
/// 而用户必须看见的是「它在等人」，不是「它在干活」；已经 `/quit` 的那条则连审批都
/// 不该再抢注意力，整个客户端马上要走。
fn lane_status(lane: &super::LaneApp<'_>) -> LaneStatus {
    if lane.exit || lane.quit_pending {
        LaneStatus::Quitting
    } else if lane.st.approval.is_some() {
        LaneStatus::WaitingApproval
    } else if lane.st.running {
        LaneStatus::Running
    } else {
        LaneStatus::Idle
    }
}

/// 这条 lane 的「在跑」指示此刻该不该动。
///
/// 两个条件缺一不可：**渲染态是「运行中」**（等审批要画 `▲`、已请求退出画 `×`，
/// 都不该被呼吸顶掉），以及**没有在取消**（取消中冻结，与状态行的 `cancel_sent`
/// 口径一致）。
///
/// 唤醒源（`TuiApp::next_spinner_tick`）与左栏绘制共用这一个判定：两边各写一套
/// 就会出现「一处算在动、另一处不算」，后台 lane 于是停在原地 —— 那正是本次要修
/// 的症状，别在这里分叉。
pub(super) fn spinner_turning(lane: &super::LaneApp<'_>) -> bool {
    matches!(lane_status(lane), LaneStatus::Running) && !lane.st.cancel_sent
}

// 左栏 lane 区的单测单独成文件：本文件贴着 920 行/文件门禁，而这一块只读快照、不画终端
// （`#[path]` 只换文件，模块路径仍是 `lane_list::tests`）。
#[cfg(test)]
#[path = "lane_list_tests.rs"]
mod tests;
