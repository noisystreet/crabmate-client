//! 终端配色板：一个 Desktop 主题 → 一组语义化终端颜色。
//!
//! 值取自 `frontend/styles/tokens.css`（默认深色基底）与 `frontend/themes/*.css`；
//! 主题名与 `UserPrefsDto.theme` 同一枚举，TUI 只读、不自建主题格式。
//! 浅色主题的聊天区底色取 `--surface-hover` 而非 `--surface`：整屏纯白在终端里刺眼，
//! 压暗一档后正文/次要信息都还够对比度。
//!
//! 终端能力差异由 [`Palette::resolve`] 兜底：无真彩时把 `Rgb` 降到最接近的 16 色，
//! `NO_COLOR` / `TERM=dumb` 时退到无色模式（靠修饰位区分，不靠颜色）。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::editor_view::editor_hl::SyntaxCat;
use super::term_image::{self, ColorDepth};
use super::ws_file_kind::FileKind;

/// 语义化配色板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// 正文（`--text`）。
    pub fg: Color,
    /// 次要信息：消息前缀、工具 note、提示（`--muted`）。
    pub muted: Color,
    /// 强调：链接、用户消息基色（`--accent`）。
    pub accent: Color,
    /// 信息态（`--info`）：**工具行的消息前缀**用它。
    ///
    /// 不复用 `ok` / `err`：那两档已被工具行的**正文**按结果占用，同一行里绿色前缀 +
    /// 红色内容会自相矛盾。也不复用 `hit`——那是 `/find` 命中色，混用会让「搜到」与
    /// 「这是工具行」看起来是一回事。
    pub info: Color,
    /// 搜索命中（非锚定行）；锚定行用它的反色块。独立于 `accent`，
    /// 否则会与用户消息的强调色混淆。
    pub hit: Color,
    /// 聊天区底色（`--surface`；浅色主题刻意改取 surface-hover 一档，见脚本的 TUI_SLOT_TOKEN_OVERRIDE）。
    pub bg: Color,
    /// 状态栏 / 侧栏 / composer 底色（比 `bg` 再压一档，形成条带）。
    pub chrome_bg: Color,
    /// 列表选中行底色（终端专用：桌面那档是 color-mix，例值取不到）。
    ///
    /// 无色模式下会被抹成 `Reset`，选中态改由 [`Palette::selection`] 退回反色表达。
    pub selection_bg: Color,
    /// 浮层（审批 / 设置面板）的卡片底色。
    ///
    /// 独立于 `chrome_bg`：条带色比聊天区更暗，用它当浮层底会让卡片看起来是**凹进去**的
    /// （背景被挖空），而不是浮在内容之上。这一档取「比 `bg` 亮一步」，深色主题下是抬升的
    /// surface、浅色主题下是更白的卡片。
    pub overlay_bg: Color,
    /// 边框（`--border`）。
    pub border: Color,
    /// 行内代码底色。
    pub code_bg: Color,
    /// 工具成功。**终端专用色阶**：比 Web 的 success 语义色更饱和 —— 终端字号小、
    /// 格子少，低饱和的 Web 语义色在 16 色降级后容易糊进 `muted`。故不跟随该 token。
    pub ok: Color,
    /// 工具失败 / 系统告警。同为终端专用色阶，理由同 [`Palette::ok`]（不跟随 error 语义色）。
    pub err: Color,
    /// 注意态（`--warn`）：未保存改动、待重试、被改写的值。
    pub notice: Color,
    /// 工作区树：按语言 / 资产细分的文件色板。
    ///
    /// **终端专用色阶**：`tokens.css` 没有「文件类型」这个概念，所以这些槽不绑定任何
    /// token（与 `ok` / `err` / `hit` 同级待遇），因此也不需要动任何 Desktop 主题文件。
    ///
    /// 色相模仿 Material Icon Theme（Rust 橙、Python 蓝、JS 黄、TS 青蓝、Go 青、JVM 紫、
    /// C 系蓝紫、脚本绿、Web 珊瑚、配置冷灰、锁文件灰、数据暗金、文档青绿、图片粉、
    /// 字体音视频淡紫灰）。只按**明暗两端**各出一套：换个主题但明暗没变时，目录树里的
    /// 「这是 Rust 还是 Python」不该跟着漂 —— 那是文件的属性，不是主题的属性。
    pub lang: LangColors,
    /// 源码视图：注释。
    ///
    /// 与 `file_*` 同一取舍，**终端专用色阶**：`tokens.css` 没有「语法类别」这个概念，
    /// 所以这六档不绑定任何 token（因此也不需要动任何 Desktop 主题文件）。只有
    /// [`Palette::syntax_style`] 读它们 —— 调用点只说类别，颜色归这里。
    pub syn_comment: Color,
    /// 源码视图：字符串 / 字符字面量。
    pub syn_str: Color,
    /// 源码视图：数字字面量。
    pub syn_number: Color,
    /// 源码视图：关键字（含 C++ 的 `#include` 整行）。
    pub syn_keyword: Color,
    /// 源码视图：类型名（内置类型表 + `UpperCamel`）。
    pub syn_type: Color,
    /// 源码视图：函数名 / 宏（`name!`）。
    pub syn_fn: Color,
    /// 无色模式：所有颜色为 `Reset`，区分只靠 `DIM` / `BOLD` / `REVERSED`。
    pub mono: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// 工作区树的语言 / 资产色板（[`Palette::ws_entry_color`] 的唯一颜色来源）。
