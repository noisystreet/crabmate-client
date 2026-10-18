//! 状态行：左侧上下文摘要 + 右缘运行态指示。
//!
//! 从 `render` 拆出（`render.rs` 已到 920 行门禁上限）：这里是一组相对独立的纯函数，
//! 输入 `StatusInfo` + 宽度、输出单行（`Line`），便于单测。
//! spinner 帧表也放在这里——它是状态行、折叠思考行与进行中工具行共用的进度字形来源，
//! 统一一处保证「换任何一处都不破坏列宽」。
//!
//! **窄终端不截尾**：左区按优先级**整段丢弃**，而不是从字符串尾巴切。
//! 原实现用 `truncate_display` 砍尾部，先消失的正是队尾的 `conv` / `find`——
//! 搜索时匹配计数恰恰是状态栏上唯一还在变化的信息，砍掉等于搜索没有反馈。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::palette::Palette;
use super::render::{StatusInfo, truncate_display};

/// 生成中 spinner 帧（Braille 点阵，均 1 列宽，不破坏右对齐与截断计算）。
pub(super) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 左区分段的保留优先级：数值越小越先保留。
///
/// `find` 最高：它是搜索期间唯一还在变化的信息，丢掉等于没有反馈。
/// `serve` 次之（排障首先要看连的哪台机器），之后才是模型 / 模式这类「一进界面就知道」的项。
mod priority {
    pub(super) const FIND: u8 = 0;
    pub(super) const SERVE: u8 = 1;
    pub(super) const MODEL: u8 = 2;
    pub(super) const MODE: u8 = 3;
    pub(super) const ROLE: u8 = 4;
    pub(super) const CONV: u8 = 5;
    pub(super) const VIEW: u8 = 6;
}

/// 段间分隔符（极暗竖线，与 A6 的分隔线同色系）。
const SEPARATOR: &str = " │ ";
/// 分隔符显示宽。
const SEPARATOR_W: usize = 3;

/// 左区一个分段：`label` 走淡色、`value` 走正文色。
struct Segment {
    label: &'static str,
    value: String,
    priority: u8,
}

impl Segment {
    /// 分段的显示宽（label + 1 空格 + value；无 label 时只有 value）。
    fn width(&self) -> usize {
        let value_w = UnicodeWidthStr::width(self.value.as_str());
        if self.label.is_empty() {
            value_w
        } else {
            UnicodeWidthStr::width(self.label) + 1 + value_w
        }
    }
}

/// `serve` 段的 label 显示宽（`serve` + 1 空格），用于判断值能占多少列。
const SERVE_LABEL_W: usize = 6;

/// 把 base URL 削成 `host:port`（去 scheme、去路径）。
fn host_port(api_base: &str) -> String {
    let base = api_base.trim();
    let without_scheme = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or(base);
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .trim();
    if host.is_empty() {
        base.to_string()
    } else {
        host.to_string()
    }
}

/// `serve` 段的取值：预算够就显示**完整** base，不够才退到 `host:port`。
///
/// 完整 base 往往是最长的一项，但它是 TUI 里唯一能看到全量 URL 的地方——`/status` 只是
/// 触发一次刷新、不打印 base。所以不能无条件削成 `host:port`（那样带路径前缀的自建 serve
/// 就只剩机器名），只在真的放不下时才削。
fn serve_value(api_base: &str, segment_budget: usize) -> String {
    let full = api_base.trim();
    if UnicodeWidthStr::width(full) <= segment_budget.saturating_sub(SERVE_LABEL_W) {
        full.to_string()
    } else {
        host_port(full)
    }
}

