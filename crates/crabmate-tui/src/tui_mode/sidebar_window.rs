//! 侧栏列表的可见窗口：条目区 `[start, end)` 的**唯一**计算来源。
//!
//! 渲染裁剪与鼠标命中检测都调这里，各算一遍就会在「列表够长且光标接近底部」时
//! 悄悄错开一行，症状是点到的行与反色高亮不同行，极难察觉。
//!
//! 窗口起点是**粘住**的：以 `prev_start` 为起点，只在光标越出窗口时做最小滚动。
//! 绝不能由光标反推（`sel + 1 - item_max`）—— 那样点一下窗口内任意行都会把该行
//! 钉到窗口底边、整列跳动，双击的第二下便落到别的行上，表现成「双击打开了错误的
//! 文件」（长树、滚到下方时最明显）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::palette::Palette;
use super::ws_sidebar::sidebar_view;

/// 侧栏列的内容区：去掉列内左留白 1 列与底部 1 行（按键提示）。
///
/// 渲染与鼠标命中检测共用；放在这里是因为它和 [`sidebar_window`] 一起定义
/// 「列表在屏幕上的位置与可见范围」这一件事。
pub(super) fn pane_list_area(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .split(inner)[0]
}

/// 侧栏一帧的内容快照：行、光标与底部提示行。
///
/// 三件东西总是一起由 `sessions_content` / `workspace_content` 产出、一起交给
/// `render_pane`，打包成一个参数既省调用处的折行，也让 clippy 的形参上限有余量。
pub(super) struct SidebarRows {
    pub rows: Vec<Line<'static>>,
    pub cursor: usize,
    pub hint: Option<Line<'static>>,
    /// 内容宽度（列内左右各留 1 列后的值），与行内容是同一口径。
    pub width: usize,
}

/// 条目区（不含固定的标题行）的可见窗口 `[start, end)`。
pub(super) fn sidebar_window(
    rows_len: usize,
    height: usize,
    selected: usize,
    prev_start: usize,
) -> (usize, usize) {
    let items_len = rows_len.saturating_sub(1);
    if height == 0 || items_len == 0 {
        return (0, 0);
    }
    if rows_len <= height {
        return (0, items_len);
    }
    let item_max = height - 1;
    let sel = selected.min(items_len - 1);
    // 起点先夹进合法区间（内容变短 / 窗口变高后，旧起点可能越界）。
    let mut start = prev_start.min(items_len.saturating_sub(item_max));
    if sel < start {
        start = sel;
    } else if sel >= start.saturating_add(item_max) {
        start = (sel + 1).saturating_sub(item_max);
    }
    (start, (start + item_max).min(items_len))
}

/// 通用固定列渲染：内容窗口裁剪（含提示行）。内容行/光标/提示由 [`super::ws_sidebar`] 提供。
/// 列内左右各留 1 列空隙（对齐聊天区观感），行宽按 `width - 2` 截断。
pub(super) fn render_pane(
    frame: &mut Frame,
    area: Rect,
    content: SidebarRows,
    pal: &Palette,
    list_start: &mut usize,
) {
    if area.width <= 1 || area.height == 0 {
        return;
    }
    let SidebarRows {
        rows,
        cursor,
        hint,
        width,
    } = content;
    // 内容区与提示行矩形同样跑鼠标命中检测用的那份计算。
    let list_area = pane_list_area(area);
    // 窗口起点**粘住**：只在光标越出窗口时最小滚动，结果写回状态供命中检测读。
    let (start, end) = sidebar_window(rows.len(), list_area.height as usize, cursor, *list_start);
    *list_start = start;
    // 行是**移动**进去的：窗口覆盖全部时 `sidebar_view` 原样交还，不必整表复制一遍。
    let shown = sidebar_view(rows, start, end, width, pal);
    let list = Paragraph::new(shown).style(Style::new().fg(pal.fg));
    frame.render_widget(list, list_area);
    // 提示行占列内最后一行（高度不足 2 行时没有提示位）。内容自带样式：会话栏是静态
    // 键位，工作区栏在光标行被截断时会改成完整相对路径。
    if area.height >= 2
        && let Some(text) = hint
    {
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 1, area.width - 1, 1);
        frame.render_widget(Paragraph::new(text), hint_area);
    }
}