///
/// 十五档对着 [`FileKind`] 的十五个非目录 / 非其它变体，一一对应 —— 渲染处只说「这是
/// 什么文件」，颜色归这里（与 `syn_*` / `NoticeLevel` 同一条路）。
///
/// 为什么不给每个主题各写一份：这些色相是**类型身份**，不是主题表达。七套主题里四套深色
/// 共用 [`LANG_DARK`]、两套浅色共用 [`LANG_LIGHT`]，新加主题只要挑明暗，不必重新调 15 个色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LangColors {
    pub rust: Color,
    pub python: Color,
    pub js: Color,
    pub ts: Color,
    pub go: Color,
    pub jvm: Color,
    pub native: Color,
    pub shell: Color,
    pub web: Color,
    pub config: Color,
    pub lock: Color,
    pub data: Color,
    pub doc: Color,
    pub image: Color,
    pub media: Color,
}

/// 深色底：色相取 Material Icon Theme 的原值，明度提到能在 `#16172（bg）` 上一眼分辨。
const LANG_DARK: LangColors = LangColors {
    rust: rgb(0xde, 0xa5, 0x84),
    python: rgb(0x6f, 0xa8, 0xdc),
    js: rgb(0xe5, 0xc0, 0x7b),
    ts: rgb(0x4f, 0xa3, 0xd1),
    go: rgb(0x3f, 0xc7, 0xc7),
    jvm: rgb(0xc7, 0x92, 0xea),
    native: rgb(0x82, 0xaa, 0xff),
    shell: rgb(0x89, 0xe0, 0x51),
    web: rgb(0xf0, 0x71, 0x78),
    config: rgb(0xa8, 0xb0, 0xc0),
    lock: rgb(0x8a, 0x8f, 0x98),
    data: rgb(0xc9, 0xa2, 0x27),
    doc: rgb(0x8f, 0xbc, 0xbb),
    image: rgb(0xe7, 0x9a, 0xc0),
    media: rgb(0xb0, 0xa0, 0xc0),
};

/// 浅色底：同一批色相压暗到 `#f0ece2` / `#f4f4f5` 上仍能读（照抄深色值会糊成一片浅色）。
const LANG_LIGHT: LangColors = LangColors {
    rust: rgb(0x8a, 0x4b, 0x2a),
    python: rgb(0x1f, 0x5c, 0x86),
    js: rgb(0x7a, 0x5b, 0x00),
    ts: rgb(0x0e, 0x5c, 0x7a),
    go: rgb(0x0b, 0x6e, 0x6e),
    jvm: rgb(0x5b, 0x3d, 0x8f),
    native: rgb(0x1f, 0x3f, 0x9e),
    shell: rgb(0x2e, 0x6b, 0x12),
    web: rgb(0x9e, 0x2b, 0x2e),
    config: rgb(0x3f, 0x4a, 0x5a),
    lock: rgb(0x4a, 0x4f, 0x57),
    data: rgb(0x6b, 0x53, 0x00),
    doc: rgb(0x2f, 0x5b, 0x45),
    image: rgb(0x8f, 0x2f, 0x63),
    media: rgb(0x4b, 0x3f, 0x6b),
};

/// 无色模式：全 `Reset`。漏一个就是「`NO_COLOR` 下目录树仍然带色」——
/// 与 `palette.rs` 对 mono 的承诺正面冲突，而那种 bug 只在无色终端里可见。
const LANG_MONO: LangColors = LangColors {
    rust: Color::Reset,
    python: Color::Reset,
    js: Color::Reset,
    ts: Color::Reset,
    go: Color::Reset,
    jvm: Color::Reset,
    native: Color::Reset,
    shell: Color::Reset,
    web: Color::Reset,
    config: Color::Reset,
    lock: Color::Reset,
    data: Color::Reset,
    doc: Color::Reset,
    image: Color::Reset,
    media: Color::Reset,
};

