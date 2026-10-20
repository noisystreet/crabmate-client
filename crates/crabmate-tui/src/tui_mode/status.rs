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

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
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

/// 状态行右缘的运行态。
enum RunningKind {
    Idle,
    /// 生成中。`phase` 是方形亮块所在的格：方块随拍右移，六格一圈。
    /// `None` 表示整条常亮——动效关闭（`CM_TUI_REDUCED_MOTION`）时用
    /// 「全亮且静止」代替平移，仍然一眼可辨。
    Generating {
        phase: Option<usize>,
    },
    Cancelling,
}

/// 仪器条的格数。固定 6 格：换格不改宽度，状态行因此不会随帧抖动。
const BAR_CELLS: usize = 6;

/// 几拍基础帧换一次相位。`spinner.rs` 每 120ms 推一拍，这里 ×2 → **240ms 交替一次**。
///
/// 只在这个算式里除以倍数，而不是去调 `SPINNER_INTERVAL`：那个间隔还被折叠思考行与
/// 进行中工具行的 Braille 转圈共用，全局放慢会连它们一起改动（120ms 对 Braille 是对的）。
const BAR_TICKS_PER_CELL: usize = 2;

/// 仪器条占宽 = 格数。熄灭格用小号字形（见 [`BAR_CELL_DIM`]）自带留白，
/// 不再额外插格间空格——插空格会让亮点每步跨 2 列，看起来像跳格。
const BAR_W: usize = BAR_CELLS;

/// 熄灭的格：空心方块 `□`（Geometric Shapes）。
///
/// 与点亮的 [`BAR_CELL_LIT`] **同形不同填充**：六个格位任何时候都是六个正方形，
/// 只是空心的那个没点亮。这样读起来是「一排灯窗，其中一盏亮着在走」，而不是
/// 「一颗亮点在空条上跳」——六格的位置恒定，位移才有一把可比的尺子。
///
/// 换成空心方还有个副作用是好的：`·` 与 `■` 的差别主要靠**面积**（小点 vs 满方块），
/// 远看时暗格几乎看不见，画面塌成「只有一个亮点」；`□` 与 `■` 面积相同、只差填充，
/// 远处也留得住六个格位。
///
/// 亮灭仍然靠**字形差**而不是只靠颜色：`mono` / 8 色下颜色全丢时，
/// 镂空与实心依旧分得开。
///
/// 宽度风险与 `○ 空闲` 同级（East-Asian-Ambiguous）：终端若按全角渲染会整行错位——
/// 这是 `○ 空闲` 早就带着的风险，不是本轮新引入。
const BAR_CELL_DIM: &str = "□";

/// 点亮的格：实心方块 `■`（Geometric Shapes）。
///
/// 选方块、不选圆点：`●` 太小、留白又大，单颗移动看着像仪表上一颗将熄的指示灯；
/// 方块有实心面积，作为「色块」才立得住。而且它和熄灭格的 `□` 同形，
/// 亮灭读作「同一扇窗的两种状态」，不是两个不相干的符号。
const BAR_CELL_LIT: &str = "■";

/// 亮块长度：**一颗**。
///
/// 不取「连续几格」是有原因的——终端里相邻实心方块之间没有可靠的字形间隔，`█` 与 `■`
/// 在不少字体里都接近满格，三颗连排必然粘成一根长条，那正是要避免的观感；靠字形自带留白
/// 来分隔是**赌字体**，换个终端就失效。单颗方块不存在粘连问题，而且它是条内唯一的亮特征，
/// 眼睛直接跟得住，行进感反而最强。
///
/// 想换回多格只改这个常量：`is_lit` 用的是通用窗口判定（`(i + BAR_CELLS - phase) % BAR_CELLS < BAR_BLOCK`）。
const BAR_BLOCK: usize = 1;

/// 右缘文案。生成中**不写字**：动效本身就在表达「在跑」，
/// 而 `○ 空闲` / `✳ 取消中` 是静止的，静止的东西才需要文字解释。
const IDLE_TEXT: &str = "○ 空闲";
const CANCELLING_TEXT: &str = "✳ 取消中";

/// `✳ 取消中` 的显示宽：8 列 5 字符。`UnicodeWidthStr::width` 不是 const fn，故写死；
/// 若它和实际宽度不再相等，`running_indicator_right_aligned_across_states` 会因整行宽度
/// 对不上而失败。
const CANCELLING_W: usize = 8;

