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
use super::spinner;
use super::status::SPINNER_FRAMES;
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
    /// 该 lane 的 serve 当前工作区根。`None` = 还没取到（或待绑定的空根）。
    pub workspace: Option<String>,
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
    /// 生成中 spinner 的当前帧（`⠋`…）。非生成中为 `None`，此时用 [`LaneStatus::mark`]。
    pub spin: Option<&'static str>,
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

/// 画出整块 lane 区：标题行 + 每条 lane 一行底色条，行与行之间不留空行。
///
/// 色条左右各内缩 1 列（与左右侧栏的内容区同口径）；条数少于可用行时下方留空
/// （不拉伸），多于可用行时按行截断并在标题补 `↓n`。
pub(super) fn render_lane_list(
    frame: &mut Frame,
    area: Rect,
    lanes: &[LaneEntry],
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
    let rows = lane_list_rows(lanes, inner.width as usize, inner.height, palette);
    frame.render_widget(Paragraph::new(rows), inner);
}

/// lane 区内容行：首行标题，其后每条 lane 一行。
///
/// `lanes` 是外壳层算好的快照 —— lane 的状态各自住在自己的 `LaneApp` 里，
/// 渲染不该为了画一行去借遍所有 lane（那会把借用冲突引到渲染路径上）。
fn lane_list_rows(
    lanes: &[LaneEntry],
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
        truncate_display(&lane_title(lanes.len(), shown), width),
        Style::new().fg(palette.fg).add_modifier(Modifier::BOLD),
    )));
    for entry in lanes.iter().take(shown) {
        rows.push(lane_row(entry, width, palette));
    }
    rows
}