impl LangColors {
    /// 16 色降级：与 [`Palette::downgrade_basic`] 同一口径，逐槽取最近命名色。
    ///
    /// 真彩终端上十五档互不相同；降到 16 色后必然有若干档合并 —— 那是降级的固有代价，
    /// 不是这里可以避免的（`nearest_ansi` 只答「最近的那个」，答不了「能不能分辨」）。
    #[must_use]
    fn downgraded(self) -> Self {
        Self {
            rust: nearest_ansi(self.rust),
            python: nearest_ansi(self.python),
            js: nearest_ansi(self.js),
            ts: nearest_ansi(self.ts),
            go: nearest_ansi(self.go),
            jvm: nearest_ansi(self.jvm),
            native: nearest_ansi(self.native),
            shell: nearest_ansi(self.shell),
            web: nearest_ansi(self.web),
            config: nearest_ansi(self.config),
            lock: nearest_ansi(self.lock),
            data: nearest_ansi(self.data),
            doc: nearest_ansi(self.doc),
            image: nearest_ansi(self.image),
            media: nearest_ansi(self.media),
        }
    }
}

/// 默认深色（`tokens.css` 根值；`theme` 缺失或未知时使用）。
const DEFAULT_DARK: Palette = Palette {
    fg: rgb(0xee, 0xf1, 0xf6),
    muted: rgb(0x90, 0x99, 0xb4),
    accent: rgb(0x8f, 0xa4, 0xea),
    info: rgb(0x6f, 0xb8, 0xd8),
    hit: rgb(0xe6, 0xc4, 0x5c),
    bg: rgb(0x16, 0x17, 0x27),
    chrome_bg: rgb(0x10, 0x11, 0x1f),
    selection_bg: rgb(0x23, 0x25, 0x41),
    // 直接取桌面 `--surface-hover`（#1f2033）：比 `bg`(=--surface) 亮一档，
    // 又比 `selection_bg` 暗，卡片既能浮起来又不抢选中行的注意力。
    overlay_bg: rgb(0x1f, 0x20, 0x33),
    border: rgb(0x2a, 0x2b, 0x40),
    code_bg: rgb(0x1f, 0x20, 0x33),
    ok: rgb(0x6c, 0xc4, 0x9a),
    err: rgb(0xf2, 0x77, 0x7a),
    notice: rgb(0xc4, 0x9a, 0x6e),
    lang: LANG_DARK,
    // 语法高亮六档（源码视图）。与 `file_*` 同一取舍：**终端专用色阶**，不绑 token
    // —— `tokens.css` 没有「语法类别」这个概念，绑了就要同步改 5 个 theme css 并过镜像
    // 门禁，而 Desktop 侧的代码块高亮本身还没做。
    //
    // 取名对着 `SyntaxCat` 的六个变体一一对应；`Palette::syntax_style` 是唯一出口。
    syn_comment: rgb(0x6b, 0x73, 0x90),
    syn_str: rgb(0x9e, 0xce, 0x6a),
    syn_number: rgb(0xff, 0x9e, 0x64),
    syn_keyword: rgb(0xbb, 0x9a, 0xf7),
    syn_type: rgb(0x7d, 0xcf, 0xff),
    syn_fn: rgb(0x7a, 0xa2, 0xf7),
    mono: false,
};

/// `themes/high-contrast-dark.css`：高对比，边框与正文字都拉到亮端。
const HIGH_CONTRAST_DARK: Palette = Palette {
    fg: rgb(0xfa, 0xfa, 0xfa),
    muted: rgb(0xb8, 0xb8, 0xb8),
    accent: rgb(0xf5, 0xf5, 0xf5),
    info: rgb(0xd0, 0xd0, 0xd0),
    hit: rgb(0xff, 0xe0, 0x66),
    bg: rgb(0x24, 0x24, 0x24),
    chrome_bg: rgb(0x1a, 0x1a, 0x1a),
    selection_bg: rgb(0x3a, 0x3a, 0x3a),
    overlay_bg: rgb(0x2f, 0x2f, 0x2f),
    border: rgb(0xe8, 0xe8, 0xe8),
    code_bg: rgb(0x2c, 0x2c, 0x2c),
    ok: rgb(0x66, 0xff, 0x99),
    err: rgb(0xff, 0x6b, 0x6b),
    notice: rgb(0xc6, 0xc6, 0xc6),
    lang: LANG_DARK,
    // 高对比主题的主色刻意是灰阶，但语法档仍需可辨：取**高明度**彩色，
    // 与 `file_*` 四档在这个主题下的处理一致（它们同样是彩色）。
    syn_comment: rgb(0x9a, 0x9a, 0x9a),
    syn_str: rgb(0xc8, 0xf0, 0xa8),
    syn_number: rgb(0xff, 0xd0, 0x80),
    syn_keyword: rgb(0xff, 0xb0, 0xe8),
    syn_type: rgb(0x90, 0xe0, 0xff),
    syn_fn: rgb(0xb8, 0xc8, 0xff),
    mono: false,
};

