//! 回合执行过程的折叠与展开（issue #369）。
//!
//! 折叠单元是**连续的过程行段**（`Thinking` / `Tool`），不是「第 N 轮到第 M 轮」：
//! 区间在 `/older` 前插历史（`state_history::prepend_replayed`）后全错位，而连续段在
//! 实时与回放里是同一个定义 —— 两处复用同一个函数，且幂等（生成的 `Process` 行不是
//! 过程行，不会被再折一次）。
//!
//! 段内原文**全部保留**（只标 [`LogLine::is_hidden`]），摘要行另插一条。改写段首行会
//! 让展开无法逐字还原，那是这一层唯一的不可逆损失，不能接受。
//!
//! 为什么不用 `LogLine::collapsed`：那个的语义是「一行折成一行预览」（`fold_thinking`），
//! `Ctrl+E` 靠它表达 thinking 展开。复用它会让「展开思考」把整段也一起展开。

use std::fmt::Write as _;

use super::{LineKind, LogLine, UiState};

/// 少于这么多条过程行就不折：一条工具行折成一条摘要只是换个说法，还多出一次
/// 「这里折了什么」的困惑。
const MIN_RUN: usize = 2;

/// 是否是「执行过程」行（可被折叠）。
///
/// 只认这两种：`Assistant` 是答案、`System` 是错误与中断提示，折掉就等于丢结果；
/// `User` 与 `Process` 自然不在内。
fn is_process(kind: LineKind) -> bool {
    matches!(kind, LineKind::Thinking | LineKind::Tool)
}

/// 折叠 / 展开只经由本文件改这几个标记：它们成对出现，谁在别处直接改，
/// 摘要行与成员行就会不一致（摘要行还在、成员行已经回来了，或者反过来）。
impl LogLine {
    pub(crate) fn is_hidden(&self) -> bool {
        self.hidden
    }

    fn set_hidden(&mut self, hidden: bool) {
        if self.hidden != hidden {
            self.hidden = hidden;
            self.touch();
        }
    }

    /// 用户是否用 `/fold` 显式展开过这一行。
    fn is_sticky(&self) -> bool {
        self.sticky_visible
    }

    fn set_sticky(&mut self) {
        if !self.sticky_visible {
            self.sticky_visible = true;
            self.touch();
        }
    }
}

impl UiState {
    /// 把当前 transcript 里所有连续的过程行段折成摘要行。
    ///
    /// `suffix` 追加在读数之后（耗时 / 结局），由调用方给：实时路径知道这一轮跑了多久、
    /// 怎么结束的，回放路径不知道。已折过的段自动跳过，因此可以反复调用。
    pub fn fold_process_runs(&mut self, suffix: Option<&str>) {
        let runs = self.process_runs();
        if runs.is_empty() {
            return;
        }
        // 折完之后每段多出**一条**摘要行，所以视口锚点要按这个偏移重算。不映射的话：
        // 段在锚点上方 → 钉到偏 N 行的地方；段跨过锚点 → 那一行已被隐藏、根本不产出
        // 物理行，`position` 找不到，reanchor 整个失效。必须在插入前算（`runs` 是旧下标）。
        let top = self.body_top_log.map(|t| {
            let mut delta = 0;
            for (start, end) in &runs {
                if *end <= t {
                    delta += 1;
                } else if *start <= t {
                    // 锚点本来就落在被折的段里：它现在就是段首那条摘要行。
                    return *start + delta;
                }
            }
            t + delta
        });
        // 从后往前：插入摘要行会让后面的下标整体 +1，倒序处理省掉一次重映射。
        for (start, end) in runs.into_iter().rev() {
            let summary = self.run_summary(start, end, suffix);
            for line in &mut self.lines[start..end] {
                line.set_hidden(true);
            }
            self.lines
                .insert(start, LogLine::new(LineKind::Process, &summary, false));
        }
        // 只有回合结束路径才清工具行映射：`/older` 前插也会调到这里，而它刚把映射
        // 按前插量平移好（见 `state_history` 的 ①），清掉等于堵死「哪天放开回合内翻页」。
        if suffix.is_some() {
            self.tool_pending.clear();
        }
        self.after_fold_change(top);
    }

