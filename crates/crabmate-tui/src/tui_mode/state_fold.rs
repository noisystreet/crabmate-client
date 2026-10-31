//! 回合执行过程的折叠与展开（issue #369）。
//!
//! 折叠单元是**连续的过程行段**（`Thinking` / `Tool`），不是「第 N 轮到第 M 轮」：
//! 区间在 `/older` 前插历史（`state_history::prepend_replayed`）后全错位，而连续段在
//! 实时与回放里是同一个定义 —— 两处复用同一个函数，且幂等（生成的 `Process` 行不是
//! 过程行，不会被再折一次）。
//!
//! 段内原文**全部保留**（只标 [`LogLine::is_hidden`]），折叠时在段首**插入**一条摘要行；
//! 开合之后不再增删行。改写段首行会让展开无法逐字还原，那是这一层唯一的不可逆损失，
//! 不能接受。
//!
//! 展开是**可逆**的（`/fold` 就是 toggle）：摘要行始终保留，开合只翻成员的 `hidden`，
//! 外加一个 `run_open` 位决定前缀 `▸ ` / `▾ `。所以在折叠态与展开态之间来回切换是
//! 逐字还原，行数也不变 —— 视口锚点、`tool_pending` 这类按下标记的东西不会因为一次
//! toggle 整体错位。第一版（PR370）的展开会删掉摘要行，那让「再折回」无从下手。
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

/// `/fold` 的目标（`Last` = 无参数，最近一条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FoldTarget {
    Last,
    /// 从底部数第 n 条（1 起）：`1` 与 [`Self::Last`] 同义，留着数字只是让
    /// 「倒数第几条」能一次说清。
    Nth(usize),
    /// 全部展开。
    All,
    /// 全部折回。
    Close,
}

