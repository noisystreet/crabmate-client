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
/// `view`（`↑N` 回看行数）紧随其后：它只在用户主动滚动时存在，而滚动的那一瞬间它是
/// **唯一**能回答「我在哪 / 下面还有多少」的信息。`serve` 虽然重要，却是全程常驻项，
/// 晚显示几列不损失判断依据；反过来，滚动时丢掉回看提示等于这次操作没有反馈。
/// 之后才是模型 / 模式这类「一进界面就知道」的项。
mod priority {
    // 数值越小越先保留（横版按列预算裁剪、纵版按行预算裁剪共用）。
    pub(super) const FIND: u8 = 0;
    pub(super) const VIEW: u8 = 1;
    pub(super) const SERVE: u8 = 2;
    pub(super) const MODEL: u8 = 3;
    // 上下文用量比 mode / role 更常被盯：窄屏时先丢后两者。
    pub(super) const CONTEXT: u8 = 4;
    pub(super) const MODE: u8 = 5;
    pub(super) const ROLE: u8 = 6;
    /// 权限等级：常驻但对"现在该干什么"没有影响（它是授权上限，不是当前动作），
    /// 窄屏时最先让位——丢它的代价只是看不到等级，丢 `ctx` / `find` 则丢的是正在变化的信息。
    pub(super) const PERM: u8 = 7;
}

/// 段间分隔符（极暗竖线，与 A6 的分隔线同色系）。
const SEPARATOR: &str = " │ ";
/// 分隔符显示宽。
const SEPARATOR_W: usize = 3;

/// 纵向状态栈的 label 列固定宽（`serve` / `model` 是最长的 5 列）。
///
/// 状态栈里所有行都按这一列宽排：第一列 label，第二列值。运行指示那一行不例外
/// （第一列回看提示、第二列运行态），否则整块会变成「一列数据 + 一个页脚」。
///
/// 放这里而不是 `status_stack`：运行指示行的缩进也要用它，两处各写一份常量迟早
/// 错开一列。新增更长的 label 时**必须**同步这里（`labels_fit_the_fixed_column` 会失败）。
pub(super) const LABEL_W: usize = 5;

/// label 列实际占的列数：定宽标签 + 1 列间隔。第二列（值 / 运行指示）从这里起。
pub(super) const LABEL_COL_W: usize = LABEL_W + 1;

/// 左区一个分段：`label` 走淡色、`value` 走正文色。
pub(super) struct Segment {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) priority: u8,
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