/// `themes/material-dark.css`。
const MATERIAL_DARK: Palette = Palette {
    fg: rgb(0xec, 0xec, 0xee),
    muted: rgb(0x97, 0x97, 0x9f),
    accent: rgb(0xc2, 0xc4, 0xcd),
    info: rgb(0x95, 0x9e, 0xad),
    hit: rgb(0xe6, 0xc4, 0x5c),
    bg: rgb(0x1b, 0x1b, 0x1f),
    chrome_bg: rgb(0x14, 0x14, 0x17),
    selection_bg: rgb(0x2b, 0x2b, 0x31),
    overlay_bg: rgb(0x25, 0x25, 0x2b),
    border: rgb(0x32, 0x32, 0x36),
    code_bg: rgb(0x24, 0x24, 0x28),
    ok: rgb(0x6c, 0xc4, 0x9a),
    err: rgb(0xf2, 0x77, 0x7a),
    notice: rgb(0xc2, 0x98, 0x68),
    lang: LANG_DARK,
    syn_comment: rgb(0x76, 0x76, 0x76),
    syn_str: rgb(0xa8, 0xd0, 0x8a),
    syn_number: rgb(0xe8, 0xa8, 0x7c),
    syn_keyword: rgb(0xc0, 0xa8, 0xe0),
    syn_type: rgb(0x90, 0xc8, 0xd8),
    syn_fn: rgb(0xa8, 0xb8, 0xe0),
    mono: false,
};

/// `themes/shadcn-dark.css`。
const SHADCN_DARK: Palette = Palette {
    fg: rgb(0xfa, 0xfa, 0xfa),
    muted: rgb(0xa1, 0xa1, 0xaa),
    accent: rgb(0x60, 0xa5, 0xfa),
    info: rgb(0x38, 0xbd, 0xf8),
    hit: rgb(0xf5, 0xd0, 0x5a),
    bg: rgb(0x18, 0x18, 0x1b),
    chrome_bg: rgb(0x10, 0x10, 0x12),
    selection_bg: rgb(0x26, 0x26, 0x31),
    overlay_bg: rgb(0x22, 0x22, 0x28),
    border: rgb(0x27, 0x27, 0x2a),
    code_bg: rgb(0x27, 0x27, 0x2a),
    ok: rgb(0x6c, 0xc4, 0x9a),
    err: rgb(0xf2, 0x77, 0x7a),
    notice: rgb(0xfb, 0xbf, 0x24),
    lang: LANG_DARK,
    syn_comment: rgb(0x71, 0x71, 0x7a),
    syn_str: rgb(0x86, 0xef, 0xac),
    syn_number: rgb(0xfd, 0xba, 0x74),
    syn_keyword: rgb(0xd8, 0xb4, 0xfe),
    syn_type: rgb(0x7d, 0xd3, 0xfc),
    syn_fn: rgb(0x93, 0xc5, 0xfd),
    mono: false,
};

/// 已知主题名：`from_theme` 的 match 与 `CM_TUI_THEME` 的校验共用一份。
///
/// 加新预设时两处都要改——`every_listed_theme_name_maps_to_its_own_palette`
/// 会在漏接 `from_theme` 时变红。
///
/// `crabmate-dark` 对应 `tokens.css` 的 `:root` 深色基线，配色与 [`DEFAULT_DARK`]
/// 相同但名字独立：列入后 `CM_TUI_THEME=crabmate-dark` 不再被当作未知名忽略。
const THEME_NAMES: [&str; 8] = [
    "crabmate-light",
    "crabmate-dark",
    "shadcn-light",
    "shadcn-dark",
    "material-dark",
    "high-contrast-dark",
    "shadcn-transparent",
    "default-dark",
];

/// 设置面板「主题」字段的 ←→ 循环选项（首项 `None` = 清空 prefs 键 = 产品默认透明）。
///
/// 选项全部取自 [`THEME_NAMES`]，因此面板里看到的每一项都能被 [`Palette::from_theme`]
/// 精确映射（选了就有变化，不会静默落到深色兜底）。`system` 不在其中：终端拿不到
/// 桌面端的 `matchMedia` / Tauri 明暗提示，选它只会退成实底深色，与 Desktop 浅色
/// 系统下的表现相反。
pub(crate) const THEME_OPTIONS: [Option<&str>; 9] = [
    None,
    Some("shadcn-transparent"),
    Some("default-dark"),
    Some("crabmate-dark"),
    Some("shadcn-dark"),
    Some("material-dark"),
    Some("high-contrast-dark"),
    Some("crabmate-light"),
    Some("shadcn-light"),
];