/// 左区所有候选分段（顺序即显示顺序，与优先级无关）。
fn left_segments(info: &StatusInfo, conv: Option<&str>, budget: usize) -> Vec<Segment> {
    let mut out = vec![Segment {
        label: "serve",
        value: serve_value(&info.api_base, budget),
        priority: priority::SERVE,
    }];
    if let Some(v) = info.model.as_deref() {
        out.push(Segment {
            label: "model",
            value: v.to_string(),
            priority: priority::MODEL,
        });
    }
    if let Some(v) = info.role.as_deref() {
        out.push(Segment {
            label: "role",
            value: v.to_string(),
            priority: priority::ROLE,
        });
    }
    if let Some(v) = info.mode.as_deref() {
        out.push(Segment {
            label: "mode",
            value: v.to_string(),
            priority: priority::MODE,
        });
    }
    out.push(Segment {
        label: "conv",
        value: conv.unwrap_or("(new)").to_string(),
        priority: priority::CONV,
    });
    if let Some(term) = info.search_term.as_deref() {
        out.push(Segment {
            label: "find",
            value: match info.search_total {
                0 => format!("「{term}」无匹配"),
                n => format!("「{term}」{n}处"),
            },
            priority: priority::FIND,
        });
    }
    if info.view_offset > 0 {
        out.push(Segment {
            label: "",
            value: format!("↑{}", info.view_offset),
            priority: priority::VIEW,
        });
    }
    out
}

/// 按优先级挑选能装进 `budget` 的分段，返回**显示顺序**的下标。
///
/// **严格优先级**：按优先级从高到低尝试，**一旦某个装不下就收手**，不允许更低的段"插队"。
/// 否则会排成「`model` 在、`serve` 没了」——把一个次要项顶到排障最需要的那项前面。
/// 剩下的列留白，用户能看出「这里本来还有东西但放不下」。
///
/// 唯一例外是「最高优先级也装不下」：让它独占预算、由 [`push_segment`] 按宽截断，
/// 而不是干脆什么都不显示。
fn fit_segments(segs: &[Segment], budget: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by_key(|&i| (segs[i].priority, i));
    let mut chosen: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for i in order {
        let need = segs[i].width() + if chosen.is_empty() { 0 } else { SEPARATOR_W };
        if used + need <= budget {
            used += need;
            chosen.push(i);
            continue;
        }
        if chosen.is_empty() {
            chosen.push(i);
        }
        break;
    }
    chosen.sort_unstable();
    chosen
}

/// 追加一个分段，返回它实际占用的显示宽。
///
/// 宽裕时原样输出（label 淡色 + value 正文色）；装不下（只有「独占预算」的退化路径会走到）
/// 时先保住 label，再把 value 截到剩余宽度——绝不把超宽文本交给 `Paragraph` 去裁尾。
fn push_segment(
    spans: &mut Vec<Span<'static>>,
    seg: &Segment,
    budget: usize,
    pal: &Palette,
) -> usize {
    if seg.width() <= budget {
        if !seg.label.is_empty() {
            spans.push(Span::styled(seg.label, Style::new().fg(pal.muted)));
            spans.push(Span::styled(" ", Style::new()));
        }
        spans.push(Span::styled(seg.value.clone(), Style::new().fg(pal.fg)));
        return seg.width();
    }
    let label_w = if seg.label.is_empty() {
        0
    } else {
        UnicodeWidthStr::width(seg.label) + 1
    };
    let keep_label = label_w > 0 && label_w < budget;
    if keep_label {
        spans.push(Span::styled(seg.label, Style::new().fg(pal.muted)));
        spans.push(Span::styled(" ", Style::new()));
    }
    let text = truncate_display(&seg.value, budget.saturating_sub(label_w));
    let text_w = UnicodeWidthStr::width(text.as_str());
    spans.push(Span::styled(text, Style::new().fg(pal.fg)));
    if keep_label { label_w + text_w } else { text_w }
}

/// 状态行右侧运行态指示（空闲 / 生成中 spinner / 取消中）。
/// 只有生成中才换帧：空闲与取消中保持静态，帧号相同则整行不变，
/// 不会让终端每帧重写这一行。
fn status_running_text(info: &StatusInfo) -> String {
    match (info.running, info.cancel_sent) {
        (true, true) => "✳ 取消中".to_string(),
        (true, false) => format!(
            "{} 生成中",
            SPINNER_FRAMES[info.spinner % SPINNER_FRAMES.len()]
        ),
        (false, _) => "○ 空闲".to_string(),
    }
}