/// 环境段：`serve` / `model` / `role` / `mode` / `find`（顺序即显示顺序，与优先级无关）。
///
/// **不含回看提示**：纵向状态栈把它并进运行指示那一行（见
/// [`running_row_with_view`]），既不占独立行、折叠态也照样可见；只有横版底栏
/// 才需要它作为一个独立段 —— 见 [`left_segments`]。
///
/// **不含会话 id**：desktop 的状态区也没有它，而整行放一个被截断的 id 没有信息量
/// （要看当前会话，左栏有 `*` 标记）。
pub(super) fn environment_segments(info: &StatusInfo, budget: usize) -> Vec<Segment> {
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
    // 显示顺序排在 `mode` 之后，但优先级高于 `mode` / `role`（见 `priority`）。
    // 无条件 push：没有实测时值是占位 `—`，段消失会让人误判「功能没生效」。
    out.push(Segment {
        label: "ctx",
        value: info.context.clone(),
        priority: priority::CONTEXT,
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
    // 权限等级排在显示顺序最右（紧邻右缘运行态）：它是"我这一轮最多能动到什么"，
    // 与 `serve` / `model` 同为环境事实，但读数频率低，所以优先级最低。
    // 只有真拿到了才 push：`None` 是不确定（老 serve / 未拉到），不是"没权限"。
    if let Some(level) = info.permission.as_deref() {
        out.push(Segment {
            label: "perm",
            value: level.to_string(),
            priority: priority::PERM,
        });
    }
    out
}

/// 回看行数文案（`↑12`）；未滚动时为 `None`。
///
/// 状态栈（并进运行行）与横版底栏（当独立段）共用这一处格式：两处各拼一遍，
/// 迟早出现「横条显示 `↑12`、栈里显示 `↑ 12 行`」这种同义不同形的漂移。
pub(super) fn view_back_text(info: &StatusInfo) -> Option<String> {
    // 数字右对齐到两列：`↑9`(3 列) 与 `↑10`(4 列) 宽度不同，会让左区预算在跨越
    // 10 的倍数时忽多忽少 —— 临界宽度下 `serve` 就随滚轮闪进闪出。
    (info.view_offset > 0).then(|| format!("↑{:>2}", info.view_offset))
}

/// 横版左区：环境段 + 回看提示段。
///
/// 回看段排在显示顺序的末尾：滚动时它每帧都在变，放行尾就不会把前面的
/// `serve` 路径推来推去。
pub(super) fn left_segments(info: &StatusInfo, budget: usize) -> Vec<Segment> {
    let mut out = environment_segments(info, budget);
    if let Some(value) = view_back_text(info) {
        out.push(Segment {
            label: "",
            value,
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
pub(super) enum RunningKind {
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
pub(super) fn running_kind(info: &StatusInfo, reduced_motion: bool) -> RunningKind {
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
    spans.extend(running_spans(kind, pal));
}

/// 运行态自身的 span（**不含**右对齐补白）。
///
/// 横版状态行右对齐到 [`RIGHT_W`]、纵向状态栈左对齐 —— 两者共用这一份字形与配色，
/// 免得仪器条在两处各写一遍（改一处忘一处就会出现「横条在动、栈里不动」）。
pub(super) fn running_spans(kind: &RunningKind, pal: &Palette) -> Vec<Span<'static>> {
    let bold = |fg: Color| Style::new().fg(fg).add_modifier(Modifier::BOLD);
    match kind {
        RunningKind::Idle => vec![Span::styled(IDLE_TEXT, bold(pal.muted))],
        RunningKind::Cancelling => vec![Span::styled(CANCELLING_TEXT, bold(pal.accent))],
        RunningKind::Generating { phase } => (0..BAR_CELLS)
            .map(|i| {
                // 方形亮块沿条右移：条内只有这一个亮特征，方向不会被误读。
                // 「隔一亮一」不行——那种图案只有两个相位，平移一格就等于整体反转，
                // 眼睛只会看到闪烁（周期 2 < 位移可分辨的尺度）。
                let is_lit = phase.is_none_or(|p| (i + BAR_CELLS - p) % BAR_CELLS < BAR_BLOCK);
                let (cell, style) = if is_lit {
                    (BAR_CELL_LIT, bar_lit_style(pal))
                } else {
                    (BAR_CELL_DIM, bar_dim_style(pal))
                };
                Span::styled(cell, style)
            })
            .collect(),
    }
}

/// 纵向栈的「运行指示」那一行：**按两列排**——第一列回看提示（可空），第二列运行态。
///
/// 与上面各段同一套列（第一列 label、第二列值），整块因此是一张两列表格：
///
/// ```text
/// serve 127.0.0.1:8080
/// model gpt-x
/// role  coder
/// mode  plan
/// ↑12   ○ 空闲
/// ```
///
/// 之前这一行贴右缘，与上面的值列不成列，整块读起来像「一列数据 + 一个页脚」。
///
/// 回看提示并进这一行、而不是自己占一段一行的理由：
/// 1. 折叠态只画这一行（见 `status_stack::stack_lines`），独立成段的话一折叠它就
///    彻底消失——恰恰是滚动时最需要它的形态；
/// 2. 展开态里它会挤掉一行环境段，而它只是个临时提示。
///
/// 运行态是**硬需求**（生成中 / 取消中必须随时可见），回看提示可丢弃：宽度连
/// 「第二列」都放不下时退回贴左的纯运行态，不缩进。
pub(super) fn running_row_with_view(
    kind: &RunningKind,
    view_back: Option<&str>,
    width: usize,
    pal: &Palette,
) -> Vec<Span<'static>> {
    if LABEL_COL_W + running_content_w(kind) > width {
        return running_spans(kind, pal);
    }
    // 回看提示自己也可能超出一列（`↑123456`）：截到列宽内，别把第二列推歪。
    let mut first = view_back.map_or_else(String::new, |v| truncate_display(v, LABEL_W));
    first.push_str(&" ".repeat(LABEL_COL_W.saturating_sub(UnicodeWidthStr::width(first.as_str()))));
    // 淡色：它是「此刻在哪」的辅助提示，不该和右侧运行态抢注意力。
    let mut spans: Vec<Span<'static>> = vec![Span::styled(first, Style::new().fg(pal.muted))];
    spans.extend(running_spans(kind, pal));
    spans
}

/// `CM_TUI_REDUCED_MOTION=1`：动效退回静态。与 `NO_COLOR` 同理——录屏、远程终端、
/// 屏幕阅读器场景下这类效果是干扰。
///
/// 进程内只读一次环境变量：本函数在渲染热路径上（每帧 2–50ms 一次），不该每帧分配字符串。
pub(super) fn reduced_motion() -> bool {
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
pub(super) fn status_line(info: &StatusInfo, width: usize, pal: &Palette) -> Line<'static> {
    status_line_with(info, width, pal, reduced_motion())
}

/// [`status_line`] 的可注入版本：单测要能强制 reduced motion，不依赖真实环境变量
/// （与 `Palette::resolve_with` 同一手法）。
fn status_line_with(
    info: &StatusInfo,
    width: usize,
    pal: &Palette,
    reduced_motion: bool,
) -> Line<'static> {
    let kind = running_kind(info, reduced_motion);
    // 右缘恒占 `RIGHT_W` 列：预算不随状态变，切换时左区不跳。
    let right_w = RIGHT_W;
    let budget = width.saturating_sub(right_w);
    let segs = left_segments(info, budget);
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
pub(super) fn status_text(info: &StatusInfo, width: usize) -> String {
    status_line(info, width, &Palette::default()).to_string()
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
