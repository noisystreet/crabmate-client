//! 正文链接：`Ctrl+单击` 命中表的 URL 校验、拉起系统浏览器、远端会话下的降级。
//!
//! **为什么这一步必须自己接**：终端会把屏幕上字面可见的 `http(s)://…` 自动认成超链接
//! （画下划线、悬停可见），但本程序自己开了鼠标上报（`shell.rs` 的 `MOUSE_ON`，
//! 1000/1002/1006）。那一下点击会被终端送进本进程，终端自己的链接处理器**收不到**；
//! 而事件进了 `mouse.rs` 就没有「交还终端」的出口。于是症状是
//! 「终端明明识别成链接了，点下去毫无反应」。
//!
//! 同一条推论也决定了**裸 URL 必须由我们登记**：既然终端自己的处理器收不到点击，
//! 「裸地址留给终端识别」就等于让它彻底不可点。`md_links::push_plain_with_urls`
//! 因此把行内文本里的 `http(s)://…` 按 `[url](url)` 登记 —— 显示文本就是地址本身，
//! 屏幕上逐字不变，只是多了一个热区。
//!
//! 交互定成 **Ctrl+单击**：聊天区单击的本意几乎总是「回到输入框」（见 `mouse.rs` 的
//! `click_body` 注释），若把打开浏览器挂在单击上，每次想打字都会弹出浏览器。
//!
//! 命中区间由渲染层回写（`render_body` → `UiState::body_links`），与 `body_rows_log`
//! 同一条规矩：折行 / 裁剪这类排版只在渲染层算一次，鼠标侧只做减法 ——
//! 在这里重算一遍，就是「点到的列与看到的列错开」的唯一来源。

use std::process::Command;

use super::clipboard;
use super::layout::contains;
use super::state::{Focus, LineKind};

/// URL 长度上限。超长的多半是数据 URI 或恶意构造：直接拒掉，不做截断 ——
/// 截断后的 URL 指向的是别的东西。
pub(crate) const MAX_URL_LEN: usize = 2048;

/// 一个可点击区间：某条物理行上 `[start_col, end_col)` 这段显示列对应的链接。
///
/// 列**相对正文区的左边界**（0-based），与 `body_area.x` 对齐：渲染层回写时就把前缀
/// 宽度算进去了，鼠标侧只需一次减法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowLink {
    pub start_col: u16,
    pub end_col: u16,
    pub url: String,
}

/// 在一行的命中区间里找 `col` 落在哪一条上。区间左闭右开：紧贴的下一条链接不会
/// 抢走上一条的末列，光标停在两段之间时也不会命中。
#[must_use]
pub(crate) fn hit(links: &[RowLink], col: u16) -> Option<&str> {
    links
        .iter()
        .find(|l| (l.start_col..l.end_col).contains(&col))
        .map(|l| l.url.as_str())
}

/// 校验并规范化 URL：只放行 `http` / `https`，去掉首尾空白。
///
/// **拒绝**而不是「清洗后放行」：含控制字符的 URL 打到 transcript 上或交给浏览器，
/// 都已经不是用户想打开的那条了（控制字符还能伪造后续输出）。
#[must_use]
pub(crate) fn sanitize(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN || url.chars().any(char::is_control) {
        return None;
    }
    has_web_scheme(url).then(|| url.to_string())
}