/// 右缘运行态样式：生成中/取消中用强调色，空闲用淡色（都加粗，保证右缘可扫读）。
fn running_style(info: &StatusInfo, pal: &Palette) -> Style {
    let fg = if info.running { pal.accent } else { pal.muted };
    Style::new().fg(fg).add_modifier(Modifier::BOLD)
}

/// 状态行（单行 span）：左侧上下文按优先级放进 `width`，右缘贴边显示运行态。
///
/// 调用方**不要**再给 `Paragraph` 套整行 `fg + BOLD`：base style 的修饰位会盖到 label 上，
/// 淡化后的标签就白做了。
pub(super) fn status_line(
    info: &StatusInfo,
    conv: Option<&str>,
    width: usize,
    pal: &Palette,
) -> Line<'static> {
    let right = status_running_text(info);
    let right_w = UnicodeWidthStr::width(right.as_str());
    let budget = width.saturating_sub(right_w);
    let segs = left_segments(info, conv, budget);
    let chosen = fit_segments(&segs, budget);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (n, &i) in chosen.iter().enumerate() {
        if n > 0 {
            spans.push(Span::styled(SEPARATOR, Style::new().fg(pal.border)));
            used += SEPARATOR_W;
        }
        used += push_segment(&mut spans, &segs[i], budget.saturating_sub(used), pal);
    }
    let pad = width.saturating_sub(used + right_w);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::new()));
    }
    spans.push(Span::styled(right, running_style(info, pal)));
    Line::from(spans)
}