/// 会话栏条目行的**文本缓存**。
///
/// 为什么值得缓存：`sidebar_rows_visible` 每帧对**全部**会话各做一次
/// `format!("{mark} {title}")` + `truncate_display`（两次分配），而屏幕上只画得下二十来行；
/// 光标移动、滚动、甚至流式输出都不改这些文本 —— 它们在每帧被白建一遍。
///
/// 失效维度分两层：
/// - **结构**：宽度变了（截断点全变）或可见集合变了（过滤词改、会话增删）→ 整表重建；
/// - **单行**：源字段（id / title / `>` / `*`）变了 → 只重建那一行。
///
/// 因此每帧的常态成本是 O(n) 次短串比较（零分配）+ O(变化的行) 次分配。
#[derive(Debug, Default)]
pub(super) struct SessionRows {
    /// 建缓存时的内容宽度。
    width: usize,
    /// 建缓存时的可见集合（下标序列）。
    visible: Vec<usize>,
    /// 与 `visible` 一一对应的行文本（已按 `width` 截断）。
    texts: Vec<String>,
    /// 与 `texts` 一一对应的源快照。
    srcs: Vec<SessionSrc>,
}

/// 影响一行会话渲染的最小源字段。
///
/// 契约类型 `SessionListRow` 来自上游 crate（不加 `PartialEq`），所以自己留一份；
/// 只记渲染真正读到的四项，比较里不会误留「改了但看不出来」的字段。
#[derive(Debug, PartialEq)]
struct SessionSrc {
    id: String,
    title: String,
    /// `>`：这一行就是本窗口正在聊的会话。
    in_use: bool,
    /// `*`：serve 侧的活跃会话。
    active: bool,
}

impl SessionRows {
    /// 让缓存跟上当前状态，返回可直接渲染的行文本（与 `visible` 一一对应）。
    ///
    /// 收的是**具体字段**而不是 `&UiState`：调用方要把 `&mut st.session_rows` 借出来，
    /// 同时还要读 `st.sessions` / `st.conversation_id` / `st.active_session_id` ——
    /// 传整个 `&UiState` 就会被借用检查拦住（同一结构不能既可变又不可变）。
    pub(super) fn sync(
        &mut self,
        sessions: &[crate::serve::SessionListRow],
        conversation_id: Option<&str>,
        active_session_id: Option<&str>,
        width: usize,
        visible: &[usize],
    ) -> &[String] {
        let structural = self.width != width
            || self.visible.len() != visible.len()
            || self.visible.iter().zip(visible).any(|(a, b)| a != b);
        if structural {
            self.width = width;
            self.visible = visible.to_vec();
            self.texts.clear();
            self.srcs.clear();
            for &i in visible {
                let row = &sessions[i];
                let (in_use, active) = marks(row, conversation_id, active_session_id);
                self.texts.push(cell_text(row, in_use, active, width));
                self.srcs.push(SessionSrc {
                    id: row.id.clone(),
                    title: row.title.clone(),
                    in_use,
                    active,
                });
            }
            return &self.texts;
        }
        for (slot, &i) in self.visible.iter().enumerate() {
            let row = &sessions[i];
            let (in_use, active) = marks(row, conversation_id, active_session_id);
            // 先比再建：比较全是 &str 上的 `==`（零分配），只有真的变了才拼字符串。
            let stale = self.srcs[slot].id != row.id
                || self.srcs[slot].title != row.title
                || self.srcs[slot].in_use != in_use
                || self.srcs[slot].active != active;
            if !stale {
                continue;
            }
            self.texts[slot] = cell_text(row, in_use, active, width);
            self.srcs[slot] = SessionSrc {
                id: row.id.clone(),
                title: row.title.clone(),
                in_use,
                active,
            };
        }
        &self.texts
    }
}

