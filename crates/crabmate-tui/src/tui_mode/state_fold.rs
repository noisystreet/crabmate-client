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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// 折叠插入摘要行之后，一个**旧**逻辑行下标落到哪里。
///
/// 折叠是「在段首插一条摘要」，所以同一份映射要同时喂给视口锚点（`body_top_log`）
/// 和折叠光标：漏掉一个就指错行 —— 前者让滚回看的画面跳，后者让 `↑/↓` 从别处
/// 开始走（甚至指到已经隐藏的行上，`position` 找不到，锚点静默失效）。
///
/// 必须在插入**之前**调用（`runs` 里是旧下标）。
fn remap_after_fold(idx: usize, runs: &[(usize, usize)]) -> usize {
    let mut delta = 0;
    for (start, end) in runs {
        if *end <= idx {
            delta += 1;
        } else if *start <= idx {
            // 旧下标落在被折的段里：那一行已经没有物理行了，只能指向段首的摘要行。
            return *start + delta;
        }
    }
    idx + delta
}

/// 进入折叠导航的键的字符部分：`Ctrl+G`。
///
/// 写成常量而不是在函数体里直接写 `Char('g')`：lizard 的 Rust 解析对后者会把函数跨度
/// 算到几百行（`fn-nloc-ratchet` 报 nloc 286），换成常量即恢复正常，语义不变。
const FOLD_NAV_KEY: char = 'g';

/// 进入折叠导航的键：`Ctrl+G`。
///
/// 单独成函数是为了让 [`UiState::fold_nav_key`] 在「未激活」与「已激活」两条路上共用
/// 一份判定，也方便单测直接钉住这个组合（终端吞 `Ctrl+字母` 是常见现象）。
fn is_fold_nav_open_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char(FOLD_NAV_KEY) && key.modifiers.contains(KeyModifiers::CONTROL)
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

    /// 摘要行是否处于展开态（`▸ ` / `▾ ` 的前缀由它决定）。
    ///
    /// 与 `is_hidden` / `is_sticky` 放一起：读折叠态的地方若各写各的判定，迟早出现
    /// 「摘要说自己展开、成员还藏着」这种自相矛盾的行。
    pub(crate) fn is_run_open(&self) -> bool {
        self.run_open
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
        // 折完之后每段多出**一条**摘要行，所以视口锚点与折叠光标都要按这个偏移重算。
        // 不映射的话：段在锚点上方 → 钉到偏 N 行的地方；段跨过锚点 → 那一行已被隐藏、
        // 根本不产出物理行，`position` 找不到，reanchor 整个失效。必须在插入前算
        // （`runs` 是旧下标）。
        let top = self.body_top_log.map(|t| remap_after_fold(t, &runs));
        let cursor = self.fold_cursor.map(|c| remap_after_fold(c, &runs));
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
        // 光标跟着插入的摘要行走（见 `remap_after_fold`）：漏了这一步，导航会在折叠之后
        // 指向别的段 —— 而折叠恰恰发生在回合结束，正是最容易被顺手按一次的时候。
        self.fold_cursor = cursor;
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
        let idx = idxs[idxs.len() - n];
        if self.toggle_process_run(idx).is_none() {
            self.push_line(LineKind::System, "这条执行过程摘要没有可展开的内容");
        } else if self.fold_cursor.is_some() {
            // 导航开着时 `/fold` 与光标说的是同一件事：把光标挪到刚切换的那条，
            // 否则「刚展开的不是光标这条」，接着按 Enter 就折错了段。
            self.fold_cursor = Some(idx);
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

    // ── 折叠光标（`Ctrl+G`） ─────────────────────────────

    /// `Ctrl+G`：进入折叠导航，落点是最靠近底部的那条摘要。
    ///
    /// 落在「最近一条」而不是「第一条」：会话长了以后要看的基本是刚跑完的那一轮，
    /// `/fold` 的手感也是最近一条 —— 两个入口落点相同，才不用记两套。
    pub fn begin_fold_nav(&mut self) {
        match self
            .lines
            .iter()
            .rposition(|l| l.kind() == LineKind::Process)
        {
            Some(idx) => {
                self.fold_cursor = Some(idx);
                // 进模式 = 接管方向键**和视口**：搜索锚点优先于折叠光标，留着它就会出现
                // 「↑/↓ 明明在动，画面却钉在 /find 的命中行上」；`/` 补全菜单则会和导航
                // 抢同一组键 —— 两个宿主同时在线，只会互相拆台。
                self.search_cursor = None;
                self.slash.dismissed = true;
            }
            None => self.push_line(LineKind::System, "没有可展开的执行过程"),
        }
    }

    /// 折叠导航的按键入口：`Ctrl+G` 进、`↑/↓` 换一条、`Enter`/`Space` 开合、`Esc` 退。
    /// 返回 `true` = 这个键归导航。
    ///
    /// **未激活时也认 `Ctrl+G`**：进入动作挂在这里而不是 `on_input_key` 的 match 上，
    /// 是因为那条链贴着 CCN 门禁 —— 每加一个组合键就要一个臂；而 `Ctrl+G` 与「导航里
    /// 的那一堆键」本来就是同一件事，都由本模块定义。
    ///
    /// 导航激活后只借这几个键，别的原样交回输入框：导航是「边看边挑」，顺手把打字吃掉
    /// 就变成「想接着写先按 Esc」，而按 Esc 的本意通常是别的（清排队）。
    pub fn fold_nav_key(&mut self, key: &KeyEvent) -> bool {
        if self.fold_cursor.is_none() {
            if !is_fold_nav_open_key(key) {
                return false;
            }
            self.begin_fold_nav();
            return true;
        }
        match key.code {
            KeyCode::Esc => self.fold_cursor = None,
            KeyCode::Up => self.move_fold_cursor(false),
            KeyCode::Down => self.move_fold_cursor(true),
            // 只认**裸** `Enter`：`Alt+Enter` 是换行（与输入框同一套语义）。空格不做
            // 开合键 —— 它是可打印字符，占了它就等于「导航期间写不出 `a b`」，而这一点
            // 用户根本无从预料。
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => {
                self.toggle_cursor_run();
            }
            _ => return false,
        }
        true
    }

    /// 在摘要行之间上下跳。到边就停，不 wrap —— 与两侧列表的导航一致。
    fn move_fold_cursor(&mut self, down: bool) {
        let Some(cur) = self.fold_cursor else {
            return;
        };
        let next = if down {
            self.lines
                .iter()
                .enumerate()
                .skip(cur + 1)
                .find(|(_, l)| l.kind() == LineKind::Process)
        } else {
            self.lines
                .iter()
                .enumerate()
                .take(cur)
                .rev()
                .find(|(_, l)| l.kind() == LineKind::Process)
        };
        if let Some((idx, _)) = next {
            self.fold_cursor = Some(idx);
        }
    }

    /// `Enter` / `Space`：切换光标那条摘要。
    ///
    /// 光标**不动**：第二下按它是为了折回去，挪走就成了「展开一条、切换下一条」。
    /// 摘要行本身不在了（回放换了 transcript）才退出导航。
    fn toggle_cursor_run(&mut self) {
        let Some(idx) = self.fold_cursor else {
            return;
        };
        if self.lines.get(idx).map(LogLine::kind) != Some(LineKind::Process) {
            self.fold_cursor = None;
            return;
        }
        self.toggle_process_run(idx);
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

// 测试拆到独立文件：生产部分已经贴着 920 行门禁，再留 ~590 行测试必然越线
// （同 render / md / settings_panel 的拆法）。
#[cfg(test)]
#[path = "state_fold_tests.rs"]
mod tests;
