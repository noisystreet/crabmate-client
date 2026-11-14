//! 澄清问卷浮层（issue #424）：绘制、按键、提交。
//!
//! 与审批浮层（`overlay.rs` + `approve.rs`）**不是**一回事，别照抄那份：
//!
//! - 审批是流内阻塞门，`Sender<ApprovalDecision>` 一回 SSE 线程就继续；
//! - 问卷只是「记下待填题面」，用户答完后作为**下一条**请求的
//!   `clarify_questionnaire_answers` 发出去（发送走 `start_turn_with_clarify`，
//!   与普通回合共用 `build_turn_opts` / `dispatch_turn`，不另开第二条发送路径）。
//!
//! 独立成文件的两个理由：`overlay.rs` 已 645 行、`state.rs` 正贴着 920 行门禁；
//! 且这里是「多题编辑 + 必填校验」的状态机，与审批的「三选一」没有共用逻辑。

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::LaneApp;
use super::state::{ClarifyOverlay, LineKind, UiState};
use crate::serve::ClarificationQuestionnaireInfo;
use crabmate_client_api::sse_dispatch::{ClarificationFormField, ClarificationKind};

/// 浮层宽度上限：再宽正文行长就难扫读（与审批浮层同一档）。
const CLARIFY_MAX_WIDTH: u16 = 76;
/// 每题占的行数（标签行 + 输入行）。渲染窗口按它算能放几题。
const ROWS_PER_QUESTION: u16 = 2;
/// `intro` 最多占几行：题面预算必须与绘制一致，否则长 intro 会把最后一题挤出可视区。
const INTRO_MAX_ROWS: usize = 2;
/// 浮层边框占的行数（上下各 1）。
const BORDER_ROWS: u16 = 2;
/// 纵向留白：intro 之后、键位提示之前各一空行。
const BLANK_ROWS: u16 = 2;
/// 底部键位提示占的行数。
const HINT_ROWS: u16 = 1;
/// 收起后重开的键（F7 未被 F3 状态栈 / F4 下一个审批 / F6 设置占用）。
pub(super) const CLARIFY_REOPEN_KEY: &str = "F7";
/// 提交后代替用户消息的占位文本（与 Web 的 `clarification_user_bubble_stub` 同义）。
pub(super) const CLARIFY_SUBMIT_STUB: &str = "（已回答澄清问卷）";