/// scheme 大小写不敏感（`HTTPS://` 一样是合法地址）。
fn has_web_scheme(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 打开动作的决策结果。
///
/// 拆成纯函数是为了能单测「远端 / 无工具」两条需要改环境变量的分支，而不必在测试里
/// 真的去拉起浏览器。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// 本地能拉起浏览器。
    Launch(&'static str),
    /// 只能把链接放进剪贴板，附带一句「为什么没直接打开」。
    Copy(&'static str),
}

/// 决定这次点击该拉起浏览器还是只能复制。
#[must_use]
pub(crate) fn plan(is_ssh: bool, tool: Option<&'static str>) -> Plan {
    if is_ssh {
        // 远端会话里 `xdg-open` 指向的是那台没有显示器的机器：要么报错，要么拉起一个
        // 用户根本看不见的浏览器。剪贴板是唯一真正有用的归宿（OSC52 会送回本地终端）。
        return Plan::Copy("远端会话不打开本地浏览器");
    }
    match tool {
        Some(name) => Plan::Launch(name),
        None => Plan::Copy("没找到 xdg-open / gio / sensible-browser / open"),
    }
}

/// 打开器：名字 + 固定参数（URL 追加在末尾）。
const OPENERS: [(&str, &[&str]); 4] = [
    ("xdg-open", &[]),
    ("gio", &["open"]),
    ("sensible-browser", &[]),
    ("open", &[]),
];

/// 按 PATH 找第一个可用的打开器。
///
/// 只 `is_file()`，不试运行：探测本身不该拉起图形进程。
fn opener_tool() -> Option<&'static str> {
    let path = std::env::var_os("PATH")?;
    OPENERS
        .into_iter()
        .find(|(name, _)| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .map(|(name, _)| name)
}

/// 拉起打开器，**不等它退出**。
///
/// 图形浏览器常常常驻：`wait` 会把事件循环挂到用户关掉浏览器为止。但也不能放着不管 ——
/// 总要有人收尸，否则子进程退出后留成僵尸。
fn launch(tool: &str, url: &str) -> Result<(), String> {
    let args = OPENERS
        .iter()
        .find(|(name, _)| *name == tool)
        .map_or(&[][..], |(_, args)| *args);
    let child = Command::new(tool)
        .args(args)
        .arg(url)
        .spawn()
        .map_err(|e| format!("拉起 {tool} 失败：{e}"))?;
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// `Ctrl+单击` 的入口：打开链接，返回一行写进 transcript 的结果。
///
/// 失败一律回落到剪贴板并给出明确文案 —— 静默失败在 TUI 里就表现为「点了没反应」，
/// 而这正是本次要修的那个症状。
pub(crate) fn open(raw: &str) -> String {
    let Some(url) = sanitize(raw) else {
        return "链接没打开：只支持 http / https 地址".to_string();
    };
    match plan(clipboard::in_ssh_session(), opener_tool()) {
        Plan::Launch(tool) => match launch(tool, &url) {
            Ok(()) => format!("已在浏览器打开：{url}"),
            Err(err) => copy_note(&url, &err),
        },
        Plan::Copy(reason) => copy_note(&url, reason),
    }
}

/// 把链接放进剪贴板并生成提示；`lead` 说明「为什么没直接打开」。
fn copy_note(url: &str, lead: &str) -> String {
    match clipboard::deliver(url) {
        Ok(route) => format!("{lead}；已复制链接（经 {}）：{url}", route.label()),
        Err(err) => format!("{lead}；复制链接也失败：{err}。链接：{url}"),
    }
}

impl super::LaneApp<'_> {
    /// 正文区 `Ctrl+单击`：命中链接热区就打开。返回 `true` = 已消费。
    ///
    /// 命中表由 `render_body` 每帧回写（`UiState::body_links`），这里只做减法 ——
    /// 自己再折一次行，就是「点到的列与看到的列错开」的来源。
    ///
    /// 无论成功失败都写一行回执：静默失败在 TUI 里就是「点了没反应」，
    /// 而这正是这条链路原本的症状。
    pub(super) fn click_link(&mut self, column: u16, row: u16) -> bool {
        let Some(area) = self.st.body_area else {
            return false;
        };
        if !contains(area, column, row) {
            return false;
        }
        // 先摘出 URL 再动 `st`：`hit` 借的是 `body_links`，而下面要可变借用它推提示行。
        let url = self
            .st
            .body_links
            .get((row - area.y) as usize)
            .and_then(|links| hit(links, column - area.x))
            .map(str::to_string);
        let Some(url) = url else {
            return false;
        };
        let notice = open(&url);
        // 键路跟着焦点一起回输入框：点完链接继续打字是最自然的下一步。
        self.st.focus = Focus::Input;
        self.st.focus_by_pointer = false;
        self.st.confirm_delete = None;
        self.st.push_line(LineKind::System, &notice);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(start_col: u16, end_col: u16, url: &str) -> RowLink {
        RowLink {
            start_col,
            end_col,
            url: url.to_string(),
        }
    }

    #[test]
    fn sanitize_keeps_web_urls_and_trims() {
        assert_eq!(
            sanitize("  https://example.com/a?b=1  ").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(
            sanitize("http://127.0.0.1:8080").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        // scheme 大小写不敏感
        assert_eq!(sanitize("HTTPS://x").as_deref(), Some("HTTPS://x"));
    }

    #[test]
    fn sanitize_rejects_non_web_schemes() {
        // 这几种交给浏览器就是执行 / 读本地文件
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "file:///etc/passwd",
        ] {
            assert_eq!(sanitize(bad), None, "{bad} 不该放行");
        }
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize("   "), None);
        assert_eq!(sanitize("example.com"), None, "没有 scheme 不算链接");
    }

    #[test]
    fn sanitize_rejects_control_chars_and_oversize() {
        assert_eq!(sanitize("https://x\u{7}evil"), None);
        assert_eq!(sanitize("https://x\nrm -rf /"), None);
        let long = format!("https://x/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(sanitize(&long), None, "超长直接拒，不截断");
    }

    #[test]
    fn hit_is_half_open_on_the_right() {
        let links = vec![link(2, 6, "https://a"), link(6, 9, "https://b")];
        assert_eq!(hit(&links, 2), Some("https://a"));
        assert_eq!(hit(&links, 5), Some("https://a"));
        // 末列（end_col）属于下一条，不是上一条
        assert_eq!(hit(&links, 6), Some("https://b"));
        assert_eq!(hit(&links, 8), Some("https://b"));
        assert_eq!(hit(&links, 9), None);
        assert_eq!(hit(&links, 0), None);
        assert_eq!(hit(&[], 0), None);
    }

    #[test]
    fn plan_prefers_copy_on_ssh_even_with_a_tool() {
        // 远端会话里本地有工具也不能用：拉起来的是「那台机器」的浏览器
        assert_eq!(
            plan(true, Some("xdg-open")),
            Plan::Copy("远端会话不打开本地浏览器")
        );
        assert_eq!(plan(false, Some("xdg-open")), Plan::Launch("xdg-open"));
        assert!(matches!(plan(false, None), Plan::Copy(_)));
    }
}