/// 标题：`连接 [总数]`，装不下时补 `↓n`（被截掉的条数）。
fn lane_title(total: usize, shown: usize) -> String {
    if shown < total {
        format!("连接 [{total}] ↓{}", total - shown)
    } else {
        format!("连接 [{total}]")
    }
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
/// 头两列是「当前 lane 标记 + 状态符号」：`▎` 指明聚焦，符号（`·` / `●` / `▲`）说明
/// 这条 lane 在做什么。两列都要 —— 后台 lane 在等审批时屏幕上看不到浮层，
/// 「切过去」与「它卡住了」全靠这一列（见 issue #337）。
fn lane_row(entry: &LaneEntry, width: usize, palette: &Palette) -> Line<'static> {
    let cursor = if entry.active { "▎" } else { " " };
    // 生成中用 spinner 帧而不是 `●`：一条 lane 在不在动，不必先切过去看右栏才
    // 知道 —— 这正是「多路并行」最该被看见的那一个事实。
    let mark = match (entry.status, entry.spin) {
        (LaneStatus::Running, Some(frame)) => frame,
        (status, _) => status.mark(),
    };
    // 尾部读数最多占半行：这一行首先是「哪条 lane」，其次才是读数。
    let tail_room = width.saturating_sub(3 + LANE_LABEL_MIN).min(width / 2);
    let tail = lane_tail(entry, tail_room);
    let tail_w = UnicodeWidthStr::width(tail.as_str());
    let gap = usize::from(!tail.is_empty());
    // `mark + 空格` 占 3 列，剩下的才是名字与工作区名能用的宽度。
    let avail = width.saturating_sub(3 + tail_w + gap);
    let body = lane_row_text(&entry.label, entry.workspace.as_deref(), avail);
    let mut text = format!("{cursor}{mark} {body}");
    if !tail.is_empty() {
        let used = UnicodeWidthStr::width(text.as_str());
        text.push_str(&" ".repeat(width.saturating_sub(used + tail_w)));
        text.push_str(&tail);
    }
    let text = pad_display(&truncate_display(&text, width), width);
    Line::from(Span::styled(text, row_style(entry, palette)))
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

/// 行内文字：`名字`，宽度够时补 `· 工作区末段`。
///
/// 工作区只显示**末段**（完整路径在右栏标题里）：lane 行是一行一刀的窄条，整条路径
/// 会把名字挤没。剩余宽度不够 4 列时宁可不显示 —— 挤出一个字母既没有信息量，也会让
/// 「两条 lane 是不是同一个工作区」这件事看不出来。
fn lane_row_text(label: &str, workspace: Option<&str>, avail: usize) -> String {
    let Some(ws) = workspace.and_then(workspace_hint) else {
        return label.to_string();
    };
    let used = UnicodeWidthStr::width(label) + 3; // " · "
    if avail < used + 4 {
        return label.to_string();
    }
    format!("{label} · {ws}")
}

/// 工作区根 → 行内末段名。空路径（待绑定 / 尚未取到）返回 `None`。
///
/// 同时接受 `/` 与 `\\` 分隔：serve 可能跑在 Windows 上，这条只是显示，不该因此消失。
fn workspace_hint(path: &str) -> Option<&str> {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    trimmed.rsplit(['/', '\\']).next().filter(|s| !s.is_empty())
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

/// 有名字与状态点了，还缺两组计数：连了几条（`lanes 2/8`）、几条在等审批。
///
/// 合成一个值而不是两个字段，是为了让「两处不得各算一份」成为类型事实：状态栏拿到
/// 这一个值，就没机会自己再数一遍 `lanes.len()`。
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LaneTally {
    /// 已连接数。与左栏标题 `连接 [n]` 是**同一个数字**。
    pub connected: usize,
    /// 正在等审批的 lane 数（0 = 状态栏不显示该段）。
    ///
    /// 判定复用 [`lane_status`]，也就是左栏 `▲` 与 `F4` 的 `next_approval_index` 用的那一个：
    /// 各判一遍的症状是「左栏亮着 `▲`、状态栏说没有人在等」。
    pub pending_approvals: usize,
}

impl LaneEntry {
    /// 这条 lane 现在在等审批（左栏 `▲`、`F4` 跳转与状态栏计数**共用**这一判定）。
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
    /// 出现「左栏两条、状态栏三条」。
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
        self.lanes
            .iter()
            .enumerate()
            .map(|(i, lane)| {
                let status = lane_status(lane);
                let persisted = lane.persisted.as_ref();
                // 耗时与 spinner 帧都从同一个起点算：从**发出**起（见 `turn_elapsed`）。
                let elapsed = lane.turn_elapsed();
                LaneEntry {
                    label: lane.label.clone(),
                    workspace: lane.st.workspace_path.clone(),
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
                    // 帧号来自墙钟：后台 lane 不必被谁推进，切过去之前它也在动。
                    spin: spinner_turning(lane)
                        .then(|| SPINNER_FRAMES[spinner::frame_at(elapsed) % SPINNER_FRAMES.len()]),
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

/// 这条 lane 的 spinner 此刻该不该转。
///
/// 两个条件缺一不可：**渲染态是「运行中」**（等审批要画 `▲`、已请求退出画 `×`，
/// 都不该被帧顶掉），以及**没有在取消**（取消中冻结，与状态行的 `cancel_sent`
/// 口径一致）。
///
/// 唤醒源（`TuiApp::next_spinner_tick`）与左栏绘制共用这一个判定：两边各写一套
/// 就会出现「一处算在转、另一处不算」，后台 lane 于是停在原地 —— 那正是本次要修
/// 的症状，别在这里分叉。
pub(super) fn spinner_turning(lane: &super::LaneApp<'_>) -> bool {
    matches!(lane_status(lane), LaneStatus::Running) && !lane.st.cancel_sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::inbound::Inbound;
    use crate::tui_mode::test_support::{test_client, test_prefs, test_shell_with_jobs};

    fn pal() -> Palette {
        Palette::default()
    }

    /// 只有名字、没有工作区、第一条是当前 lane —— 大多数测试只关心这个。
    fn lanes(names: &[&str]) -> Vec<LaneEntry> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| LaneEntry {
                label: (*n).to_string(),
                active: i == 0,
                ..LaneEntry::default()
            })
            .collect()
    }

    /// 带工作区的单条 lane（用来钉「工作区名怎么进这一行」）。
    fn lane_with(label: &str, workspace: Option<&str>, active: bool) -> LaneEntry {
        LaneEntry {
            label: label.to_string(),
            workspace: workspace.map(str::to_string),
            active,
            ..LaneEntry::default()
        }
    }

    /// 单 lane：标题 1 行 + 1 行底色条，其余行留空（版面高度固定，不随条数拉伸）。
    #[test]
    fn one_lane_takes_one_row_under_the_title() {
        let rows = lane_list_rows(&lanes(&["local"]), 20, 20, &pal());
        // 1（标题）+ 1（那条 lane）= 2 行有内容，其余交给调用方的底色。
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [1]");
        // 唯一那条就是当前 lane，故整行都是选中底色。
        assert_eq!(rows[1].spans[0].style.bg, Some(pal().selection_bg));
        assert!(rows[1].spans[0].content.contains("local"));
    }

    /// 当前 lane 用选中底色 + 竖条标记；其它 lane 用一档抬升底色，每条各占一行。
    #[test]
    fn active_lane_is_marked_and_tinted() {
        let rows = lane_list_rows(&lanes(&["a", "b"]), 20, 20, &pal());
        // 一条一行：a = rows[1]，b = rows[2]。
        let a = &rows[1];
        let b = &rows[2];
        assert!(a.spans[0].content.starts_with('▎'));
        assert_eq!(a.spans[0].style.bg, Some(pal().selection_bg));
        assert!(!b.spans[0].content.starts_with('▎'));
        assert_eq!(b.spans[0].style.bg, Some(pal().overlay_bg));
    }

    /// 每行宽度恒等于内容区宽度：色块必须是矩形，不是「文字那么宽」的底色。
    #[test]
    fn every_row_fills_the_width() {
        let rows = lane_list_rows(&lanes(&["local", "很长很长的连接名字"]), 16, 20, &pal());
        for r in &rows {
            let w: usize = r
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert!(w <= 16, "行宽 {w} 超出 16 列");
        }
        // 色块行必须**正好**铺满（不足部分由 pad_display 补空格）。
        for r in &rows[1..] {
            let w: usize = r
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            assert_eq!(w, 16);
        }
    }

    /// 装不下时标题行给出被截掉的条数，而不是静默丢弃。
    #[test]
    fn overflow_is_reported_in_the_title() {
        let names: Vec<String> = (0..30).map(|i| format!("lane{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let rows = lane_list_rows(&lanes(&refs), 20, 10, &pal());
        // 可用 9 行、块高 1 → 画 9 条，标题报 ↓21。
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [30] ↓21");
        assert_eq!(rows.len(), 10);
    }

    /// 只有标题行放得下时（高度 1）不 panic，且标题如实报出「全部被截掉」。
    #[test]
    fn one_row_only_shows_the_title() {
        let rows = lane_list_rows(&lanes(&["a", "b"]), 20, 1, &pal());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans[0].content.as_ref(), "连接 [2] ↓2");
    }

    #[test]
    fn zero_height_or_width_draws_nothing() {
        assert!(lane_list_rows(&lanes(&["a"]), 0, 10, &pal()).is_empty());
        assert!(lane_list_rows(&lanes(&["a"]), 10, 0, &pal()).is_empty());
    }

    /// 一实例一工作区：lane 行必须能看出「这两条连的是不同项目」。
    #[test]
    fn lane_row_shows_the_workspace_tail() {
        let entries = vec![
            lane_with("lane-a", Some("/data/work/proj-a"), true),
            lane_with("lane-b", Some("/data/work/proj-b"), false),
        ];
        let rows = lane_list_rows(&entries, 40, 20, &pal());
        let a = rows[1].spans[0].content.as_ref();
        let b = rows[2].spans[0].content.as_ref();
        assert!(a.contains("proj-a"), "应显示工作区末段：{a}");
        assert!(b.contains("proj-b"), "{b}");
        // 只显示末段：整条路径在这条窄条里会把名字挤没。
        assert!(!a.contains("/data/work"), "不该显示整条路径：{a}");
    }

    /// 没取到工作区（或待绑定的空根）时，行里只有名字 —— 不能留一个空的分隔符。
    #[test]
    fn missing_workspace_leaves_the_label_alone() {
        for entries in [
            lanes(&["local"]),
            vec![lane_with("local", Some(""), true)],
            vec![lane_with("local", Some("   "), true)],
        ] {
            let rows = lane_list_rows(&entries, 20, 20, &pal());
            assert_eq!(rows[1].spans[0].content.trim(), "▎· local");
        }
    }

    /// 宽度不够时宁可不显示工作区，也不要挤出一个字母。
    #[test]
    fn narrow_row_drops_the_workspace_hint() {
        let entries = vec![lane_with("lane-a", Some("/data/work/proj-a"), true)];
        // 14 列：`▎ ` + `lane-a` + ` · ` 之后只剩不到 4 列给名字。
        let rows = lane_list_rows(&entries, 14, 20, &pal());
        assert!(
            !rows[1].spans[0].content.contains("proj-a"),
            "太窄时不该硬塞：{}",
            rows[1].spans[0].content
        );
    }

    /// 状态符号必须各不相同、且**都是单列**：双列符号会把整行撑破。
    #[test]
    fn status_marks_are_distinct_and_single_column() {
        for status in [
            LaneStatus::Idle,
            LaneStatus::Running,
            LaneStatus::WaitingApproval,
            LaneStatus::Quitting,
        ] {
            let mark = status.mark();
            assert_eq!(UnicodeWidthStr::width(mark), 1, "`{mark}` 必须是单列");
        }
        let mut marks = vec![
            LaneStatus::Idle.mark(),
            LaneStatus::Running.mark(),
            LaneStatus::WaitingApproval.mark(),
            LaneStatus::Quitting.mark(),
        ];
        marks.sort_unstable();
        marks.dedup();
        assert_eq!(
            marks.len(),
            4,
            "四种状态不能共用符号：否则分不出「在跑」「等审批」与「正在退出」"
        );
    }

    /// 等审批的**后台** lane 必须一眼能看见：符号 + 告警色，两样都要。
    ///
    /// 后台 lane 的浮层画不到屏幕上，这一行是唯一线索（issue #337）。
    #[test]
    fn waiting_approval_is_marked_and_tinted() {
        let mut entries = lanes(&["lane-a", "lane-b"]);
        entries[1].status = LaneStatus::WaitingApproval;
        let rows = lane_list_rows(&entries, 30, 20, &pal());
        let b = rows[2].spans[0].content.as_ref();
        assert!(b.contains('▲'), "等审批要有状态符号：{b}");
        assert_eq!(
            rows[2].spans[0].style.fg,
            Some(pal().err),
            "非当前 lane 的等审批走告警色"
        );
    }

    /// 状态栏的两组计数与左栏同源：`connected` 是 entries 条数，`pending_approvals`
    /// 只数 `▲` 那一种状态 —— 两处判定不同就会「左栏亮 `▲`、状态栏说 0」。
    #[test]
    fn tally_counts_entries_and_waiting_lanes() {
        let mut entries = lanes(&["lane-a", "lane-b", "lane-c"]);
        entries[1].status = LaneStatus::WaitingApproval;
        entries[2].status = LaneStatus::WaitingApproval;
        let tally = LaneTally::from_entries(&entries);
        assert_eq!(tally.connected, 3);
        assert_eq!(tally.pending_approvals, 2);
        // 没人等审批时是 0（状态栏据此**不画** ▲ 后缀）。
        assert_eq!(LaneTally::from_entries(&entries[..1]).pending_approvals, 0);
    }

    /// 当前 lane 的 `▎` 在任何状态下都要保留：否则「哪条是我正在看的」就没了。
    #[test]
    fn active_lane_keeps_its_cursor_mark_in_every_status() {
        for status in [
            LaneStatus::Idle,
            LaneStatus::Running,
            LaneStatus::WaitingApproval,
        ] {
            let mut entries = lanes(&["lane-a"]);
            entries[0].status = status;
            let rows = lane_list_rows(&entries, 30, 20, &pal());
            assert!(
                rows[1].spans[0].content.starts_with('▎'),
                "{status:?} 时仍要标出当前 lane"
            );
        }
    }

    /// 尾部读数：上下文占比、耗时、未读点同时出现，未读点在最右。
    #[test]
    fn tail_shows_context_busy_and_unread_right_aligned() {
        let mut entries = lanes(&["lane-a"]);
        entries[0].context = Some("62%".to_string());
        entries[0].busy_secs = Some(12);
        entries[0].unread = true;
        let rows = lane_list_rows(&entries, 40, 20, &pal());
        let line = rows[1].spans[0].content.as_ref();
        assert!(line.ends_with('•'), "未读点必须在最右：{line}");
        assert!(line.contains("62%"), "{line}");
        assert!(line.contains("12s"), "{line}");
        // 读数不许把身份挤掉：这一行首先是「哪条 lane」。
        assert!(line.contains("lane-a"), "{line}");
        assert_eq!(UnicodeWidthStr::width(line), 40);
    }

    /// 窄行先丢 ctx、再丢耗时：留到最后的必须是未读点。
    #[test]
    fn tail_drops_readings_before_the_unread_dot() {
        let mut entries = lanes(&["lane-a"]);
        entries[0].context = Some("100%".to_string());
        entries[0].busy_secs = Some(125);
        entries[0].unread = true;
        // 16 列：尾部最多占 8 列，装不下三个读数。
        let rows = lane_list_rows(&entries, 16, 20, &pal());
        let line = rows[1].spans[0].content.as_ref();
        assert!(line.ends_with('•'), "{line}");
        assert!(!line.contains("100%"), "窄行先丢 ctx：{line}");
        assert_eq!(UnicodeWidthStr::width(line), 16);

        // 更窄（12 列）：耗时也让位，只剩那个点。
        let rows = lane_list_rows(&entries, 12, 20, &pal());
        let line = rows[1].spans[0].content.as_ref();
        assert!(line.ends_with('•'), "{line}");
        assert!(!line.contains("m0"), "耗时也要让位：{line}");
        assert_eq!(UnicodeWidthStr::width(line), 12);
    }

    /// 生成中用 spinner 帧顶替 `●`：后台 lane 在不在动，不切过去也看得见。
    #[test]
    fn running_lane_shows_the_spinner_frame() {
        let mut entries = lanes(&["lane-a"]);
        entries[0].status = LaneStatus::Running;
        entries[0].spin = Some("⠹");
        let rows = lane_list_rows(&entries, 30, 20, &pal());
        let line = rows[1].spans[0].content.as_ref();
        assert!(line.contains('⠹'), "{line}");
        assert!(!line.contains('●'), "生成中不再画静态度点：{line}");
        // 帧符号仍是单列，否则整行会被撑破。
        assert_eq!(UnicodeWidthStr::width(line), 30);
    }

    /// 后台 lane 的 spinner 帧跟着墙钟走：它不进 `draw_frame`，也不该因此冻结。
    #[test]
    fn background_lane_spinner_follows_the_clock() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        // 两条都在跑：当前 lane 刚发出，后台那条已经跑了 0.25s。
        app.lanes[0].st.running = true;
        app.lanes[0].turn_started = Some(std::time::Instant::now());
        let behind = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(250))
            .expect("系统时钟不支持回退 Instant");
        app.lanes[1].st.running = true;
        app.lanes[1].turn_started = Some(behind);

        let entries = app.lane_entries();
        let front = entries[0].spin.expect("当前 lane 在跑就该有帧");
        let back = entries[1].spin.expect("后台 lane 在跑就该有帧");
        assert_ne!(
            front, back,
            "后台 lane 的帧号必须跟着墙钟走，不能停在切走时那一格"
        );
    }

    /// 取消中不转：状态行已经在说「取消中」，lane 行再转就是在骗人。
    #[test]
    fn cancelling_lane_does_not_spin() {
        let client = test_client();
        let mut prefs = [test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        app.lanes[0].st.running = true;
        app.lanes[0].st.cancel_sent = true;
        app.lanes[0].turn_started = Some(std::time::Instant::now());
        assert!(
            app.lane_entries()[0].spin.is_none(),
            "取消中不该画推进中的帧"
        );
    }

    /// 耗时换成定宽单位：`s` → `m` → `h`，不让秒数无限长。
    #[test]
    fn busy_duration_switches_units_to_stay_narrow() {
        assert_eq!(duration_text(0), "0s");
        assert_eq!(duration_text(59), "59s");
        assert_eq!(duration_text(60), "1m00s");
        assert_eq!(duration_text(3599), "59m59s");
        assert_eq!(duration_text(3600), "1h00m");
        assert_eq!(duration_text(7325), "2h02m");
    }

    /// 后台 lane 的新输出点亮未读点，当前 lane 的不点亮；切过去即熄。
    #[test]
    fn background_output_marks_unread_but_the_active_lane_does_not() {
        let client = test_client();
        let mut prefs = [test_prefs(), test_prefs()];
        let (mut app, _jobs) = test_shell_with_jobs(&client, &mut prefs);
        let (first, second) = (app.lanes[0].id, app.lanes[1].id);

        app.on_msg(Inbound::FromLane(second, UiEvent::Text("hi".into())));
        assert!(!app.lane_entries()[0].unread, "当前 lane 不该亮未读");
        assert!(app.lane_entries()[1].unread, "后台 lane 有输出要亮");

        // 切过去即熄（帧循环每帧调用 `clear_active_unread`）。
        app.active = 1;
        app.clear_active_unread();
        assert!(!app.lane_entries()[1].unread);

        // 回到第一条：它自己说话不点亮自己，否则那个点永远亮着。
        app.active = 0;
        app.clear_active_unread();
        app.on_msg(Inbound::FromLane(first, UiEvent::Text("x".into())));
        assert!(!app.lane_entries()[0].unread);
    }

    #[test]
    fn workspace_hint_takes_the_tail_and_ignores_empty() {
        assert_eq!(workspace_hint("/data/work/proj-a"), Some("proj-a"));
        assert_eq!(workspace_hint("/data/work/proj-a/"), Some("proj-a"));
        assert_eq!(workspace_hint("proj"), Some("proj"));
        // serve 可能跑在 Windows 上：反斜杠也要认。
        assert_eq!(workspace_hint("C:\\work\\proj"), Some("proj"));
        assert_eq!(workspace_hint(""), None);
        assert_eq!(workspace_hint("   "), None);
        assert_eq!(workspace_hint("/"), None);
    }
}
