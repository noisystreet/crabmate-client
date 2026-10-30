//! 输入框 `/` 补全菜单：内建命令 + serve 侧 skill（对齐 Desktop 的 composer 浮层）。
//!
//! 触发条件与 Web 端一致：草稿是「`/` + 可选前缀、尚无空白」。一旦出现空白或第二个
//! `/` 就按普通输入处理 —— `/conv use c1` 的参数段、`/data/proj` 这类路径都不该被菜单挡住。
//!
//! 为什么菜单不并入 `controls::parse_control`：那是**提交时**的语义（决定这行文本是命令
//! 还是消息），这里只是输入期的候选提示。两者共用 `crabmate_client_api::slash` 的名字表，
//! 但候选清单是端内自己列的 —— Web 有 `/agent` `/export` `/config` 这些 TUI 没有 handler
//! 的命令，照抄会给出「选了也没用」的项。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use serde::Deserialize;
use unicode_width::UnicodeWidthStr;

use crabmate_client_api::slash;

use super::LaneApp;
use super::palette::Palette;
use super::state::UiState;
use super::text::truncate_display;
use super::worker::WorkerJob;

/// 菜单最多显示多少行（超出时窗口跟随光标）。
const MAX_MENU_ROWS: usize = 8;
/// skill 候选上限：与 Web 端一致，避免几百个 skill 把菜单撑成搜索页。
const MAX_SKILL_ROWS: usize = 24;

/// `GET /skills` 的单项（只反序列化菜单要用的字段）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillEntry {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub path: String,
}

/// `GET /skills` 响应（其余字段如 skills_dir 由 serde 忽略）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillsPayload {
    pub enabled: bool,
    #[serde(default)]
    pub skills: Vec<SkillEntry>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 已拉到的 skill 目录（菜单数据源；`None` 表示还没拉过）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsCache {
    pub enabled: bool,
    pub skills: Vec<SkillEntry>,
    pub err: Option<String>,
}

impl SkillsCache {
    #[must_use]
    pub fn from_payload(p: SkillsPayload) -> Self {
        Self {
            enabled: p.enabled,
            skills: p.skills,
            err: p.error,
        }
    }
}

/// 候选项来源：端内命令 / skill。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashKind {
    Builtin,
    Skill,
}

/// 菜单里的一个候选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashItem {
    pub kind: SlashKind,
    /// 前缀匹配键（不含 `/`）。
    pub match_key: String,
    /// 接受后写进输入框的全文。
    pub insert: String,
    /// 左侧展示（含 `/`）。
    pub label: String,
    pub description: String,
}

/// TUI 端内命令清单（`controls::parse_control` 认得、且本端有 handler 的那些）。
#[must_use]
pub fn builtin_items() -> Vec<SlashItem> {
    let item = |key: &str, insert: &str, desc: &str| SlashItem {
        kind: SlashKind::Builtin,
        match_key: key.to_string(),
        insert: insert.to_string(),
        label: format!("/{key}"),
        description: desc.to_string(),
    };
    vec![
        item(slash::HELP, "/help", "命令与快捷键说明"),
        item(
            slash::STATUS,
            "/status",
            "刷新 serve 模型 / 端点 / 会话模式",
        ),
        item("settings", "/settings", "设置面板（等同 F2）"),
        item(slash::MODEL, "/model ", "模型 override（off 清除）"),
        item(
            slash::MODE,
            "/mode ",
            "会话模式 ask / plan / act（off 清除）",
        ),
        item(slash::ROLE, "/role ", "Agent role override（off 清除）"),
        item(
            slash::PERM,
            "/perm ",
            "权限等级（off 清除；也可点状态行的 perm 段；上限仍由服务端 min 链决定）",
        ),
        item("find", "/find ", "搜索并高亮记录（off 清除）"),
        item(
            "background",
            "/background ",
            "全屏背景（kitty 走像素，其余用每格底色；off 关闭，图会压暗）",
        ),
        item(
            slash::WORKSPACE,
            "/workspace ",
            "查看 / 切换 serve 工作区根（~/ 展开、off 恢复默认，/cd 别名）",
        ),
        item(slash::CONV, "/conv ", "会话列表 / 切换 / 新建"),
        item(
            slash::RESUME,
            "/resume",
            "续传被中断的回合（serve 侧 job 仍在跑时）",
        ),
        item(
            "older",
            "/older",
            "加载更早的消息（分页向前，等同 /history）",
        ),
        item(
            "lane",
            "/lane ",
            "运行期加一条连接（attach <name> 连已就绪实例 / new <name> [workspace] 新建并等就绪）",
        ),
        item(slash::QUIT, "/quit", "退出（/exit、/q 同义）"),
    ]
}