impl LaneApp<'_> {
    /// 问卷到达：记状态 + 落一行提示，**先不弹**。
    ///
    /// Server 是在工具成功后补发问卷的，此刻工具结果还要回喂模型、本轮往往仍在流 ——
    /// 直接弹浮层会盖住正在写的 transcript。展开时机见 `open_clarify_after_turn`。
    pub(super) fn begin_clarify(&mut self, info: ClarificationQuestionnaireInfo) {
        let n = info.fields.len();
        if n == 0 {
            // 空问卷会开出一张「0/0、无题、Enter 发空 answers」的表：没有可答的东西，
            // 说出来比让用户对着空表单猜好。
            self.st
                .push_line(LineKind::System, "模型下发的澄清问卷没有题目，已忽略");
            return;
        }
        // 上一份还没交（`Esc` 收起过）就被顶掉：草稿一起没，先说一句，
        // 否则用户只会觉得「我刚填的字自己消失了」。
        if self
            .st
            .clarify
            .as_ref()
            .is_some_and(|ov| ov.form.filled_count() > 0)
        {
            self.st.push_line(
                LineKind::System,
                "上一份澄清问卷尚未提交，草稿已被新问卷替换",
            );
        }
        self.st.clarify = Some(ClarifyOverlay::new(info));
        self.st.push_line(
            LineKind::System,
            &format!("模型请求澄清（{n} 题）：本回合结束后自动打开问卷"),
        );
    }

    /// 回合收尾：有待填问卷就展开浮层（`TurnDone` 处调用）。
    ///
    /// 排在「发出排队消息」之后、并回看 `running`：排队的那一句会在同一处立刻开一个
    /// 新回合，此时开浮层既挡住它的 transcript，又留下一个可按 Enter 的模态 ——
    /// 而 Enter 会再发一轮，两个流并发写同一条 lane。
    pub(super) fn open_clarify_after_turn(&mut self) {
        if self.st.clarify.is_some() && !self.st.running {
            self.st.clarify_open = true;
        }
    }

    /// 普通回合发出时调用：上一份未提交的问卷随这一轮一起作废。
    ///
    /// `Esc` 收起时提示写的是「这一轮等于没回答」——答案只能随**紧接着的下一条**请求
    /// 回去。用户已经改主意、发了别的消息，那份 `questionnaire_id` 就再也接不回去了；
    /// 留着它只会在之后每一轮结束时反复弹出（提交还会带上过期的 id）。
    pub(super) fn discard_stale_clarify(&mut self) {
        if self.st.clarify.take().is_none() {
            return;
        }
        self.st.clarify_open = false;
        self.st
            .push_line(LineKind::System, "上一份澄清问卷已作废（已开始新回合）");
    }

    /// `F7`：把收起过的问卷重新打开。没有待填问卷时返回 `false`（交给别处处理）。
    pub(super) fn reopen_clarify(&mut self) -> bool {
        if self.st.clarify.is_none() {
            return false;
        }
        self.st.clarify_open = true;
        true
    }

    /// 浮层开着就交给 [`Self::on_clarify_key`]，否则不动。
    ///
    /// 单独一层是为了 `on_key` 的 CCN：那里的 if 链贴着门禁，而这里三个 `&&` 本来就
    /// 各算一个分支 —— 摊在 `on_key` 上直接把那条链顶过 10。
    pub(super) fn consume_clarify_key(&mut self, key: &KeyEvent) -> bool {
        self.st.clarify_open && self.st.clarify.is_some() && self.on_clarify_key(key)
    }

    /// 非阻塞模态浮层（澄清问卷 / 工具详情）的按键接管，供 `on_key` 那条 if 用。
    ///
    /// 合在一处是为了那条 if 链不随浮层种类增长而顶到 CCN 门禁；两个判定各自留在
    /// 自己的模块里（问卷在本文件，详情在 `detail`）。问卷优先：它是等着作答的表单，
    /// 而详情只是只读浏览。
    pub(super) fn consume_modal_key(&mut self, key: &KeyEvent) -> bool {
        self.consume_clarify_key(key) || self.consume_detail_key(key)
    }

    /// 浮层按键。返回 `true` = 已消费（否则交回常规分发）。
    ///
    /// 分两层是为了压 CCN：这一层只认「离开 / 提交 / 换题」，字符编辑在
    /// [`Self::on_clarify_edit_key`]。合成一层的话光 match 臂就顶到门禁。
    pub(super) fn on_clarify_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.close_clarify();
                true
            }
            KeyCode::Enter => {
                self.submit_clarify();
                true
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Down | KeyCode::Up => {
                self.on_clarify_focus_key(key);
                true
            }
            _ => self.on_clarify_edit_key(key),
        }
    }

    /// 换题：`Tab` / `Down` 往后，`Shift+Tab` / `BackTab` / `Up` 往前。
    fn on_clarify_focus_key(&mut self, key: &KeyEvent) {
        let forward = matches!(key.code, KeyCode::Tab | KeyCode::Down)
            && !key.modifiers.contains(KeyModifiers::SHIFT);
        let Some(ov) = self.st.clarify.as_mut() else {
            return;
        };
        if forward {
            ov.focus_next();
        } else {
            ov.focus_prev();
        }
    }

    /// 字符编辑：只动**当前题**的草稿与光标。
    fn on_clarify_edit_key(&mut self, key: &KeyEvent) -> bool {
        let Some(ov) = self.st.clarify.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                ov.insert_char(c);
                true
            }
            KeyCode::Backspace => {
                ov.backspace();
                true
            }
            // `Delete` 是向后删的对偶：问卷答案常有几十个字，改中间的错字不该把右边重打。
            KeyCode::Delete => {
                ov.delete_forward();
                true
            }
            KeyCode::Left => {
                ov.move_cursor(false);
                true
            }
            KeyCode::Right => {
                ov.move_cursor(true);
                true
            }
            _ => false,
        }
    }

    /// `Esc`：收起浮层。**问卷不丢**，下次 `F7` 还能接着填。
    ///
    /// issue 期望 3：要让用户知道「这一轮等于没回答」—— 只在第一句说「已收起」太轻，
    /// 用户会以为答案已经提交了。
    fn close_clarify(&mut self) {
        if self.st.clarify.is_none() {
            return;
        }
        self.st.clarify_open = false;
        self.st.push_line(
            LineKind::System,
            &format!("问卷已收起，尚未作答：这一轮等于没回答。按 {CLARIFY_REOPEN_KEY} 可重新打开"),
        );
    }

    /// `Enter`：必填校验通过就作为下一轮请求发出；缺必答时把光标跳到那一题并说清是哪题。
    fn submit_clarify(&mut self) {
        // 回合在跑就不能发：`dispatch_turn` 会顶掉 `cancel` / `sent_turn`（旧回合再也
        // 取消不了、`/resume` 指向错的那轮），两个 SSE 流写进同一条 lane，先结束的那个
        // `TurnDone` 还会把 `running` 清成空闲。浮层留着，答完再提交。
        if self.st.running {
            self.st.push_line(
                LineKind::System,
                "回合进行中：等它结束后再提交问卷（答案还在）",
            );
            return;
        }
        let outcome = {
            let Some(ov) = self.st.clarify.as_mut() else {
                return;
            };
            match ov.form.answers_json() {
                Ok(v) => Ok(v),
                Err(missing) => {
                    // 光标跳过去：12 题的表单里只留一句「有必答题没填」等于让人从头找。
                    ov.focus = missing.index;
                    ov.error = Some(missing.index);
                    Err(missing.label)
                }
            }
        };
        match outcome {
            Ok(answers) => {
                self.st.clarify = None;
                self.st.clarify_open = false;
                self.start_turn_with_clarify(CLARIFY_SUBMIT_STUB, answers);
            }
            Err(label) => self.st.push_line(
                LineKind::System,
                &format!("「{label}」是必答题，填上才能提交"),
            ),
        }
    }
}