/// `shadcn-dark` 的透明变体：**只**把两块底色还给终端（`Color::Reset`）。
///
/// 终端没有 alpha：`Reset` 的意思是「这一格不铺底色」，于是露出终端自己的背景
/// （终端自身开了 opacity 才看得到壁纸）。分区感因此只剩 `border` 与气泡这类实色，
/// 其余槽位一律沿用 `shadcn-dark`，且 `border` / `code_bg` / `selection_bg` /
/// `overlay_bg` 必须保持实色，否则整屏会糊成一片。
const SHADCN_TRANSPARENT: Palette = Palette {
    bg: Color::Reset,
    chrome_bg: Color::Reset,
    ..SHADCN_DARK
};

/// `themes/crabmate-light.css`：聊天区底色用 `--surface-hover`（`#f0ece2`）压暗一档。
const CRABMATE_LIGHT: Palette = Palette {
    fg: rgb(0x1c, 0x19, 0x13),
    muted: rgb(0x6b, 0x63, 0x57),
    accent: rgb(0x55, 0x70, 0x5b),
    info: rgb(0x3a, 0x6e, 0x83),
    hit: rgb(0x8a, 0x6d, 0x00),
    bg: rgb(0xf0, 0xec, 0xe2),
    chrome_bg: rgb(0xf7, 0xf4, 0xec),
    selection_bg: rgb(0xe3, 0xde, 0xd0),
    overlay_bg: rgb(0xfd, 0xfb, 0xf7),
    border: rgb(0xe2, 0xdd, 0xd1),
    code_bg: rgb(0xe8, 0xe2, 0xd6),
    ok: rgb(0x2f, 0x7d, 0x4f),
    err: rgb(0xb4, 0x23, 0x18),
    notice: rgb(0x83, 0x66, 0x28),
    lang: LANG_LIGHT,
    syn_comment: rgb(0x85, 0x7c, 0x6b),
    syn_str: rgb(0x3f, 0x6b, 0x26),
    syn_number: rgb(0x8a, 0x5a, 0x00),
    syn_keyword: rgb(0x6b, 0x3d, 0x8f),
    syn_type: rgb(0x14, 0x66, 0x6b),
    syn_fn: rgb(0x2b, 0x4f, 0x8f),
    mono: false,
};

/// `themes/shadcn-light.css`：同上，底色取 `--surface-hover`（`#f4f4f5`）。
const SHADCN_LIGHT: Palette = Palette {
    fg: rgb(0x09, 0x09, 0x0b),
    muted: rgb(0x69, 0x69, 0x71),
    accent: rgb(0x25, 0x63, 0xeb),
    info: rgb(0x03, 0x69, 0xa1),
    hit: rgb(0x8a, 0x6d, 0x00),
    bg: rgb(0xf4, 0xf4, 0xf5),
    chrome_bg: rgb(0xec, 0xec, 0xed),
    selection_bg: rgb(0xdc, 0xdc, 0xe1),
    overlay_bg: rgb(0xff, 0xff, 0xff),
    border: rgb(0xe4, 0xe4, 0xe7),
    code_bg: rgb(0xe8, 0xe8, 0xea),
    ok: rgb(0x2f, 0x7d, 0x4f),
    err: rgb(0xb4, 0x23, 0x18),
    notice: rgb(0xb4, 0x53, 0x09),
    lang: LANG_LIGHT,
    syn_comment: rgb(0x71, 0x71, 0x7a),
    syn_str: rgb(0x15, 0x80, 0x3d),
    syn_number: rgb(0xb4, 0x53, 0x09),
    syn_keyword: rgb(0x7e, 0x22, 0xce),
    syn_type: rgb(0x0f, 0x76, 0x6e),
    syn_fn: rgb(0x1d, 0x4e, 0xd8),
    mono: false,
};

/// 无色模式（`NO_COLOR` / `TERM=dumb`）。
const MONO: Palette = Palette {
    fg: Color::Reset,
    muted: Color::Reset,
    accent: Color::Reset,
    info: Color::Reset,
    hit: Color::Reset,
    bg: Color::Reset,
    chrome_bg: Color::Reset,
    selection_bg: Color::Reset,
    overlay_bg: Color::Reset,
    border: Color::Reset,
    code_bg: Color::Reset,
    ok: Color::Reset,
    err: Color::Reset,
    notice: Color::Reset,
    lang: LANG_MONO,
    // 无色模式：语法色一并抹平。漏一个就是「`NO_COLOR` 下源码视图仍然带色」，
    // 与 `palette.rs` 对 mono 的承诺正面冲突 —— 而那种 bug 只在无色终端里可见。
    syn_comment: Color::Reset,
    syn_str: Color::Reset,
    syn_number: Color::Reset,
    syn_keyword: Color::Reset,
    syn_type: Color::Reset,
    syn_fn: Color::Reset,
    mono: true,
};