/// 草稿是否处于「仅 `/` + 可选前缀、尚无空白」的浮层态；返回 `/` 之后的前缀。
#[must_use]
pub fn slash_prefix(draft: &str) -> Option<&str> {
    let s = draft.trim_start();
    let rest = s.strip_prefix('/')?;
    if rest.contains(char::is_whitespace) || rest.contains('/') {
        return None;
    }
    Some(rest)
}

fn matches_prefix(item: &SlashItem, prefix: &str) -> bool {
    prefix.is_empty()
        || item
            .match_key
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
}

fn skill_item(s: &SkillEntry) -> SlashItem {
    let desc = if s.description.trim().is_empty() {
        s.path.clone()
    } else {
        s.description.trim().to_string()
    };
    SlashItem {
        kind: SlashKind::Skill,
        match_key: s.id.clone(),
        insert: format!("/{} ", s.id),
        label: format!("/{}", s.id),
        description: desc,
    }
}

/// 按前缀过滤：内建命令在前，skill 在后（按 id 排序并截断）。
#[must_use]
pub fn filtered_items(prefix: &str, cache: Option<&SkillsCache>) -> Vec<SlashItem> {
    let mut out: Vec<SlashItem> = builtin_items()
        .into_iter()
        .filter(|it| matches_prefix(it, prefix))
        .collect();
    if let Some(cache) = cache
        && cache.enabled
    {
        let mut skills: Vec<SlashItem> = cache
            .skills
            .iter()
            .map(skill_item)
            .filter(|it| matches_prefix(it, prefix))
            .collect();
        skills.sort_by(|a, b| {
            a.match_key
                .to_ascii_lowercase()
                .cmp(&b.match_key.to_ascii_lowercase())
        });
        skills.truncate(MAX_SKILL_ROWS);
        out.extend(skills);
    }
    out
}

/// 菜单打开时，一个按键归一化后的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashMenuAction {
    /// Esc：收起菜单（直到草稿离开 `/` 前缀态才重开）。
    Dismiss,
    Prev,
    Next,
    /// Tab / Enter：补全当前项。
    ///
    /// `pass_when_empty` 区分两者的退路：Tab 没有候选也**吞掉**（否则焦点会跳去侧栏），
    /// Enter 没有候选则放行，交给发送路径。
    Accept {
        pass_when_empty: bool,
    },
    /// 未绑定的键：交回 `on_input_key` 的常规分发。
    Ignore,
}

/// 按键 → 动作。纯函数：`has_items` 为假时上下键不消费，免得候选为空（如 `/zzz`）
/// 时把"滚动 transcript"这个既有语义吃掉。
fn slash_menu_action(has_items: bool, key: &KeyEvent) -> SlashMenuAction {
    match key.code {
        KeyCode::Esc => SlashMenuAction::Dismiss,
        KeyCode::Up if has_items => SlashMenuAction::Prev,
        KeyCode::Down if has_items => SlashMenuAction::Next,
        KeyCode::Tab => SlashMenuAction::Accept {
            pass_when_empty: false,
        },
        // Alt+Enter 是换行（多行编辑），不抢。
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::ALT) => SlashMenuAction::Accept {
            pass_when_empty: true,
        },
        _ => SlashMenuAction::Ignore,
    }
}

/// Tab 总是吞掉按键（否则焦点跳去侧栏）；Enter 只在真补全了才吞，否则交回发送路径。
fn accept_consumes(pass_when_empty: bool, accepted: bool) -> bool {
    accepted || !pass_when_empty
}

impl LaneApp<'_> {
    /// 菜单打开时接管这几个键；返回 `true` 表示已消费。
    pub(super) fn slash_menu_key(&mut self, key: &KeyEvent) -> bool {
        let n = self.st.slash_items().len();
        match slash_menu_action(n > 0, key) {
            SlashMenuAction::Dismiss => {
                self.st.slash_dismiss();
                true
            }
            SlashMenuAction::Prev => {
                self.st.slash_step(n, -1);
                true
            }
            SlashMenuAction::Next => {
                self.st.slash_step(n, 1);
                true
            }
            SlashMenuAction::Accept { pass_when_empty } => {
                accept_consumes(pass_when_empty, self.st.slash_accept())
            }
            SlashMenuAction::Ignore => false,
        }
    }

    /// 菜单打开但还没有 skill 目录时拉一次（失败保留提示，不在每帧重试）。
    pub(super) fn maybe_fetch_skills(&mut self) {
        if !self.st.slash_menu_open() || self.st.skills.is_some() || self.st.skills_loading {
            return;
        }
        if self.job_tx.send(WorkerJob::Skills).is_err() {
            return;
        }
        self.st.skills_loading = true;
    }
}