/// 这一行是否就是本窗口正在聊的会话（行首 `>`）。
///
/// 与 `UiState::row_in_use` **同一实现**：侧栏缓存那边拿不到 `&UiState`（见
/// [`SessionRows::sync`] 的说明），所以把判定下沉成自由函数，两边都调它 ——
/// 「什么算 in_use」只该有一处定义。
pub(super) fn row_in_use(
    row: &crate::serve::SessionListRow,
    conversation_id: Option<&str>,
) -> bool {
    conversation_id.is_some_and(|c| row.server_conversation_id.as_deref() == Some(c))
}

/// 一行会话的两个标记位：`>`（本窗口在聊）与 `*`（serve 侧活跃）。
fn marks(
    row: &crate::serve::SessionListRow,
    conversation_id: Option<&str>,
    active_session_id: Option<&str>,
) -> (bool, bool) {
    let active = active_session_id == Some(row.id.as_str());
    (row_in_use(row, conversation_id), active)
}

/// 一行会话的渲染文本。**这是会话行文本的唯一定义**：缓存与「无条件重建」两条路都走它，
/// 所以两者不可能漂（对照测试因此断言的是「缓存 == 全量」而不是某段文字长什么样）。
pub(super) fn cell_text(
    row: &crate::serve::SessionListRow,
    in_use: bool,
    active: bool,
    width: usize,
) -> String {
    let mark = if in_use {
        ">"
    } else if active {
        "*"
    } else {
        " "
    };
    let title = if row.title.trim().is_empty() {
        "(untitled)"
    } else {
        row.title.trim()
    };
    super::render::truncate_display(&format!("{mark} {title}"), width)
}

#[cfg(test)]
mod tests {
    use super::sidebar_window;

    /// 窗口起点**粘住**：光标在窗口内移动时不动。
    ///
    /// 「双击打开错误文件」的根因回归：过去起点由光标反推，点一下窗口最上面
    /// 那行就会把整列往上顶，双击的第二下在新位置上重算 → 落到别的行。
    #[test]
    fn start_sticks_when_cursor_stays_inside() {
        // 101 行（标题 + 100 条），窗口 5 行 = 标题 + 4 条
        let (rows_len, height) = (101, 5);
        // 光标在底部：起点让第 40 条成为最后一条可见项
        let (start, _) = sidebar_window(rows_len, height, 40, 0);
        assert_eq!(start, 37, "40 + 1 - 4");

        // 点可视第 1 行（条目 37）与最后一行（条目 40）：起点都必须不变
        assert_eq!(sidebar_window(rows_len, height, start, start).0, start);
        assert_eq!(sidebar_window(rows_len, height, 40, start).0, start);
    }

    /// 只有光标真的越出窗口时才最小滚动。
    #[test]
    fn start_scrolls_minimally_on_overflow() {
        let (rows_len, height, start) = (101, 5, 37);
        // 向下越界一格 → 只滚一行
        assert_eq!(sidebar_window(rows_len, height, 41, start).0, 38);
        // 向上越界一格 → 光标变成新起点
        assert_eq!(sidebar_window(rows_len, height, 36, start).0, 36);
    }

    /// 内容变短 / 窗口变高后，旧起点越界要被夹回去，而不是留下空白。
    #[test]
    fn start_clamps_when_content_shrinks() {
        // 只有 3 条，窗口 5 行 → 全部可见
        assert_eq!(sidebar_window(4, 5, 0, 37), (0, 3));
        // 仍比窗口长但没这么长：光标取 8（sel=0 本就要求露出来，起点会归 0）
        assert_eq!(sidebar_window(11, 5, 8, 37).0, 6, "10 条 - 4 = 6");
    }
}