/// `/fold [n|all|close]` 的参数解析。
///
/// 与 [`super::hotkeys::of`] 同一手法：早返回 + 查表式 match，让 `fold_command` 那条
/// 分派不往 CCN 10 上再加分支；用法提示作为 `Err` 交给调用方写进 transcript
/// （与 `/lane` 的 `Err(usage)` 同一条规矩）。
fn parse_fold_arg(arg: Option<&str>) -> Result<FoldTarget, String> {
    const USAGE: &str = "用法：/fold [n|all|close] —— n 从底部数（1 起），all 全展开，close 全折回";
    let Some(raw) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(FoldTarget::Last);
    };
    if raw.eq_ignore_ascii_case("all") {
        return Ok(FoldTarget::All);
    }
    if matches!(raw, "close" | "none" | "off") {
        return Ok(FoldTarget::Close);
    }
    match raw.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(FoldTarget::Nth(n)),
        _ => Err(USAGE.to_string()),
    }
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

    /// `sticky` 是**双向**的：展开时置上（下一轮结束的自动折叠别把它收走），折回时
    /// 清掉（否则这一段再也折不上，只能切会话重新回放）。
    fn set_sticky(&mut self, sticky: bool) {
        if self.sticky_visible != sticky {
            self.sticky_visible = sticky;
            self.touch();
        }
    }

    /// 摘要行开合（同 `set_collapsed`：没变不动版本）。只对 `LineKind::Process` 有意义。
    fn set_run_open(&mut self, open: bool) {
        if self.run_open != open {
            self.run_open = open;
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

    /// 摘要行之后连续属于本段的过程行（`[start, end)`，半开）。
    ///
    /// 顺序即结构，两种态共用一份判据：折叠态成员带 `hidden`，展开态成员带 `sticky`。
    /// 认这两种标记（而不是只认 `hidden`）让 toggle 不必先问「它现在是哪一态」；
    /// 遇到的第一个两者皆无的过程行 = 本段结束 —— 那可能是下一轮刚起的工具行，
    /// 没折过，不该被这一下带走。
    fn run_members(&self, idx: usize) -> Option<(usize, usize)> {
        let start = idx + 1;
        let mut end = start;
        while let Some(line) = self.lines.get(end) {
            let member = is_process(line.kind()) && (line.is_hidden() || line.is_sticky());
            if !member {
                break;
            }
            end += 1;
        }
        (end > start).then_some((start, end))
    }

    /// 把第 `idx` 行那条摘要设成 `open`（成员显隐 + `sticky` 一起改）。返回是否改了。
    fn set_run_state(&mut self, idx: usize, open: bool) -> bool {
        if self.lines.get(idx).map(LogLine::kind) != Some(LineKind::Process) {
            return false;
        }
        let Some((start, end)) = self.run_members(idx) else {
            return false;
        };
        let top = self.body_top_log;
        for line in &mut self.lines[start..end] {
            line.set_hidden(!open);
            // `sticky`：展开时这是用户显式要看的原文，下一轮结束的自动折叠不该收走；
            // 折回时清掉，否则这一段再也折不上。
            line.set_sticky(open);
        }
        self.lines[idx].set_run_open(open);
        self.after_fold_change(top);
        true
    }

    /// 切换第 `idx` 行那条摘要的展开态，返回切换后的状态（`None` = 不是摘要行 / 没有成员）。
    ///
    /// 摘要行**始终保留**：开合只翻成员的 `hidden`，所以来回切换逐字还原、行数不变。
    pub fn toggle_process_run(&mut self, idx: usize) -> Option<bool> {
        let open = !self.lines.get(idx)?.is_run_open();
        self.set_run_state(idx, open).then_some(open)
    }

    /// 所有摘要行的下标（自上而下）。
    fn summary_indices(&self) -> Vec<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind() == LineKind::Process)
            .map(|(i, _)| i)
            .collect()
    }

    /// `/fold [n|all|close]`：切换执行过程的展开态。
    ///
    /// 折叠是自动的（回合结束 / 回放插入后），这条命令管另一个方向。摘要行始终在，所以
    /// 它同时是「展开」与「折回」：无参数 = 最近一条（与 PR370 的手感一致）；`<n>` =
    /// 从底部数第 n 条（1 起，`1` 与无参数同义）；`all` 全展开；`close`（`none` /
    /// `off` 同义）全折回。
    pub fn fold_command(&mut self, arg: Option<&str>) {
        let target = match parse_fold_arg(arg) {
            Ok(target) => target,
            Err(usage) => {
                self.push_line(LineKind::System, &usage);
                return;
            }
        };
        match target {
            FoldTarget::All => self.report_all_runs(true),
            FoldTarget::Close => self.report_all_runs(false),
            FoldTarget::Last => self.toggle_from_bottom(1),
            FoldTarget::Nth(n) => self.toggle_from_bottom(n),
        }
    }

    /// 从底部数第 `n` 条摘要切换一次；没成时把原因写清楚（两种「没成」不是一回事）。
    fn toggle_from_bottom(&mut self, n: usize) {
        let idxs = self.summary_indices();
        if idxs.is_empty() {
            self.push_line(LineKind::System, "没有可展开的执行过程");
            return;
        }
        if n == 0 || n > idxs.len() {
            let msg = format!("只有 {} 条执行过程摘要（n 从底部数，1 起）", idxs.len());
            self.push_line(LineKind::System, &msg);
            return;
        }
        if self.toggle_process_run(idxs[idxs.len() - n]).is_none() {
            self.push_line(LineKind::System, "这条执行过程摘要没有可展开的内容");
        }
    }

    /// `all` / `close`：把所有摘要设成同一态，并回报**实际**改了几条。
    ///
    /// 只对「当前态与目标不同」的摘要动手：`set_run_state` 只校验「是摘要行 + 有成员」，
    /// 拿它的返回值直接累加，会让连按两次 `/fold all` 都说「已展开 N 段」—— 用户据此
    /// 以为画面该变了，实际什么都没动。
    fn report_all_runs(&mut self, open: bool) {
        let what = if open { "展开" } else { "折回" };
        let targets: Vec<usize> = self
            .summary_indices()
            .into_iter()
            .filter(|&idx| self.lines[idx].is_run_open() != open)
            .collect();
        if targets.is_empty() {
            let msg = self.bulk_idle_notice(what, open);
            self.push_line(LineKind::System, &msg);
            return;
        }
        let mut changed = 0;
        for idx in targets.into_iter().rev() {
            if self.set_run_state(idx, open) {
                changed += 1;
            }
        }
        let msg = format!("已{what} {changed} 段执行过程");
        self.push_line(LineKind::System, &msg);
    }

    /// 一条都不用动时的说法：**没有摘要** 与 **都已经是目标态** 是两件事 ——
    /// 「没有可折回的执行过程」在「已经全折回」时会读成「这里没有执行过程」。
    fn bulk_idle_notice(&self, what: &str, open: bool) -> String {
        if self.summary_indices().is_empty() {
            format!("没有可{what}的执行过程")
        } else {
            format!("执行过程都已是{}", if open { "展开态" } else { "收起态" })
        }
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
    use super::{FoldTarget, parse_fold_arg};

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

    /// 第 `idx` 条逻辑行在**真实渲染**里的前缀（用户唯一看得见的状态：
    /// 折叠 `▸ ` / 展开 `▾ `）。必须走 `build_body_rows`：字段读对了但缓存没失效，
    /// 屏幕上照样是旧标记 —— 那种情况只有渲染断言能抓到。
    fn summary_prefix(st: &UiState, idx: usize) -> String {
        render::build_body_rows(st, 40)
            .iter()
            .find(|r| r.log_index == Some(idx))
            .and_then(|r| r.line.spans.first().map(|s| s.content.to_string()))
            .unwrap_or_default()
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

    /// 展开 / 折回双向都逐字还原（不是重新概括），且摘要行始终在。
    #[test]
    fn toggling_restores_the_hidden_rows_verbatim_both_ways() {
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

        assert_eq!(st.toggle_process_run(summary), Some(true), "第一次是展开");
        assert_eq!(
            visible(&st),
            vec![
                (LineKind::User, "问".to_string()),
                // 摘要行**保留**：它是 toggle 的把手，删掉就再也折不回去。
                (LineKind::Process, "2 个工具 · 完成".to_string()),
                (LineKind::Tool, "✓ read_file — a.rs".to_string()),
                (LineKind::Tool, "✓ search_replace — b.rs".to_string()),
                (LineKind::Assistant, "答".to_string()),
            ],
            "展开后成员与折叠前逐字一致，另加摘要行本身"
        );
        assert_eq!(hidden_count(&st), 0, "成员行全部回到可见");
        assert_eq!(st.lines.len(), 5, "toggle 不插不删行");
        assert!(st.lines[summary].is_run_open(), "摘要行记着自己的开合");
        assert_eq!(summary_prefix(&st, summary), "▾ ", "展开态渲染出 ▾");

        assert_eq!(st.toggle_process_run(summary), Some(false), "第二次是折回");
        assert_eq!(
            visible(&st),
            vec![
                (LineKind::User, "问".to_string()),
                (LineKind::Process, "2 个工具 · 完成".to_string()),
                (LineKind::Assistant, "答".to_string()),
            ],
            "折回后只剩摘要"
        );
        assert_eq!(hidden_count(&st), 2);
        assert_eq!(st.lines.len(), 5, "折回同样不改行数");
        assert!(!st.lines[summary].is_run_open());
        assert_eq!(summary_prefix(&st, summary), "▸ ", "折回态渲染回 ▸");

        // 折回后把 `sticky` 清干净了，所以还能再展开一次（否则这段就锁死了）。
        assert_eq!(st.toggle_process_run(summary), Some(true), "折回后可再展开");
        assert_eq!(hidden_count(&st), 0);
    }

    /// `/fold` 的参数解析：无参 = 最近一条，数字从底部数，`all` / `close` 两个方向。
    #[test]
    fn fold_arg_parsing() {
        assert_eq!(parse_fold_arg(None), Ok(FoldTarget::Last));
        assert_eq!(parse_fold_arg(Some("   ")), Ok(FoldTarget::Last));
        assert_eq!(parse_fold_arg(Some("ALL")), Ok(FoldTarget::All));
        assert_eq!(parse_fold_arg(Some("close")), Ok(FoldTarget::Close));
        assert_eq!(parse_fold_arg(Some("off")), Ok(FoldTarget::Close));
        assert_eq!(parse_fold_arg(Some("2")), Ok(FoldTarget::Nth(2)));
        // `0` 与 `1` 是两种错法：0 越界，1 等价无参 —— 所以只认 `>= 1`。
        assert!(parse_fold_arg(Some("0")).is_err());
        assert!(parse_fold_arg(Some("x")).is_err(), "非数字给用法提示");
    }

    /// `/fold <n>` 从底部数：`2` 命中的是**较早**那一段，不是最近那段。
    #[test]
    fn fold_with_an_index_unfolds_the_nth_run_from_the_bottom() {
        let mut st = state_with(&[
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
            (LineKind::Assistant, "中间结论"),
            (LineKind::Tool, "✓ c"),
            (LineKind::Tool, "✓ d"),
        ]);
        st.fold_process_runs(Some(" · 完成"));
        assert_eq!(hidden_count(&st), 4, "两段都折上了");

        st.fold_command(Some("2"));

        let open: Vec<bool> = st
            .lines
            .iter()
            .filter(|l| l.kind() == LineKind::Process)
            .map(|l| l.is_run_open())
            .collect();
        assert_eq!(open, vec![true, false], "只有倒数第 2 条（较早那段）展开");
        assert_eq!(hidden_count(&st), 2, "最近那段仍折叠");
    }

    /// 越界的 `n` 只给提示，不动任何一段。
    #[test]
    fn fold_with_a_too_large_index_only_explains_itself() {
        let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
        st.fold_process_runs(None);

        st.fold_command(Some("3"));

        assert_eq!(hidden_count(&st), 2, "越界不该展开任何段");
        let last = st.lines.last().expect("应有提示行");
        assert_eq!(last.kind(), LineKind::System);
        assert!(last.text().contains("只有 1 条"), "{}", last.text());
    }

    /// `/fold all` 全展开、`/fold close` 全折回，两个方向分别回显改了几段。
    #[test]
    fn fold_all_and_close_move_every_run() {
        let mut st = state_with(&[
            (LineKind::Tool, "✓ a"),
            (LineKind::Tool, "✓ b"),
            (LineKind::Assistant, "中间结论"),
            (LineKind::Tool, "✓ c"),
            (LineKind::Tool, "✓ d"),
        ]);
        st.fold_process_runs(None);

        st.fold_command(Some("all"));
        assert_eq!(hidden_count(&st), 0, "全部展开");
        assert!(
            st.lines
                .last()
                .expect("回显")
                .text()
                .contains("已展开 2 段")
        );
        // 再按一次 `all`：一段都没动，就不能照抄上一次的「已展开 2 段」——
        // 用户会据此以为画面该变了。
        st.fold_command(Some("all"));
        assert_eq!(hidden_count(&st), 0, "重复 all 不该改变任何东西");
        assert!(
            st.lines
                .last()
                .expect("回显")
                .text()
                .contains("都已是展开态"),
            "{}",
            st.lines.last().expect("回显").text()
        );

        st.fold_command(Some("close"));
        assert_eq!(hidden_count(&st), 4, "全部折回");
        assert!(
            st.lines
                .last()
                .expect("回显")
                .text()
                .contains("已折回 2 段")
        );
        // `close` 同理：都已经收起时说「都已是收起态」，而不是再报一次「已折回 2 段」，
        // 更不是「没有可折回的执行过程」（那读起来像「这里没有执行过程」）。
        st.fold_command(Some("close"));
        assert!(
            st.lines
                .last()
                .expect("回显")
                .text()
                .contains("都已是收起态"),
            "{}",
            st.lines.last().expect("回显").text()
        );
        assert!(
            st.lines
                .iter()
                .filter(|l| l.kind() == LineKind::Process)
                .all(|l| !l.is_run_open()),
            "折回后没有一条仍记着展开"
        );
    }

    /// 一条摘要都没有时，`/fold` 说明「没有可展开的」而不是静默。
    #[test]
    fn fold_without_any_summary_says_so() {
        let mut st = state_with(&[(LineKind::User, "问"), (LineKind::Assistant, "答")]);

        st.fold_command(None);

        let last = st.lines.last().expect("应有提示行");
        assert_eq!(last.text(), "没有可展开的执行过程");
    }

    /// 非法参数给用法，且不碰 transcript 里已有的过程段。
    #[test]
    fn fold_with_a_bad_argument_prints_usage() {
        let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
        st.fold_process_runs(None);

        st.fold_command(Some("bogus"));

        assert_eq!(hidden_count(&st), 2, "参数错不该展开任何段");
        let last = st.lines.last().expect("应有用法行");
        assert!(last.text().contains("用法：/fold"), "{}", last.text());
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

    /// `/fold` 展开的原文，下一轮结束时不被重新折回去；新的一轮单独成段。
    #[test]
    fn an_expanded_run_is_not_refolded_on_the_next_turn() {
        let mut st = state_with(&[(LineKind::Tool, "✓ a"), (LineKind::Tool, "✓ b")]);
        st.fold_process_runs(Some(" · 完成"));
        st.fold_command(None); // 等价 `/fold`
        assert_eq!(visible(&st).len(), 3, "摘要 + 两条原文");

        st.push_line(LineKind::Tool, "✓ c");
        st.push_line(LineKind::Tool, "✓ d");
        st.fold_after_turn(false, false);

        let shown = visible(&st);
        assert_eq!(
            shown
                .iter()
                .filter(|(k, _)| *k == LineKind::Process)
                .count(),
            2,
            "旧段仍展开、新段单独折：{shown:?}"
        );
        assert_eq!(shown.len(), 4, "摘要A + 两条旧原文 + 摘要B：{shown:?}");
    }
}