    /// 展开第 `idx` 行那条摘要（把它的成员行放回来、自己删掉）。返回是否真的做了。
    pub fn unfold_process_run(&mut self, idx: usize) -> bool {
        if self.lines.get(idx).map(LogLine::kind) != Some(LineKind::Process) {
            return false;
        }
        // 摘要行之后连续的隐藏行就是它的成员：折叠时正是这样排的，顺序即结构。
        let start = idx + 1;
        let mut end = start;
        while end < self.lines.len() && self.lines[end].is_hidden() {
            end += 1;
        }
        if end == start {
            // 没有成员 = 不是我们折出来的（或已被展开）—— 不删，免得吃掉一行。
            return false;
        }
        let top = self.body_top_log;
        for line in &mut self.lines[start..end] {
            line.set_hidden(false);
            // `sticky`：这是用户显式要看的原文，下一轮结束的自动折叠不该再把它收走。
            line.set_sticky();
        }
        self.lines.remove(idx);
        self.after_fold_change(top);
        true
    }

    /// 展开最后一条过程摘要（`/fold`）。没有可展开的段时返回 `false`。
    pub fn unfold_last_process_run(&mut self) -> bool {
        let Some(idx) = self
            .lines
            .iter()
            .rposition(|l| l.kind() == LineKind::Process)
        else {
            return false;
        };
        self.unfold_process_run(idx)
    }

    /// `/fold`：展开最近一条过程摘要，并把结果说清楚。
    pub fn fold_command(&mut self) {
        if self.unfold_last_process_run() {
            return;
        }
        // 两种「没展开」不是一回事：一条摘要行都没有，和有一条但成员已经不在（不一致态）。
        let msg = if self.lines.iter().any(|l| l.kind() == LineKind::Process) {
            "最近一条执行过程摘要没有可还原的内容"
        } else {
            "没有可展开的执行过程"
        };
        self.push_line(LineKind::System, msg);
    }

    /// 一轮结束时的折叠：调用点只关心「这一轮怎么结束的」。
    ///
    /// 文案只能是聚合读数 —— 一句话的任务摘要要跨工具归纳，客户端拿不到（见 issue #369
    /// 的非目标）。将来 Server 若在回合结束事件里给出摘要，这里换文案即可，结构不用动。
    pub fn fold_after_turn(&mut self, cancelled: bool, failed: bool) {
        // 取消优先于「出错」：`failed` 把 `interrupted.is_some()` 也算进去，而 Ctrl+C 的
        // 常见结局正是「cancel 未送达 / 传输中断」——先判 `failed` 会把用户主动取消的
        // 回合写成「出错」（见 `resume::record_turn_outcome` 对这两个来源的区分）。
        let tail = match (cancelled, failed) {
            (true, _) => " · 已取消",
            (_, true) => " · 出错",
            _ => " · 完成",
        };
        self.fold_process_runs(Some(tail));
    }