/// 提示行的语义级别（**内容**）。调用方只说「这是什么」，颜色由 [`Palette`] 决定（**样式**）。
///
/// 之前调用点直接传 `Color::LightYellow` / `LightRed` / `LightGreen`：命名色不经过
/// [`Palette::resolve_with`]，于是主题切换不影响它们、`NO_COLOR` / `TERM=dumb` 下仍然
/// 带色（`mono()` 只抹 palette 的槽），「未保存」与「待重试」也各自挑色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeLevel {
    /// 中性引导 / 预览文本（按键提示、`←→` 循环预览）。
    Muted,
    /// 中性信息（已保存、面板解锁）。
    Info,
    /// 需要注意（未保存改动、只读、无改动可保存）。
    Warn,
    /// 失败（保存失败、输入校验不通过）。
    Error,
    /// 成功（写入完成）。
    Ok,
}

/// 设置面板值列的色调（**内容**）：说的是一格值**是什么性质**，不是它该长什么颜色。
///
/// 换成枚举的直接理由和 [`NoticeLevel`] 一样——原来的 `Color::White` / `Gray` /
/// `DarkGray` 让「值来自 user-data」在切主题后仍旧是那三档 ANSI 灰，
/// 与 `fg` / `muted` 的实际色值对不上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldTone {
    /// 当前生效值。
    Value,
    /// 次要值 / 只读说明。
    Muted,
    /// 未设置（占位措辞）：比 [`FieldTone::Muted`] 再淡一档，靠 `DIM` 表达。
    Unset,
    /// 已 staged、尚未保存的改写值。
    Pending,
    /// 来自 override 层的值（带 `*` 标记）。
    Override,
}

/// 结构体兜底：**实底深色**，与 [`Palette::from_theme`] 的「未设置 → 透明」刻意不同。
///
/// 两者分工不同：`from_theme(None)` 表达产品默认（透明），而 `Default` 只给测试夹具与
/// `..Default::default()` 一个「有底色、可读」的安全基线。若把 `Default` 也做成透明，
/// 任何拿它当夹具的用例都会静默失去条带色（`render_tests` / `status` 里就有十几处）。
impl Default for Palette {
    fn default() -> Self {
        Self::DARK
    }
}

impl Palette {
    /// 实底深色基线（与 `DEFAULT_DARK` 同值）：测试夹具与「没有主题上下文」时的借用目标。
    ///
    /// `PanelCtx.pal` 是借用字段，而测试里的 `ctx()` helper 返回 `PanelCtx<'a>`，喂不了
    /// 临时值 —— `&Palette::DARK` 走常量提升，可以直接给 `'static`。
    pub(crate) const DARK: Self = DEFAULT_DARK;

    /// Desktop 主题名 → 配色板；未知 / 空 → 默认深色（不回落到不可读的随机取值）。
    #[must_use]
    pub(crate) fn from_theme(theme: Option<&str>) -> Self {
        match theme.map(str::trim) {
            // 未设置（含空串）：产品默认就是透明——启动到拉回 prefs 之间也走这里。
            None | Some("") => SHADCN_TRANSPARENT,
            Some("crabmate-light") => CRABMATE_LIGHT,
            Some("shadcn-light") => SHADCN_LIGHT,
            Some("shadcn-dark") => SHADCN_DARK,
            Some("material-dark") => MATERIAL_DARK,
            Some("high-contrast-dark") => HIGH_CONTRAST_DARK,
            Some("shadcn-transparent") => SHADCN_TRANSPARENT,
            // 旧的默认深色留一个显式名字：透明是新的默认，但得能切回来。
            Some("default-dark") => DEFAULT_DARK,
            // 认识不了的名字（前端将来新增主题 / user-data 被手改 / 拼错）→ 回退实底深色。
            // 这里**不能**给透明：静默变透明的表现是「一片黑、没有条带」，几乎无法归因；
            // 实底深色至少有可读的正文与分区。
            Some(_) => DEFAULT_DARK,
        }
    }

    /// 无色模式：颜色一律 `Reset`，调用方改用修饰位表达语义。
    #[must_use]
    pub(crate) fn mono() -> Self {
        MONO
    }

    /// 当前调色板在无色模式下？
    #[must_use]
    pub(crate) fn is_mono(&self) -> bool {
        self.mono
    }

    /// 值列色调 → 样式（唯一的「色调怎么画」定义）。
    ///
    /// 无色模式下 `fg` / `muted` 全是 `Reset`，层级仍靠 `DIM` 与 `REVERSED` 表达：
    /// `Unset` 淡一档、`Busy` 也淡一档，两者靠**文本本身**区分（「(跟随 server)」vs「保存中…」）。
    #[must_use]
    pub(crate) fn tone_style(&self, tone: FieldTone) -> Style {
        match tone {
            FieldTone::Value => Style::new().fg(self.fg),
            FieldTone::Muted => Style::new().fg(self.muted),
            FieldTone::Unset => Style::new().fg(self.muted).add_modifier(Modifier::DIM),
            FieldTone::Pending => Style::new().fg(self.notice),
            FieldTone::Override => Style::new().fg(self.info),
        }
    }