impl ClarifyOverlay {
    /// 在当前题的光标处插入字符。
    pub(super) fn insert_char(&mut self, c: char) {
        let cursor = self.cursor();
        if let Some(v) = self.current_mut() {
            let byte = char_to_byte(v, cursor);
            v.insert(byte, c);
        }
        self.set_cursor(cursor + 1);
        self.error = None;
    }

    /// 删掉光标前一个字符（删成功才算「消费」；渲染侧不必再判）。
    pub(super) fn backspace(&mut self) {
        let cursor = self.cursor();
        if cursor == 0 {
            return;
        }
        if let Some(v) = self.current_mut() {
            let byte = char_to_byte(v, cursor - 1);
            v.remove(byte);
        }
        self.set_cursor(cursor - 1);
        self.error = None;
    }

    /// 删掉光标**处**（之后）的一个字符：`backspace` 的对偶，光标不动。
    ///
    /// 光标已在末尾时是 no-op —— 与 `backspace` 在开头时对称。按字符走而不是按字节：
    /// 中文一个字三字节，按字节删会切出半个字并让 `String::remove` 在非字符边界 panic。
    pub(super) fn delete_forward(&mut self) {
        let cursor = self.cursor();
        if cursor >= self.current_len() {
            return;
        }
        if let Some(v) = self.current_mut() {
            let byte = char_to_byte(v, cursor);
            v.remove(byte);
        }
        // 光标留在原处（向后删不移位），但「必答没填」的提示该撤掉了 —— 用户已经在改这一题。
        self.error = None;
    }

    /// 左右移动光标（按**字符**走，不能按字节 —— 中文一个字三字节）。
    pub(super) fn move_cursor(&mut self, forward: bool) {
        let cursor = self.cursor();
        let len = self.current_len();
        let next = if forward {
            (cursor + 1).min(len)
        } else {
            cursor.saturating_sub(1)
        };
        self.set_cursor(next);
    }

    /// 当前题的光标（越界时夹到草稿长度内）。
    fn cursor(&self) -> usize {
        let i = self.focus_index();
        let len = self.current_len();
        self.cursors.get(i).copied().unwrap_or(0).min(len)
    }

    fn set_cursor(&mut self, at: usize) {
        let i = self.focus_index();
        if let Some(slot) = self.cursors.get_mut(i) {
            *slot = at;
        }
    }

    /// 当前题草稿的字符数。
    fn current_len(&self) -> usize {
        let i = self.focus_index();
        self.form
            .values
            .get(i)
            .map(|s| s.chars().count())
            .unwrap_or(0)
    }
}