/// 窗口起点：保证光标项可见（`rows` 为可见行数）。
fn window_start(cursor: usize, total: usize, rows: usize) -> usize {
    if total <= rows || rows == 0 {
        return 0;
    }
    cursor.saturating_sub(rows - 1).min(total - rows)
}

/// 菜单行：`label` 定宽成列，右侧接描述；整行铺 `overlay_bg`，选中行用 `selection_bg`。
///
/// 分三段拼（命令名 / 描述 / 右侧补白）而不是「先拼整行再按列切」：命令名要单独上色，
/// 整行切一次等于把已经确定好的样式再按显示宽找回来，白算一遍还容易在 CJK 上切歪。
fn menu_line(
    item: &SlashItem,
    label_w: usize,
    width: usize,
    selected: bool,
    pal: &Palette,
) -> Line<'static> {
    let bg = if selected {
        pal.selection_bg
    } else {
        pal.overlay_bg
    };
    let base = Style::new().fg(pal.muted).bg(bg);
    let pad = label_w.saturating_sub(UnicodeWidthStr::width(item.label.as_str()));
    // 窄终端下命令名列本身可能就超宽：先把它截到一行内，描述就没地方了。
    let head = truncate_display(&format!(" {}{} ", item.label, " ".repeat(pad)), width);
    let head_cells = UnicodeWidthStr::width(head.as_str());
    let desc = truncate_display(&item.description, width.saturating_sub(head_cells));
    let tail = width.saturating_sub(head_cells + UnicodeWidthStr::width(desc.as_str()));
    let mut label_style = Style::new()
        .fg(if selected { pal.accent } else { pal.fg })
        .bg(bg);
    if selected {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        Span::styled(head, label_style),
        Span::styled(desc, base),
        Span::styled(" ".repeat(tail), base),
    ])
}

/// 菜单底部的 skill 状态提示。
///
/// 拉取失败或 skills 目录未启用时必须说一句：否则「菜单里只有内建命令」和
/// 「这个工作区本来就没有 skill」在界面上完全一样，用户无从判断是空还是坏了。
fn skill_hint(st: &UiState) -> Option<(String, Color)> {
    let cache = st.skills.as_ref()?;
    if let Some(err) = cache
        .err
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        return Some((format!("skill 目录不可用：{err}"), st.palette.err));
    }
    (!cache.enabled).then(|| {
        (
            "skills 未启用（serve 侧 skills 目录为空或已关闭）".to_string(),
            st.palette.muted,
        )
    })
}

/// 单行提示：按显示宽截断后补满，整行铺卡片底色。
fn hint_line(text: &str, width: usize, fg: Color, pal: &Palette) -> Line<'static> {
    let body = truncate_display(text, width);
    let used = UnicodeWidthStr::width(body.as_str());
    Line::from(Span::styled(
        format!("{body}{}", " ".repeat(width.saturating_sub(used))),
        Style::new().fg(fg).bg(pal.overlay_bg),
    ))
}