    /// 语法类别 → 前景样式（源码视图的唯一出口）。
    ///
    /// 返回 `Style` 而不是 `Color`，是为了让「语法色」今后与别的样式位（修饰位、底色）
    /// 一起长出时调用点不必再改一次。无色模式下六个槽都是 `Reset`，于是这里给出的 `fg`
    /// 就是「不指定」—— 与其余槽位同一口径，不额外判断 `mono`。
    ///
    /// 调用方负责**叠加顺序**：先本样式、后 `md::highlight_override`。查找命中因此压过
    /// 语法色 —— 正在找的东西优先于装饰。
    #[must_use]
    pub(crate) fn syntax_style(&self, cat: SyntaxCat) -> Style {
        let color = match cat {
            SyntaxCat::Comment => self.syn_comment,
            SyntaxCat::Str => self.syn_str,
            SyntaxCat::Number => self.syn_number,
            SyntaxCat::Keyword => self.syn_keyword,
            SyntaxCat::Type => self.syn_type,
            SyntaxCat::Function => self.syn_fn,
        };
        Style::new().fg(color)
    }

    /// 提示级别 → 样式。无色模式下四个色槽都是 `Reset`，区别只剩文本措辞。
    #[must_use]
    pub(crate) fn notice_style(&self, level: NoticeLevel) -> Style {
        let color = match level {
            NoticeLevel::Muted => self.muted,
            NoticeLevel::Info => self.info,
            NoticeLevel::Warn => self.notice,
            NoticeLevel::Error => self.err,
            NoticeLevel::Ok => self.ok,
        };
        Style::new().fg(color)
    }

    /// 工作区树条目颜色（[`FileKind`] → 色槽）。
    ///
    /// 只给颜色、不给 `Style`：调用方要把它再交给 [`Self::selection`] 组成选中态，
    /// 「选中行长什么样」仍然只有那一处定义。分类本身在 [`super::ws_file_kind`]，
    /// 这里只做「类型 → 槽」的映射 —— 让渲染处不自己挑槽，是本文件上半部分
    /// （[`NoticeLevel`] / [`FieldTone`]）已经走过的同一条路。
    #[must_use]
    pub(crate) fn ws_entry_color(&self, kind: FileKind) -> Color {
        match kind {
            FileKind::Dir => self.accent,
            FileKind::Rust => self.lang.rust,
            FileKind::Python => self.lang.python,
            FileKind::Js => self.lang.js,
            FileKind::Ts => self.lang.ts,
            FileKind::Go => self.lang.go,
            FileKind::Jvm => self.lang.jvm,
            FileKind::Native => self.lang.native,
            FileKind::Shell => self.lang.shell,
            FileKind::Web => self.lang.web,
            FileKind::Config => self.lang.config,
            FileKind::Lock => self.lang.lock,
            FileKind::Data => self.lang.data,
            FileKind::Doc => self.lang.doc,
            FileKind::Image => self.lang.image,
            FileKind::Media => self.lang.media,
            FileKind::Other => self.fg,
        }
    }

    /// 把 `Rgb` 降到最接近的 16 色（非真彩终端用）；已是命名色则原样返回。
    #[must_use]
    pub(crate) fn downgrade_basic(self) -> Self {
        let mut p = self;
        p.fg = nearest_ansi(p.fg);
        p.muted = nearest_ansi(p.muted);
        p.accent = nearest_ansi(p.accent);
        p.info = nearest_ansi(p.info);
        p.hit = nearest_ansi(p.hit);
        p.bg = nearest_ansi(p.bg);
        p.chrome_bg = nearest_ansi(p.chrome_bg);
        p.border = nearest_ansi(p.border);
        p.selection_bg = nearest_ansi(p.selection_bg);
        p.overlay_bg = nearest_ansi(p.overlay_bg);
        p.code_bg = nearest_ansi(p.code_bg);
        p.ok = nearest_ansi(p.ok);
        p.err = nearest_ansi(p.err);
        p.lang = p.lang.downgraded();
        p
    }

    /// 列表选中行样式：有彩走底色，无色模式退回反色。
    ///
    /// `mono` 下 `selection_bg` 已被抹成 `Reset`，只靠底色选中态会彻底消失，
    /// 故这里必须降级到 `REVERSED`（这是无色模式的兜底分支，不是主路径）。
    #[must_use]
    pub(crate) fn selection(&self, fg: Color) -> Style {
        if self.mono {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new().fg(fg).bg(self.selection_bg)
        }
    }