/// 字符下标 → 字节偏移（`String::insert` / `remove` 要字节）。
fn char_to_byte(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// 题型标记：只有 `Choice` 要标（题面绝大多数是 `Text`，标出来只是噪音）。
///
/// 文案写在这里而不是 `crabmate-client-api`：那个 crate 被 Web / TUI / connect 共用，
/// Web 有 `Locale` 体系 —— 把中文放进共享层就等于替它决定了界面语言。
fn kind_badge(kind: ClarificationKind) -> Option<&'static str> {
    match kind {
        ClarificationKind::Text => None,
        ClarificationKind::Choice => Some("选择题"),
    }
}

/// 把 `intro` 按**显示宽度**折成最多 [`INTRO_MAX_ROWS`] 行（末行超宽截断补 `…`）。
///
/// 必须自己折：浮层高度是按「intro 占几行」算出来的，交给 `Paragraph::wrap` 等于把
/// 预算与绘制拆成两处，长 intro 会折出预算外的行，把最后一题挤出可视区。
fn wrap_intro(text: &str, width: u16) -> Vec<String> {
    let limit = (width as usize).max(1);
    let mut rows: Vec<String> = vec![String::new()];
    let mut used = 0usize;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        let wrap = c == '\n' || used + w > limit;
        if wrap && rows.len() == INTRO_MAX_ROWS {
            if let Some(last) = rows.last_mut() {
                ellipsize(last, limit);
            }
            return rows;
        }
        if wrap {
            rows.push(String::new());
            used = 0;
        }
        if c != '\n'
            && let Some(last) = rows.last_mut()
        {
            last.push(c);
            used += w;
        }
    }
    rows
}

/// 给末行腾出一格再补 `…`（`pop` 不会切出半个字符）。
fn ellipsize(row: &mut String, limit: usize) {
    while row.as_str().width() >= limit {
        if row.pop().is_none() {
            break;
        }
    }
    row.push('…');
}

/// 绘制浮层（`render.rs` 的 body 分支调用）。
pub(super) fn render_clarify_overlay(frame: &mut Frame, st: &UiState, area: Rect) {
    let Some(ov) = st.clarify.as_ref() else {
        return;
    };
    let Some(rect) = clarify_rect(area, ov) else {
        return;
    };
    let pal = &st.palette;
    // 与审批浮层同样的处理：先清字形再铺主题底色，否则浅色主题下会和底下正文视觉重叠。
    frame.render_widget(Clear, rect);
    super::render::paint_bg(frame, rect, pal.overlay_bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(pal.accent))
        .title(Span::styled(
            format!(" 澄清问卷 · {}/{} ", ov.form.filled_count(), ov.len()),
            Style::new().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    // 不交给 `Wrap`：intro 已按内宽自己折好（行数正是高度预算里那几行），
    // 让渲染层再折一次会多出预算外的行，把最后一题挤出可视区。
    frame.render_widget(Paragraph::new(clarify_lines(ov, pal, inner)), inner);
}

/// 浮层矩形：宽度随屏夹取，高度按「intro 行数 + 可见题 + 键位提示」算。
fn clarify_rect(area: Rect, ov: &ClarifyOverlay) -> Option<Rect> {
    if area.width < 20 || area.height < 6 {
        // 太小就整块让位：挤到看不清的浮层比不画更糟（用户会以为键盘坏了）。
        return None;
    }
    let width = CLARIFY_MAX_WIDTH.min(area.width.saturating_sub(4)).max(20);
    let max_height = area.height.saturating_sub(2);
    let max_inner = max_height.saturating_sub(BORDER_ROWS);
    let intro = wrap_intro(&ov.form.intro, width.saturating_sub(BORDER_ROWS));
    let visible = visible_questions(ov, max_inner, intro.len() as u16);
    let height = overlay_height(intro.len() as u16, visible).min(max_height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Some(Rect::new(x, y, width, height))
}

/// 浮层总高度（含边框）：intro 行 + 每题 2 行 + 上下留白 + 提示行 + 边框。
fn overlay_height(intro_rows: u16, visible: u16) -> u16 {
    intro_rows + ROWS_PER_QUESTION * visible + BLANK_ROWS + HINT_ROWS + BORDER_ROWS
}

/// 一屏能放几题（至少 1 题；否则焦点题看不见）。
fn visible_questions(ov: &ClarifyOverlay, inner_height: u16, intro_rows: u16) -> u16 {
    // inner 里先让出 intro、两个空行与提示行，剩下的才排题 —— 与 `clarify_lines`
    // 写出的行数同一份口径（两处各算一遍就是 long intro 挤掉最后一题的根源）。
    let usable = inner_height.saturating_sub(intro_rows + BLANK_ROWS + HINT_ROWS);
    (usable / ROWS_PER_QUESTION).clamp(1, ov.len().max(1) as u16)
}

/// 组装浮层文本行：intro → 窗口内的题 → 键位提示。
fn clarify_lines(
    ov: &ClarifyOverlay,
    pal: &super::palette::Palette,
    inner: Rect,
) -> Vec<Line<'static>> {
    let intro = wrap_intro(&ov.form.intro, inner.width);
    let visible = visible_questions(ov, inner.height, intro.len() as u16) as usize;
    let from = window_start(ov.focus_index(), visible, ov.len());
    let mut lines: Vec<Line<'static>> = intro
        .into_iter()
        .map(|row| Line::from(Span::styled(row, Style::new().fg(pal.muted))))
        .collect();
    lines.push(Line::from(""));
    for (i, f) in ov.form.fields.iter().enumerate().skip(from) {
        if i >= from + visible {
            break;
        }
        lines.extend(question_lines(ov, pal, i, f));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Tab/↑↓ 换题 · Enter 提交 · Esc 稍后再填（{CLARIFY_REOPEN_KEY} 重开）"),
        Style::new().fg(pal.muted),
    )));
    lines
}