    /// 所有「够长、还没被折」的过程行段（`[start, end)`，半开区间）。
    fn process_runs(&self) -> Vec<(usize, usize)> {
        let mut runs = Vec::new();
        let mut i = 0;
        while i < self.lines.len() {
            if !self.is_open_process(i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < self.lines.len() && self.is_open_process(i) {
                i += 1;
            }
            if i - start >= MIN_RUN {
                runs.push((start, i));
            }
        }
        runs
    }

    /// 第 `i` 行是否是「尚未折叠的过程行」。
    ///
    /// 用户用 `/fold` 展开过的行不算（`sticky`）：否则下一轮结束又把它折回去，
    /// 等于把刚做的展开撤销掉。
    fn is_open_process(&self, i: usize) -> bool {
        self.lines
            .get(i)
            .is_some_and(|l| is_process(l.kind()) && !l.is_hidden() && !l.is_sticky())
    }

    /// 一段的摘要文本（前缀 `▸ ` 由 `prefix::kind_prefix` 给，这里不重复）。
    fn run_summary(&self, start: usize, end: usize, suffix: Option<&str>) -> String {
        let tools = self.lines[start..end]
            .iter()
            .filter(|l| l.kind() == LineKind::Tool)
            .count();
        let steps = end - start;
        let mut text = String::new();
        // 三种成分分开说：全是思考时讲「步」没意义，全是工具时「步」就是工具数。
        let _ = match (tools, steps) {
            (0, steps) => write!(text, "{steps} 段思考"),
            (tools, steps) if tools == steps => write!(text, "{tools} 个工具"),
            (tools, steps) => write!(text, "{steps} 步 · {tools} 个工具"),
        };
        if let Some(suffix) = suffix {
            text.push_str(suffix);
        }
        text
    }

    /// 折叠 / 展开之后的共同收尾：搜索重扫 + 视线锚点。
    fn after_fold_change(&mut self, top: Option<usize>) {
        // 命中集合是绝对下标，插入/删行会让它整体错位。折叠是低频操作，整表重扫
        // （O(行数)）比维护增量便宜得多 —— 与 `prepend_replayed` 同一条规矩。
        if self.search.is_some() {
            self.rescan_search();
        }
        // 锚点行本身可能已被这次改动挪走，释放比猜一个近似行更安全（`sync_search_total`
        // 在归零时也这么处理）。
        self.search_cursor = None;
        // 贴顶时继续贴顶（与 `prepend_replayed` 同一条规矩）：折叠只会让内容变短，
        // 重新打到哨兵上，渲染 clamp 后就是新的顶部。
        if self.body_at_top {
            self.view_offset = super::SCROLL_TOP;
        } else if self.view_offset > 0 {
            // 回看中：把视口钉回折叠前那条逻辑行，否则眼前内容会突然上跳。
            self.reanchor_log = top;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tui_mode::render;

    use super::super::{LineKind, UiState};

    /// 造一份最小 transcript（`push_line` 走真实路径，行号与 `search_hits` 才一致）。
    fn state_with(rows: &[(LineKind, &str)]) -> UiState {
        let mut st = UiState::new();
        for (kind, text) in rows {
            st.push_line(*kind, text);
        }
        st
    }

    fn visible(st: &UiState) -> Vec<(LineKind, String)> {
        st.lines
            .iter()
            .filter(|l| !l.is_hidden())
            .map(|l| (l.kind(), l.text().to_string()))
            .collect()
    }

    fn hidden_count(st: &UiState) -> usize {
        st.lines.iter().filter(|l| l.is_hidden()).count()
    }

    /// 一轮的工具行折成一条摘要，原文全部保留。
    #[test]
    fn a_run_of_tool_rows_folds_into_one_summary() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ read_file — a.rs"),
            (LineKind::Tool, "✓ search_replace — b.rs"),
            (LineKind::Assistant, "答"),
        ]);

        st.fold_process_runs(Some(" · 完成"));

        assert_eq!(
            visible(&st),
            vec![
                (LineKind::User, "问".to_string()),
                (LineKind::Process, "2 个工具 · 完成".to_string()),
                (LineKind::Assistant, "答".to_string()),
            ],
            "一轮只剩摘要一条，答案不动"
        );
        assert_eq!(hidden_count(&st), 2, "原文行仍留在 `lines` 里");
        assert_eq!(
            st.lines
                .iter()
                .filter(|l| l.kind() == LineKind::Process)
                .count(),
            1,
            "摘要行恰好一条"
        );
    }