/// 纯文本形态：只给测试用（生产路径一律走 [`status_line`] 的 span）。
///
/// `#[cfg(test)]` 是刻意的：留着它当生产 API 就会有人拿它拼字符串，样式又退回单色。
#[cfg(test)]
pub(super) fn status_text(info: &StatusInfo, conv: Option<&str>, width: usize) -> String {
    status_line(info, conv, width, &Palette::default()).to_string()
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    fn base_info(api_base: &str) -> StatusInfo {
        StatusInfo {
            api_base: api_base.into(),
            model: None,
            role: None,
            mode: None,
            running: false,
            cancel_sent: false,
            spinner: 0,
            view_offset: 0,
            search_term: None,
            search_total: 0,
        }
    }

    #[test]
    fn host_port_strips_scheme_and_path() {
        assert_eq!(host_port("http://127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(
            host_port("https://cloud.example.com/crabmate/base"),
            "cloud.example.com"
        );
        assert_eq!(host_port("  http://x  "), "x");
        // 没有 scheme 时原样保留首段，不做猜测。
        assert_eq!(host_port("localhost:9000"), "localhost:9000");
    }

    #[test]
    fn serve_shows_full_base_when_it_fits_and_shortens_only_when_narrow() {
        let info = base_info("https://cloud.example.com/crabmate");
        // 宽终端：完整 base（含路径前缀）可见—— TUI 里没有第二个入口能看全量 URL。
        let wide = status_text(&info, None, 200);
        assert!(
            wide.contains("serve https://cloud.example.com/crabmate"),
            "{wide}"
        );
        // 窄终端：退到 host:port，否则一整行只够 serve 一项。
        let narrow = status_text(&info, None, 40);
        assert!(narrow.contains("serve cloud.example.com"), "{narrow}");
        assert!(!narrow.contains("/crabmate"), "{narrow}");
    }

    #[test]
    fn narrow_bar_keeps_only_the_top_priority_segment() {
        let mut info = base_info("http://127.0.0.1:8080");
        info.model = Some("gpt-x".into());
        info.mode = Some("plan".into());
        info.search_term = Some("ab".into());
        info.search_total = 2;
        // 40 列：find(14) 之后连 `serve`(27) 都放不下 → **严格优先级**到此收手，
        // 不允许 `model` 之类的次要项插队（否则会出现「model 在、serve 没了」）。
        let text = status_text(&info, Some("conv-id-1234"), 40);
        assert!(text.contains("find"), "搜索反馈必须常驻：{text}");
        assert!(!text.contains("serve"), "serve 放不下就整体不收：{text}");
        assert!(!text.contains("model"), "更低的 model 更不该插队：{text}");
        assert!(!text.contains("conv"), "conv 应被丢弃：{text}");
    }

    #[test]
    fn serve_outranks_model_when_both_compete() {
        let mut info = base_info("http://127.0.0.1:8080");
        info.model = Some("gpt-x".into());
        info.search_term = Some("ab".into());
        info.search_total = 2;
        // 60 列：find(14) + serve(27) + 分隔(3) = 44 装得下，`model` 就轮不上了。
        let text = status_text(&info, None, 60);
        assert!(text.contains("find"), "{text}");
        assert!(text.contains("serve"), "serve 优先于 model：{text}");
        assert!(!text.contains("model"), "model 应被丢弃：{text}");
    }

    #[test]
    fn dropped_segments_never_truncate_the_tail() {
        let mut info = base_info("http://127.0.0.1:8080");
        info.model = Some("gpt-x".into());
        info.search_term = Some("needle".into());
        info.search_total = 3;
        // 20 列：只够 find 独占（它自己也超宽，按宽截断），`conv` / `model` 整段丢弃，
        // 而不是被砍成半截留在行尾。
        let text = status_text(&info, Some("c1"), 20);
        assert!(!text.contains("conv"), "丢弃是整段丢弃：{text}");
        assert!(!text.contains("model"), "丢弃是整段丢弃：{text}");
        assert!(text.starts_with("find"), "留下的应是最高优先级段：{text}");
        assert_eq!(UnicodeWidthStr::width(text.as_str()), 20, "整行铺满");
    }

    #[test]
    fn labels_use_muted_and_values_use_fg() {
        let pal = Palette::default();
        let info = base_info("http://127.0.0.1:8080");
        let line = status_line(&info, Some("c1"), 200, &pal);
        let spans = &line.spans;
        assert_eq!(spans[0].content, "serve");
        assert_eq!(spans[0].style.fg, Some(pal.muted), "label 走淡色");
        assert_eq!(spans[1].content, " ");
        assert_eq!(spans[2].style.fg, Some(pal.fg), "值走正文色");
        // 分隔符必须是极暗竖线，否则窄终端里像多了一列正文。
        assert!(
            spans.iter().any(|s| s.content == SEPARATOR),
            "缺分隔符：{spans:?}"
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == SEPARATOR)
                .map(|s| s.style.fg),
            Some(Some(pal.border))
        );
    }

    #[test]
    fn running_indicator_right_aligned_across_states() {
        let pal = Palette::default();
        let mut info = base_info("http://x");
        for (running, cancel, want) in [
            (false, false, "○ 空闲"),
            (true, false, "⠋ 生成中"),
            (true, true, "✳ 取消中"),
        ] {
            info.running = running;
            info.cancel_sent = cancel;
            let line = status_line(&info, None, 40, &pal);
            let text = line.to_string();
            assert!(text.ends_with(want), "{text} 应以 {want} 结尾");
            assert_eq!(UnicodeWidthStr::width(text.as_str()), 40, "整行铺满");
        }
    }

    #[test]
    fn mono_palette_drops_every_color() {
        // 无色模式：分段取舍与文本不变，但不再有任何一个 span 指定彩色
        // （`Palette::mono()` 的所有槽都是 `Reset`）。
        let info = base_info("http://127.0.0.1:8080");
        let line = status_line(&info, Some("c1"), 80, &Palette::mono());
        let text = line.to_string();
        assert!(text.contains("serve"), "{text}");
        assert!(text.contains("conv c1"), "{text}");
        assert!(
            line.spans
                .iter()
                .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset)),
            "无色模式下不该有彩色 span：{:?}",
            line.spans
        );
    }
}