/// 右缘固定宽度：三态一律右对齐到这一列数。
///
/// 统一宽度是为了**状态切换时左区不跳列**：带「生成中」文案时右缘是 13 列、空闲是 6 列，
/// 每回合开始 / 结束，左区（`serve` / `find`）都会整体平移一次。
///
/// 取三态里最宽的那个：`✳ 取消中` 恒为 [`CANCELLING_W`] 列、仪器条为 [`BAR_W`] 列，
/// 另两态前导补空即可，不必改动任何可见文案。
const RIGHT_W: usize = if BAR_W > CANCELLING_W {
    BAR_W
} else {
    CANCELLING_W
};

/// 右缘内容自身的显示宽（不含为对齐补的前导空格）。
fn running_content_w(kind: &RunningKind) -> usize {
    match kind {
        RunningKind::Idle => UnicodeWidthStr::width(IDLE_TEXT),
        RunningKind::Cancelling => UnicodeWidthStr::width(CANCELLING_TEXT),
        RunningKind::Generating { .. } => BAR_W,
    }
}

/// 当前运行态。`reduced_motion` 时整条常亮、不扫描——动效可关是硬约束（录屏 / 远程终端 /
/// 屏幕阅读器场景），但右缘仍要能一眼看出在生成，所以不能干脆不画。
fn running_kind(info: &StatusInfo, reduced_motion: bool) -> RunningKind {
    match (info.running, info.cancel_sent) {
        (true, false) => RunningKind::Generating {
            // 六个相位：亮块每拍右移一格，走满一圈回绕。
            phase: (!reduced_motion).then_some((info.spinner / BAR_TICKS_PER_CELL) % BAR_CELLS),
        },
        (true, true) => RunningKind::Cancelling,
        (false, _) => RunningKind::Idle,
    }
}

/// 点亮的格：有彩走强调色 + 加粗；无色模式退回反色块——`mono` 下所有槽都是 `Reset`，
/// 不换修饰位就完全看不出哪一格在动。反色只用在这一格上（大面积 `REVERSED` 仍是禁止项）。
fn bar_lit_style(pal: &Palette) -> Style {
    if pal.mono {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new().fg(pal.accent).add_modifier(Modifier::BOLD)
    }
}

/// 熄灭的格：次要文字色，像仪表上没点亮的灯位。**刻意不用最暗的 `border`**：
/// 空心圆的笔画比块字符细，配极暗色会彻底消失；8 色降级时 `border` 还会塌成黑、
/// 与 `chrome_bg` 同色，六格一起不见。亮灭本来就有形状差（`○` / `●`），
/// 不必再靠暗色压掉灯位——那等于把形状带来的信息丢掉。
fn bar_dim_style(pal: &Palette) -> Style {
    if pal.mono {
        Style::new().add_modifier(Modifier::DIM)
    } else {
        Style::new().fg(pal.muted)
    }
}

/// 追加右缘运行态（内容右对齐到 [`RIGHT_W`]）。
fn push_running(spans: &mut Vec<Span<'static>>, kind: &RunningKind, pal: &Palette) {
    // 三态内容宽不一（空闲 6 / 生成 6 / 取消 8）：前导补空让外沿始终贴右，
    // 状态切换时左区不会整体平移。
    let pad = RIGHT_W.saturating_sub(running_content_w(kind));
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), Style::new()));
    }
    let bold = |fg: Color| Style::new().fg(fg).add_modifier(Modifier::BOLD);
    match kind {
        RunningKind::Idle => spans.push(Span::styled(IDLE_TEXT, bold(pal.muted))),
        RunningKind::Cancelling => spans.push(Span::styled(CANCELLING_TEXT, bold(pal.accent))),
        RunningKind::Generating { phase } => {
            for i in 0..BAR_CELLS {
                // 方形亮块沿条右移：条内只有这一个亮特征，方向不会被误读。
                // 「隔一亮一」不行——那种图案只有两个相位，平移一格就等于整体反转，
                // 眼睛只会看到闪烁（周期 2 < 位移可分辨的尺度）。
                let is_lit = phase.is_none_or(|p| (i + BAR_CELLS - p) % BAR_CELLS < BAR_BLOCK);
                let (cell, style) = if is_lit {
                    (BAR_CELL_LIT, bar_lit_style(pal))
                } else {
                    (BAR_CELL_DIM, bar_dim_style(pal))
                };
                spans.push(Span::styled(cell, style));
            }
        }
    }
}