/// 窗口起点：让焦点题尽量留在可见区中段。
fn window_start(focus: usize, visible: usize, total: usize) -> usize {
    if total <= visible {
        return 0;
    }
    focus.saturating_sub(visible / 2).min(total - visible)
}

/// 单题两行：标签行（序号 / 必答标记 / 题型标记 / 提示）+ 输入行（进度条光标）。
fn question_lines(
    ov: &ClarifyOverlay,
    pal: &super::palette::Palette,
    index: usize,
    field: &ClarificationFormField,
) -> Vec<Line<'static>> {
    let focused = index == ov.focus_index();
    let marker = if focused { "▸" } else { " " };
    let mut spans = vec![
        Span::styled(
            format!("{marker} {}. ", index + 1),
            Style::new().fg(pal.accent),
        ),
        Span::styled(
            field.label.clone(),
            if focused {
                Style::new().fg(pal.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(pal.fg)
            },
        ),
    ];
    if field.required {
        spans.push(Span::styled(" *", Style::new().fg(pal.err)));
    }
    if let Some(badge) = kind_badge(field.kind) {
        spans.push(Span::styled(
            format!(" [{badge}]"),
            Style::new().fg(pal.info),
        ));
    }
    if let Some(hint) = field.hint.as_ref() {
        spans.push(Span::styled(
            format!("  {hint}"),
            Style::new().fg(pal.muted),
        ));
    }
    let value = ov.form.values.get(index).cloned().unwrap_or_default();
    // 光标用反显块，位置按**字符**算（与编辑逻辑同一套下标）。
    let cursor = ov
        .cursors
        .get(index)
        .copied()
        .unwrap_or(0)
        .min(value.chars().count());
    let (before, after) = split_at_char(&value, cursor);
    let mut input = vec![Span::styled("    ", Style::new())];
    input.push(Span::styled(before, Style::new().fg(pal.fg)));
    input.push(Span::styled("▏", Style::new().fg(pal.accent)));
    input.push(Span::styled(after, Style::new().fg(pal.fg)));
    if ov.error == Some(index) {
        input.push(Span::styled("  ← 必答题还没填", Style::new().fg(pal.err)));
    }
    vec![Line::from(spans), Line::from(input)]
}

/// 按字符切分（不用 `split_at`：那吃字节偏移，中文会 panic）。
fn split_at_char(s: &str, char_index: usize) -> (String, String) {
    let mut it = s.chars();
    let head: String = it.by_ref().take(char_index).collect();
    let tail: String = it.collect();
    (head, tail)
}

#[cfg(test)]
#[path = "clarify_tests.rs"]
mod tests;