/// 在聊天区底部（composer 正上方）画菜单；未打开或没有候选也没有提示时什么都不做。
pub(super) fn render_slash_menu(frame: &mut Frame, st: &UiState, area: Rect) {
    if !st.slash_menu_open() {
        return;
    }
    let items = st.slash_items();
    let hint = skill_hint(st);
    if items.is_empty() && hint.is_none() {
        return;
    }
    let reserved = u16::from(hint.is_some());
    // 至少给 transcript 留 1 行，避免极矮终端里菜单把正文挤没。
    let items_cap = area.height.saturating_sub(1).saturating_sub(reserved) as usize;
    let rows = items.len().min(MAX_MENU_ROWS).min(items_cap);
    let h = rows as u16 + reserved;
    if h == 0 {
        return;
    }
    let rect = Rect::new(area.x, area.y + area.height - h, area.width, h);
    let width = rect.width as usize;
    let pal = &st.palette;
    let label_w = items
        .iter()
        .map(|it| UnicodeWidthStr::width(it.label.as_str()))
        .max()
        .unwrap_or(0);
    let start = window_start(st.slash.cursor, items.len(), rows.max(1));
    let mut lines: Vec<Line<'static>> = items[start..(start + rows).min(items.len())]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            menu_line(item, label_w, width, start + offset == st.slash.cursor, pal)
        })
        .collect();
    if let Some((text, color)) = hint {
        lines.push(hint_line(&text, width, color, pal));
    }
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(pal.overlay_bg)),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn cache(ids: &[&str]) -> SkillsCache {
        SkillsCache {
            enabled: true,
            skills: ids
                .iter()
                .map(|id| SkillEntry {
                    id: (*id).to_string(),
                    description: String::new(),
                    path: format!("skills/{id}/SKILL.md"),
                })
                .collect(),
            err: None,
        }
    }

    /// 渲染一屏菜单，返回（缓冲区, 逐行文本）。菜单只在底部占几行，其余是空的。
    fn render_menu(st: &UiState) -> (ratatui::buffer::Buffer, Vec<String>) {
        let mut term = Terminal::new(TestBackend::new(40, 10)).expect("terminal");
        term.draw(|f| {
            let area = f.area();
            render_slash_menu(f, st, area);
        })
        .expect("draw");
        let buf = term.backend().buffer().clone();
        let rows = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        (buf, rows)
    }

    /// 去掉空白再比较：宽字符在缓冲区里占两格、续格是空格，逐格拼出来会变成
    /// 「skill 目 录 不 可 用」，直接 `contains` 匹配不到。
    fn squeeze(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn row_of(rows: &[String], needle: &str) -> u16 {
        let want = squeeze(needle);
        rows.iter()
            .position(|r| squeeze(r).contains(&want))
            .unwrap_or_else(|| {
                // 用具名插值而非 `{}`：lizard 会把空占位符当空代码块，区间会一路归并。
                let dump = rows.join("\n");
                panic!("屏幕上找不到 {needle}\n{dump}")
            }) as u16
    }

    #[test]
    fn prefix_requires_bare_slash_without_space() {
        assert_eq!(slash_prefix("/"), Some(""));
        assert_eq!(slash_prefix("  /he"), Some("he"));
        // 出现空白或第二个 `/` 就不再是菜单态（参数段 / 路径都走这条路）。
        assert_eq!(slash_prefix("/conv use c1"), None);
        assert_eq!(slash_prefix("/data/proj"), None);
        assert_eq!(slash_prefix("hello"), None);
        assert_eq!(slash_prefix(""), None);
    }

    #[test]
    fn builtin_items_cover_every_parse_control_command() {
        // 菜单给出的每个内建项都必须能被 parse_control 识别，否则用户选了也只是白填。
        for it in builtin_items() {
            let line = it.insert.trim_end().to_string();
            assert!(
                crate::tui_mode::controls::parse_control(&line).is_some(),
                "菜单项 {line} 不是可解析的控制命令"
            );
        }
    }

    #[test]
    fn filters_builtin_by_prefix_case_insensitively() {
        let items = filtered_items("HE", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "/help");
        // 空前缀列出全部内建命令。
        assert_eq!(filtered_items("", None).len(), builtin_items().len());
        // 没有匹配就是空列表（菜单靠调用方判空）。
        assert!(filtered_items("zzz", None).is_empty());
    }

    #[test]
    fn skills_follow_builtins_sorted_and_capped() {
        let c = cache(&["zeta", "alpha"]);
        let items = filtered_items("", Some(&c));
        let skills: Vec<&str> = items
            .iter()
            .filter(|it| it.kind == SlashKind::Skill)
            .map(|it| it.label.as_str())
            .collect();
        assert_eq!(skills, vec!["/alpha", "/zeta"]);
        // 内建永远排在 skill 前面。
        assert!(
            items
                .iter()
                .position(|it| it.kind == SlashKind::Builtin)
                .unwrap()
                < items.len()
        );
    }

    #[test]
    fn disabled_skills_are_not_listed() {
        let mut c = cache(&["alpha"]);
        c.enabled = false;
        assert!(
            filtered_items("", Some(&c))
                .iter()
                .all(|it| it.kind == SlashKind::Builtin)
        );
    }

    #[test]
    fn skill_without_description_shows_its_path() {
        let c = cache(&["alpha"]);
        let items = filtered_items("alp", Some(&c));
        assert_eq!(items[0].insert, "/alpha ");
        assert_eq!(items[0].description, "skills/alpha/SKILL.md");
    }

    #[test]
    fn window_start_keeps_cursor_visible() {
        assert_eq!(window_start(0, 3, 8), 0);
        assert_eq!(window_start(7, 20, 8), 0);
        assert_eq!(window_start(8, 20, 8), 1);
        assert_eq!(window_start(19, 20, 8), 12);
        assert_eq!(window_start(5, 0, 8), 0);
    }

    #[test]
    fn payload_maps_to_cache_and_keeps_server_error() {
        let payload: SkillsPayload = serde_json::from_str(
            r#"{"enabled":true,"skills_dir":"/s","skills":[{"id":"a","description":"d","path":"p"}]}"#,
        )
        .expect("payload");
        let c = SkillsCache::from_payload(payload);
        assert!(c.enabled);
        assert_eq!(c.skills.len(), 1);
        assert_eq!(c.err, None);

        let broken: SkillsPayload =
            serde_json::from_str(r#"{"enabled":false,"error":"skills disabled"}"#)
                .expect("payload");
        assert_eq!(
            SkillsCache::from_payload(broken).err.as_deref(),
            Some("skills disabled")
        );
    }

    #[test]
    fn menu_opens_on_bare_slash_and_closes_once_arguments_start() {
        let mut st = UiState::new();
        st.insert_char('/');
        st.slash_after_input();
        assert!(st.slash_menu_open());
        assert!(!st.slash_items().is_empty());
        // 打了空格就进入参数段：菜单消失，但草稿本身不动。
        st.insert_char(' ');
        st.slash_after_input();
        assert!(!st.slash_menu_open());
        assert_eq!(st.current_input(), "/ ");
    }

    #[test]
    fn accepting_a_command_writes_the_insert_text_and_collapses() {
        let mut st = UiState::new();
        st.set_input("/mod");
        st.slash_after_input();
        let labels: Vec<String> = st.slash_items().into_iter().map(|it| it.label).collect();
        assert_eq!(labels, vec!["/model", "/mode"]);
        assert!(st.slash_accept());
        // 带参数的命令补一个尾随空格，方便接着敲参数。
        assert_eq!(st.current_input(), "/model ");
        st.slash_after_input();
        assert!(!st.slash_menu_open());

        // 无参命令补全后草稿仍是前缀态，但这次不该再弹回来。
        let mut st = UiState::new();
        st.set_input("/hel");
        st.slash_after_input();
        assert!(st.slash_accept());
        assert_eq!(st.current_input(), "/help");
        st.slash_after_input();
        assert!(!st.slash_menu_open());
        assert!(st.slash_items().is_empty());
    }

    #[test]
    fn arrow_up_wraps_from_the_first_item_to_the_last() {
        let mut st = UiState::new();
        st.insert_char('/');
        st.slash_after_input();
        let n = st.slash_items().len();
        st.slash_step(n, -1);
        assert_eq!(st.slash.cursor, n - 1);
    }

    #[test]
    fn escape_closes_until_the_draft_leaves_the_prefix_state() {
        let mut st = UiState::new();
        st.insert_char('/');
        st.slash_after_input();
        st.slash_dismiss();
        assert!(!st.slash_menu_open());
        // 继续在前缀态里打字不会把它叫回来。
        st.insert_char('h');
        st.slash_after_input();
        assert!(!st.slash_menu_open());
        // 离开前缀态（补空格）后再回到 `/` 才重开。
        st.insert_char(' ');
        st.slash_after_input();
        assert!(!st.slash.dismissed);
        st.set_input("/he");
        st.slash_after_input();
        assert!(st.slash_menu_open());
    }

    #[test]
    fn cursor_clamps_when_the_candidate_list_shrinks() {
        let mut st = UiState::new();
        st.insert_char('/');
        st.slash_after_input();
        let n = st.slash_items().len();
        st.slash.cursor = n - 1;
        st.set_input("/quit");
        st.slash_after_input();
        assert_eq!(st.slash.cursor, 0);
        assert_eq!(st.slash_items().len(), 1);
    }

    #[test]
    fn failed_skill_fetch_is_cached_once_and_lists_nothing() {
        let mut st = UiState::new();
        st.skills_loading = true;
        st.set_skills(Err("boom".to_string()));
        assert!(!st.skills_loading);
        let cache = st.skills.as_ref().expect("cached failure");
        assert_eq!(cache.err.as_deref(), Some("boom"));
        st.set_input("/");
        st.slash_after_input();
        // 只剩内建命令：失败不是「菜单卡在 loading」。
        assert!(
            st.slash_items()
                .iter()
                .all(|it| it.kind == SlashKind::Builtin)
        );
        // 换工作区后整套清掉，下次打开菜单会重拉。
        st.clear_skills();
        assert!(st.skills.is_none());
    }

    #[test]
    fn menu_reaches_the_buffer_and_marks_the_selected_row() {
        let mut st = UiState::new();
        st.insert_char('/');
        st.slash_after_input();
        let (buf, rows) = render_menu(&st);
        // 按特征定位而非坐标：菜单行数 / 贴靠方式调整不该让这条断言变红。
        let help_y = row_of(&rows, "/help");
        let status_y = row_of(&rows, "/status");
        assert!(status_y > help_y, "候选顺序应与清单一致");
        // 光标项（首项）整行首格铺选中底色，其余候选铺卡片底色。
        assert_eq!(buf[(0, help_y)].bg, st.palette.selection_bg);
        assert_eq!(buf[(0, status_y)].bg, st.palette.overlay_bg);
        // 选中行除宽字符续格（Reset）外不应有异色格。
        let stray: Vec<u16> = (0..buf.area.width)
            .filter(|x| {
                let bg = buf[(*x, help_y)].bg;
                bg != st.palette.selection_bg && bg != Color::Reset
            })
            .collect();
        assert!(stray.is_empty(), "选中行有异色格: {stray:?}");
    }

    #[test]
    fn menu_keeps_one_row_of_transcript_and_says_why_skills_are_missing() {
        let mut st = UiState::new();
        st.set_skills(Ok(SkillsPayload {
            enabled: false,
            skills: Vec::new(),
            error: None,
        }));
        st.insert_char('/');
        st.slash_after_input();
        let (buf, rows) = render_menu(&st);
        // 提示行就在提示区最底部，且说明的是「未启用」而不是沉默。
        let hint_y = row_of(&rows, "skills 未启用");
        assert_eq!(hint_y, buf.area.height - 1);
        // 菜单（含提示行）不得吃掉整屏：顶部仍留空行给 transcript。
        let top = rows[0].clone();
        assert!(top.trim().is_empty(), "{top:?}");
    }

    #[test]
    fn skill_fetch_error_is_shown_in_the_footer() {
        let mut st = UiState::new();
        st.set_skills(Err("connect refused".to_string()));
        st.insert_char('/');
        st.slash_after_input();
        let (_buf, rows) = render_menu(&st);
        let hint_y = row_of(&rows, "skill 目录不可用");
        assert!(squeeze(&rows[hint_y as usize]).contains(&squeeze("connect refused")));
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn key_mapping_covers_close_step_accept_and_alt_enter() {
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Esc)),
            SlashMenuAction::Dismiss
        );
        // Esc 即使候选为空也照样关闭。
        assert_eq!(
            slash_menu_action(false, &key(KeyCode::Esc)),
            SlashMenuAction::Dismiss
        );
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Up)),
            SlashMenuAction::Prev
        );
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Down)),
            SlashMenuAction::Next
        );
        // 候选为空时上下键放行：`/zzz` 这类输入不能把 transcript 的滚动吃掉。
        assert_eq!(
            slash_menu_action(false, &key(KeyCode::Up)),
            SlashMenuAction::Ignore
        );
        assert_eq!(
            slash_menu_action(false, &key(KeyCode::Down)),
            SlashMenuAction::Ignore
        );
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Tab)),
            SlashMenuAction::Accept {
                pass_when_empty: false
            }
        );
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Enter)),
            SlashMenuAction::Accept {
                pass_when_empty: true
            }
        );
        // Alt+Enter 是换行，不能被补全抢走。
        assert_eq!(
            slash_menu_action(true, &KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            SlashMenuAction::Ignore
        );
        assert_eq!(
            slash_menu_action(true, &key(KeyCode::Char('a'))),
            SlashMenuAction::Ignore
        );
    }

    #[test]
    fn tab_always_consumes_but_enter_yields_when_nothing_to_accept() {
        // Tab：一律吞掉，否则焦点会跳去侧栏。
        assert!(accept_consumes(false, false));
        assert!(accept_consumes(false, true));
        // Enter：没东西可补全就放行给发送路径。
        assert!(!accept_consumes(true, false));
        assert!(accept_consumes(true, true));
    }
}