/// `CM_TUI_REDUCED_MOTION=1`：动效退回静态。与 `NO_COLOR` 同理——录屏、远程终端、
/// 屏幕阅读器场景下这类效果是干扰。
///
/// 进程内只读一次环境变量：本函数在渲染热路径上（每帧 2–50ms 一次），不该每帧分配字符串。
fn reduced_motion() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("CM_TUI_REDUCED_MOTION").is_ok_and(|v| {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "0" && v != "false"
        })
    })
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
    status_line_with(info, conv, width, pal, reduced_motion())
}

/// [`status_line`] 的可注入版本：单测要能强制 reduced motion，不依赖真实环境变量
/// （与 `Palette::resolve_with` 同一手法）。
fn status_line_with(
    info: &StatusInfo,
    conv: Option<&str>,
    width: usize,
    pal: &Palette,
    reduced_motion: bool,
) -> Line<'static> {
    let kind = running_kind(info, reduced_motion);
    // 右缘恒占 `RIGHT_W` 列：预算不随状态变，切换时左区不跳。
    let right_w = RIGHT_W;
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
    push_running(&mut spans, &kind, pal);
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

    /// 宽终端下每一段都在：本地 override 的 `*` 标记跟着值一起显示。
    #[test]
    fn wide_bar_lists_every_segment() {
        let mut info = base_info("http://127.0.0.1:8080");
        info.model = Some("gpt-x*".into());
        info.role = Some("coder".into());
        info.mode = Some("plan".into());
        let text = status_text(&info, Some("c1"), 200);
        // 宽终端预算够：`serve` 显示完整 base（TUI 里没有第二个入口能看全量 URL）。
        assert!(text.contains("serve http://127.0.0.1:8080"), "{text}");
        assert!(text.contains("model gpt-x*"), "{text}");
        assert!(text.contains("role coder"), "{text}");
        assert!(text.contains("mode plan"), "{text}");
        assert!(text.contains("conv c1"), "{text}");
        assert!(text.contains("○ 空闲"), "{text}");
    }

    #[test]
    fn serve_default_model_shows_up_without_a_mode() {
        let mut info = base_info("http://x");
        info.model = Some("deepseek".into());
        let text = status_text(&info, None, 200);
        assert!(text.contains("model deepseek"), "{text}");
        // 未设置的值不该凭空出现（`model ` 里没有 `mode ` 子串）。
        assert!(!text.contains("mode "), "{text}");
    }

    #[test]
    fn search_and_view_back_segments_show_up() {
        let mut info = base_info("http://x");
        info.view_offset = 12;
        info.search_term = Some("grep".into());
        info.search_total = 2;
        let text = status_text(&info, None, 200);
        assert!(text.contains("find 「grep」"), "{text}");
        assert!(text.contains("↑12"), "{text}");
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

    /// 生成中右缘的字形 span。
    ///
    /// 取尾部而不是固定下标：左区选了几段、右缘前置补了几个空格都不影响这里。
    /// 只挑字形 span（不按固定步长跳），这样条里有没有格间空格都对。
    fn bar_cells<'a>(line: &'a Line<'static>) -> Vec<&'a Span<'static>> {
        let tail = &line.spans[line.spans.len() - BAR_W..];
        tail.iter()
            .filter(|s| s.content.as_ref() == BAR_CELL_DIM || s.content.as_ref() == BAR_CELL_LIT)
            .collect()
    }

    /// 字形常量里唯一的那个字符（字形都是单字符）。
    fn glyph_of(cell: &str) -> char {
        cell.chars().next().expect("字形不能为空")
    }

    fn dim_ch() -> char {
        glyph_of(BAR_CELL_DIM)
    }

    fn lit_ch() -> char {
        glyph_of(BAR_CELL_LIT)
    }

    /// 行尾仪器条的 6 个字形（跳过分隔空格）。
    ///
    /// 断言形态时不能用字面串：`●` 的位置随帧号变，只有「字符合法 + 恰好一个实心」
    /// 才是稳定不变式。
    fn bar_glyphs(text: &str) -> Vec<char> {
        text.chars()
            .rev()
            .take(BAR_W)
            .filter(|c| *c != ' ')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// 取行的前 `width` 列（按显示列切，不按字符切——`✳ 取消中` 是 8 列 5 字符）。
    fn head_cols(text: &str, width: usize) -> String {
        use unicode_width::UnicodeWidthChar;
        let mut acc = 0usize;
        let mut out = String::new();
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if acc + w > width {
                break;
            }
            acc += w;
            out.push(ch);
        }
        out
    }

    #[test]
    fn running_indicator_right_aligned_across_states() {
        let pal = Palette::default();
        let mut info = base_info("http://x");
        info.model = Some("gpt-x".into());
        let mut head: Option<String> = None;
        for (running, cancel, want) in [
            (false, false, Some("○ 空闲")),
            (true, false, None),
            (true, true, Some("✳ 取消中")),
        ] {
            info.running = running;
            info.cancel_sent = cancel;
            let line = status_line(&info, None, 40, &pal);
            let text = line.to_string();
            match want {
                Some(w) => assert!(text.ends_with(w), "{text} 应以 {w} 结尾"),
                // 仪器条的实心格位置随帧号变，不能写死字面串。
                None => {
                    let glyphs = bar_glyphs(&text);
                    assert!(
                        glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
                        "应该只有仪器条字形：{text}"
                    );
                    assert_eq!(
                        glyphs.iter().filter(|c| **c == lit_ch()).count(),
                        BAR_BLOCK,
                        "应恰好一颗方块点亮：{text}"
                    );
                }
            }
            assert_eq!(UnicodeWidthStr::width(text.as_str()), 40, "整行铺满");
            // 三态的左区必须逐字相同：右缘统一到 `RIGHT_W` 列，切换时 `serve` / `model`
            // 才不会整体平移。这是「统一宽度」要买的那一个性质。
            let now = head_cols(&text, 40 - RIGHT_W);
            match &head {
                Some(first) => assert_eq!(&now, first, "左区随状态变了：{text}"),
                None => head = Some(now),
            }
        }
    }

    #[test]
    fn bar_cell_is_one_column() {
        // 两个字形都必须 1 列：仪器条靠「6 格 × 1 列」参与预算，
        // 任一被算成 2 列（或终端按全角渲染）右缘就会被推出屏幕。
        for cell in [BAR_CELL_DIM, BAR_CELL_LIT] {
            assert_eq!(UnicodeWidthStr::width(cell), 1, "{cell:?} 必须 1 列");
        }
        // 亮 / 灭必须等宽，否则扫描时行宽会跟着字形变。
        assert_eq!(
            UnicodeWidthStr::width(BAR_CELL_DIM),
            UnicodeWidthStr::width(BAR_CELL_LIT)
        );
    }

    #[test]
    fn bar_block_walks_with_frame() {
        let pal = Palette::default();
        let mut info = base_info("http://x");
        info.running = true;
        // 覆盖两圈：每 `BAR_TICKS_PER_CELL` 拍，亮块右移一格。
        let mut prev_start: Option<usize> = None;
        for frame in 0..BAR_CELLS * BAR_TICKS_PER_CELL * 2 {
            info.spinner = frame;
            let start = (frame / BAR_TICKS_PER_CELL) % BAR_CELLS;
            let want: Vec<usize> = (0..BAR_CELLS)
                .filter(|i| (i + BAR_CELLS - start) % BAR_CELLS < BAR_BLOCK)
                .collect();
            let line = status_line_with(&info, None, 60, &pal, false);
            let cells = bar_cells(&line);
            let lit: Vec<usize> = (0..BAR_CELLS)
                .filter(|i| cells[*i].style.fg == Some(pal.accent))
                .collect();
            assert_eq!(lit, want, "第 {frame} 拍亮块应从第 {start} 格起");
            // 块长固定：单颗，不多不少。数量变化会被读成进度。
            assert_eq!(lit.len(), BAR_BLOCK, "块长应恒为 {BAR_BLOCK}");
            // 亮块必须是**单一连续段**（含环绕）：只有一个「灭→亮」的入口。
            // 散点或两段就没有可追踪的边界，平移也就读不出来。
            let rises = (0..BAR_CELLS)
                .filter(|i| {
                    let cur = want.contains(i);
                    let prev = want.contains(&((i + BAR_CELLS - 1) % BAR_CELLS));
                    cur && !prev
                })
                .count();
            assert_eq!(rises, 1, "第 {frame} 拍亮块应是单一连续段：{want:?}");
            // 熄灭格仍要可见，否则块的边界看不出来。
            for i in (0..BAR_CELLS).filter(|i| !want.contains(i)) {
                assert_eq!(cells[i].style.fg, Some(pal.muted), "第 {i} 格应熄灭");
                assert_eq!(cells[i].content, BAR_CELL_DIM, "熄灭格应是熄灭字形");
            }
            // 起点每换一拍就前进一格（模 6）——「行进」就是这么来的。
            // 同一相位持续 `BAR_TICKS_PER_CELL` 拍，只在真的换格时比。
            if let Some(prev) = prev_start
                && prev != start
            {
                assert_eq!(start, (prev + 1) % BAR_CELLS, "起点应逐拍前进一格");
            }
            prev_start = Some(start);
        }
    }

    #[test]
    fn bar_keeps_constant_width_across_frames() {
        // 换格不改预算：右缘宽度只由「形态」决定，状态行才不会随帧左右抖。
        let pal = Palette::default();
        let mut info = base_info("http://x");
        info.running = true;
        let mut widths = std::collections::BTreeSet::new();
        // 含极大帧号：`(spinner / 2) % BAR_CELLS` 必须对任意 usize 安全，且不改字形。
        for frame in (0..BAR_CELLS + 3).chain([usize::MAX]) {
            info.spinner = frame;
            let text = status_line_with(&info, None, 60, &pal, false).to_string();
            // 仪器条的实心格随帧号移动，只断言「字符合法」；位置由上面那个测试守。
            let glyphs = bar_glyphs(&text);
            assert!(
                glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
                "帧 {frame}: {text}"
            );
            widths.insert(UnicodeWidthStr::width(text.as_str()));
        }
        assert_eq!(widths.len(), 1, "各帧行宽必须一致：{widths:?}");
    }

    #[test]
    fn reduced_motion_lights_the_whole_bar() {
        let pal = Palette::default();
        let mut info = base_info("http://x");
        info.running = true;
        info.spinner = 8;
        // 动效关掉后条不扫描，改成整条常亮：静止的「全亮条」与「○ 空闲」靠形状就能分辨，
        // 也就不必再加「生成中」字样（右缘定宽是硬约束）。
        let still = status_line_with(&info, None, 60, &pal, true);
        let cells = bar_cells(&still);
        assert!(
            cells.iter().all(|s| s.style.fg == Some(pal.accent)),
            "静态时应整条点亮：{cells:?}"
        );
        // 亮态必须是实心方块：形状与颜色一起变，底色消失时也分得出「运行中」和「空闲」。
        assert!(
            cells.iter().all(|s| s.content == BAR_CELL_LIT),
            "静态时应整条是实心方块：{cells:?}"
        );
        // 同一份输入在开动效时变成「亮块从第 4 格起」——证明开关真的生效。
        let moving = status_line_with(&info, None, 60, &pal, false);
        let moving_cells = bar_cells(&moving);
        assert_eq!(moving_cells[4].style.fg, Some(pal.accent));
        assert_eq!(moving_cells[1].style.fg, Some(pal.muted));
    }

    #[test]
    fn narrow_bar_keeps_only_the_bar() {
        // 右缘固定 8 列，窄终端也照旧：`serve` 被截断，但状态指示不会被挤掉。
        let pal = Palette::default();
        let mut info = base_info("http://long");
        info.model = Some("model-name".into());
        info.running = true;
        let text = status_line_with(&info, None, 16, &pal, false).to_string();
        // 窄条右缘仍是完整的 8 列（6 格 + 前导补空），只是左区被截短。
        let glyphs = bar_glyphs(&text);
        assert!(
            glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
            "窄条也保留仪器条：{text}"
        );
        assert_eq!(
            glyphs.iter().filter(|c| **c == lit_ch()).count(),
            BAR_BLOCK,
            "窄条照样一颗完整方块：{text}"
        );
        assert!(!text.contains("生成中"), "生成中已不再带文案：{text}");
        assert_eq!(UnicodeWidthStr::width(text.as_str()), 16, "仍恰好铺满");
    }

    #[test]
    fn mono_bar_falls_back_to_reversed_cell() {
        // `mono` 下所有槽都是 `Reset`，不换修饰位就完全看不出哪一格在动。
        let pal = Palette::mono();
        let mut info = base_info("http://x");
        info.running = true;
        // 4 拍 = 亮块首格前进到第 2 格（每 2 拍走一格）：2/3/4 亮，0/1/5 灭。
        info.spinner = 4;
        let line = status_line_with(&info, None, 60, &pal, false);
        let cells = bar_cells(&line);
        assert!(
            cells[2].style.add_modifier.contains(Modifier::REVERSED),
            "点亮格应走反色：{:?}",
            cells[2]
        );
        assert!(
            cells[1].style.add_modifier.contains(Modifier::DIM),
            "熄灭格应走 DIM：{:?}",
            cells[1]
        );
        assert!(
            cells
                .iter()
                .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset)),
            "无色模式不该出现彩色 span"
        );
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