    /// 按主题 + 终端能力解析出最终配色板（启动时一次）。
    #[must_use]
    pub(crate) fn resolve(theme: Option<&str>) -> Self {
        Self::resolve_with(theme, &|k| std::env::var(k).ok())
    }

    /// [`Self::resolve`] 的可注入版本（单测不依赖真实环境变量）。
    ///
    /// 判定顺序：`CM_TUI_THEME`（本地主题名覆盖）→ serve 的 prefs 主题；色深再看
    /// [`term_image::color_depth`]（`CM_TUI_COLOR` 显式覆盖 → `NO_COLOR` / `TERM=dumb`
    /// → 真彩 → 256 → 16）。配色与背景档位读同一个函数，否则迟早会漂移成「配色用真彩、
    /// 图像按 16 色」或「配色无色、背景彩色」这种自相矛盾的状态。
    #[must_use]
    pub(crate) fn resolve_with(theme: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> Self {
        // `CM_TUI_THEME` 只认已知预设：拼错时**忽略覆盖**（继续用 serve 的 prefs），
        // 而不是静默退成默认深色——后者会让人以为「透明主题没生效」却查不出原因。
        let forced = env("CM_TUI_THEME")
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| THEME_NAMES.contains(&v.as_str()));
        let p = Self::from_theme(forced.as_deref().or(theme));
        match term_image::color_depth(env) {
            ColorDepth::Mono => Self::mono(),
            ColorDepth::TrueColor => p,
            // 只有 16 色降级表；256 色可用的终端也不做索引色映射，直接退 16 色。
            ColorDepth::Ansi16 | ColorDepth::Ansi256 => p.downgrade_basic(),
        }
    }
}

/// xterm 默认 16 色调色板（顺序与 ratatui `Color` 命名一致）。
const ANSI16: [(Color, u8, u8, u8); 16] = [
    (Color::Black, 0, 0, 0),
    (Color::Red, 205, 0, 0),
    (Color::Green, 0, 205, 0),
    (Color::Yellow, 205, 205, 0),
    (Color::Blue, 0, 0, 238),
    (Color::Magenta, 205, 0, 205),
    (Color::Cyan, 0, 205, 205),
    (Color::Gray, 229, 229, 229),
    (Color::DarkGray, 127, 127, 127),
    (Color::LightRed, 255, 0, 0),
    (Color::LightGreen, 0, 255, 0),
    (Color::LightYellow, 255, 255, 0),
    (Color::LightBlue, 92, 92, 255),
    (Color::LightMagenta, 255, 0, 255),
    (Color::LightCyan, 0, 255, 255),
    (Color::White, 255, 255, 255),
];

/// 给整行套一层样式补丁（折叠光标用：底色 / 反色是「这一条就是当前那个把手」的唯一提示）。
///
/// 放在**渲染之后**调用，而不是把「选中」编进 `line_rows` / body 缓存的指纹：选中态
/// 每按一次 `↑/↓` 就变一次，进指纹等于每按一下就让那一行重建，还会把 `CachedLine` 的
/// 字段面再抬一个。与 `render::with_progress_mark`（spinner 帧）同一条路子。
///
/// 落在本模块是因为它只跟 [`Palette::selection`] 配套使用 —— 那是「选中态怎么表达」的
/// 唯一定义（无色模式退反色），而把它放进 `render.rs` 会让那个文件继续贴着 920 行门禁。
pub(crate) fn with_row_style(line: Line<'static>, patch: Style) -> Line<'static> {
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|s| Span::styled(s.content, s.style.patch(patch)))
        .collect();
    Line::from(spans)
}

fn is_gray_scale(c: Color) -> bool {
    matches!(
        c,
        Color::Black | Color::DarkGray | Color::Gray | Color::White
    )
}

fn sq_dist(r: u8, g: u8, b: u8, cr: u8, cg: u8, cb: u8) -> u32 {
    let d = |x: u8, y: u8| {
        let v = u32::from(x).abs_diff(u32::from(y));
        v * v
    };
    d(r, cr) + d(g, cg) + d(b, cb)
}

/// `Rgb` → 最近的 16 色；彩度明显时只在彩色候选里选，避免有色 accent 被灰阶抢走。
fn nearest_ansi(c: Color) -> Color {
    let Color::Rgb(r, g, b) = c else {
        return c;
    };
    let chroma = u8::max(u8::max(r, g), b) - u8::min(u8::min(r, g), b);
    let chromatic_only = chroma >= 32;
    ANSI16
        .iter()
        .filter(|(color, ..)| !chromatic_only || !is_gray_scale(*color))
        .min_by_key(|(_, cr, cg, cb)| sq_dist(r, g, b, *cr, *cg, *cb))
        .map_or(c, |(color, ..)| *color)
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