    /// 展开逐字还原（不是重新概括）。
    #[test]
    fn unfolding_restores_the_hidden_rows_verbatim() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ read_file — a.rs"),
            (LineKind::Tool, "✓ search_replace — b.rs"),
            (LineKind::Assistant, "答"),
        ]);
        st.fold_process_runs(Some(" · 完成"));
        let summary = st
            .lines
            .iter()
            .position(|l| l.kind() == LineKind::Process)
            .expect("应有摘要行");

        assert!(st.unfold_process_run(summary));

        assert_eq!(
            visible(&st),
            vec![
                (LineKind::User, "问".to_string()),
                (LineKind::Tool, "✓ read_file — a.rs".to_string()),
                (LineKind::Tool, "✓ search_replace — b.rs".to_string()),
                (LineKind::Assistant, "答".to_string()),
            ],
            "展开后与折叠前逐字一致"
        );
        assert_eq!(hidden_count(&st), 0);
    }

    /// 一条过程行不值得折：折了只是换个说法。
    #[test]
    fn a_single_process_row_is_left_alone() {
        let mut st = state_with(&[(LineKind::User, "问"), (LineKind::Tool, "✓ a")]);

        st.fold_process_runs(None);

        assert_eq!(visible(&st).len(), 2, "只有一条工具行时不折");
        assert_eq!(hidden_count(&st), 0);
    }

    /// 连续的过程行才算一段：`Assistant` 会把它们切开（答案是结果，不能折）。
    #[test]
    fn assistant_rows_split_the_runs() {
        let mut st = state_with(&[
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
            (LineKind::Assistant, "中间结论"),
            (LineKind::Tool, "✓ c"),
            (LineKind::Tool, "✓ d"),
        ]);

        st.fold_process_runs(None);

        assert_eq!(
            visible(&st),
            vec![
                (LineKind::Process, "2 个工具".to_string()),
                (LineKind::Assistant, "中间结论".to_string()),
                (LineKind::Process, "2 个工具".to_string()),
            ],
            "答案两侧各折一段，答案本身永不被折"
        );
    }

    /// 混合时把「步」和「工具」分开说。
    #[test]
    fn a_mixed_run_reports_steps_and_tools() {
        let mut st = state_with(&[
            (LineKind::Thinking, "先看看"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
        ]);

        st.fold_process_runs(Some(" · 完成"));

        assert_eq!(visible(&st)[0].1, "3 步 · 2 个工具 · 完成");
    }

    /// 回看中折叠：记下视口顶端那条逻辑行，供下一帧钉回视线。
    #[test]
    fn folding_while_scrolled_back_records_an_anchor() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
        ]);
        st.view_offset = 7;
        st.body_top_log = Some(1);

        st.fold_process_runs(None);

        assert_eq!(st.reanchor_log, Some(1), "非贴底时必须钉回原视线");
    }

    /// 贴底折叠不动视口；贴顶则继续贴顶（与 `/older` 前插同一条规矩）。
    #[test]
    fn folding_at_the_bottom_leaves_the_viewport_alone() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
        ]);

        st.fold_process_runs(None);

        assert_eq!(st.reanchor_log, None, "贴底时画面本来就不动");
        assert_eq!(st.view_offset, 0);
    }

    /// 隐藏段不该在屏幕上留下一条孤立分隔线：块间距按「最后一条可见行」算。
    #[test]
    fn a_hidden_run_leaves_no_orphan_separator() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
            (LineKind::Assistant, "答"),
        ]);
        st.fold_process_runs(None);

        let rows = render::build_body_rows(&st, 40);
        let is_sep: Vec<bool> = rows.iter().map(|r| r.log_index.is_none()).collect();
        assert!(
            !is_sep.windows(2).any(|w| w[0] && w[1]),
            "相邻两条分隔行：{is_sep:?}"
        );
        assert_eq!(
            rows.iter().filter(|r| r.log_index.is_some()).count(),
            3,
            "可见行 = 问 / 摘要 / 答"
        );
    }

    /// 回看中折叠后，锚点必须指向折叠前那条逻辑行：它上面的段各插了一条摘要行，
    /// 旧下标已经偏了。上一版只断言「记了锚点」，记到错行也能通过。
    #[test]
    fn the_anchor_follows_the_rows_inserted_above_it() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
            (LineKind::Assistant, "答"),
            (LineKind::Tool, "✓ c"),
            (LineKind::Tool, "✓ d"),
        ]);
        st.view_offset = 5;
        st.body_top_log = Some(3); // 折叠前「答」所在的行

        st.fold_process_runs(None);

        let anchor = st.reanchor_log.expect("回看中应记锚点");
        assert_eq!(anchor, 4, "中间插进一条摘要行，锚点要跟着 +1");
        assert_eq!(st.lines[anchor].text(), "答", "锚点仍指向原来那一行");
    }

    /// 锚点落在被折的段里时，指向段首那条摘要行 —— 被隐藏的成员行根本不产出物理行。
    #[test]
    fn an_anchor_inside_a_folded_run_lands_on_its_summary() {
        let mut st = state_with(&[
            (LineKind::User, "问"),
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
        ]);
        st.view_offset = 3;
        st.body_top_log = Some(2); // 落在工具行里

        st.fold_process_runs(None);

        let anchor = st.reanchor_log.expect("回看中应记锚点");
        assert_eq!(
            st.lines[anchor].kind(),
            LineKind::Process,
            "被折掉的行没有物理行，锚点只能落在摘要行上"
        );
    }

    /// 用户主动取消的回合说「已取消」，不说「出错」—— `failed` 里含着 `interrupted`。
    #[test]
    fn cancel_wins_over_error_in_the_tail() {
        let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);

        st.fold_after_turn(true, true);

        assert_eq!(visible(&st)[0].1, "2 个工具 · 已取消");
    }

    /// `/fold` 展开的原文，下一轮结束时不被重新折回去。
    #[test]
    fn an_expanded_run_is_not_refolded_on_the_next_turn() {
        let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
        st.fold_process_runs(Some(" · 完成"));
        st.fold_command(); // 等价 `/fold`
        assert_eq!(visible(&st).len(), 2, "展开后原文可见");

        st.push_line(LineKind::Tool, "✓ c");
        st.push_line(LineKind::Tool, "✓ d");
        st.fold_after_turn(false, false);

        let shown = visible(&st);
        assert_eq!(
            shown
                .iter()
                .filter(|(k, _)| *k == LineKind::Process)
                .count(),
            1,
            "只折新的一轮：{shown:?}"
        );
        assert_eq!(shown.len(), 3, "两条原文 + 一条新摘要：{shown:?}");
    }
}
