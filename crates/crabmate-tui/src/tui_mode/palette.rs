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

use super::term_image::{self, ColorDepth};

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
    /// 聊天区底色。
    pub bg: Color,
    /// 状态栏 / 侧栏 / composer 底色（比 `bg` 再压一档，形成条带）。
    pub chrome_bg: Color,
    /// 列表选中行底色（桌面 `--surface-hover` 一档）。
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
    /// 工具成功。
    pub ok: Color,
    /// 工具失败 / 系统告警。
    pub err: Color,
    /// 无色模式：所有颜色为 `Reset`，区分只靠 `DIM` / `BOLD` / `REVERSED`。
    pub mono: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
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
    mono: true,
};

/// 结构体兜底：**实底深色**，与 [`Palette::from_theme`] 的「未设置 → 透明」刻意不同。
///
/// 两者分工不同：`from_theme(None)` 表达产品默认（透明），而 `Default` 只给测试夹具与
/// `..Default::default()` 一个「有底色、可读」的安全基线。若把 `Default` 也做成透明，
/// 任何拿它当夹具的用例都会静默失去条带色（`render_tests` / `status` 里就有十几处）。
impl Default for Palette {
    fn default() -> Self {
        DEFAULT_DARK
    }
}

impl Palette {
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
mod tests {
    use super::*;

    #[test]
    fn known_themes_map_and_unknown_falls_back() {
        assert_eq!(Palette::from_theme(Some("shadcn-dark")), SHADCN_DARK);
        assert_eq!(
            Palette::from_theme(Some(" crabmate-light ")),
            CRABMATE_LIGHT
        );
        assert_eq!(
            Palette::from_theme(Some("high-contrast-dark")),
            HIGH_CONTRAST_DARK
        );
        // 旧的默认深色保留为显式可选项（透明成为默认后的回退入口）。
        assert_eq!(Palette::from_theme(Some("default-dark")), DEFAULT_DARK);
        // 未设置 / 空串 → 透明（产品默认）。
        assert_eq!(Palette::from_theme(None), SHADCN_TRANSPARENT);
        assert_eq!(Palette::from_theme(Some("")), SHADCN_TRANSPARENT);
        // 不认识的名字 → 实底深色，而不是透明：静默变透明等于「一片黑、无处归因」。
        assert_eq!(Palette::from_theme(Some("nope")), DEFAULT_DARK);
        // `Default` 与 `from_theme(None)` 刻意解耦：结构体兜底要「有底色、可读」。
        assert_eq!(Palette::default(), DEFAULT_DARK);
    }

    #[test]
    fn light_themes_dim_the_chat_background() {
        // 浅色主题聊天区底色取 --surface-hover（压暗一档），不等于纯白。
        assert_eq!(CRABMATE_LIGHT.bg, rgb(0xf0, 0xec, 0xe2));
        assert_eq!(SHADCN_LIGHT.bg, rgb(0xf4, 0xf4, 0xf5));
        assert_ne!(SHADCN_LIGHT.bg, rgb(0xff, 0xff, 0xff));
        // 浅色主题正文是深色，深色主题正文是浅色。
        assert!(luma(CRABMATE_LIGHT.fg) < luma(CRABMATE_LIGHT.bg));
        assert!(luma(DEFAULT_DARK.fg) > luma(DEFAULT_DARK.bg));
    }

    fn luma(c: Color) -> u32 {
        match c {
            Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
            _ => 0,
        }
    }

    #[test]
    fn downgrade_leaves_no_rgb() {
        let p = Palette::from_theme(Some("shadcn-dark")).downgrade_basic();
        for c in [
            p.fg,
            p.muted,
            p.accent,
            p.hit,
            p.bg,
            p.chrome_bg,
            p.border,
            p.code_bg,
            p.ok,
            p.err,
        ] {
            assert!(
                !matches!(c, Color::Rgb(..)),
                "16 色模式下不应残留 Rgb: {c:?}"
            );
        }
    }

    #[test]
    fn nearest_ansi_picks_sensible_16_color() {
        // 深底 → 黑；浅字 → xterm 的 color7（#e5e5e5，比纯白更接近 #eef1f6）；
        // 蓝紫 accent → 亮蓝（不被灰阶抢走）。
        assert_eq!(nearest_ansi(rgb(0x16, 0x17, 0x27)), Color::Black);
        assert_eq!(nearest_ansi(rgb(0xee, 0xf1, 0xf6)), Color::Gray);
        assert_eq!(nearest_ansi(rgb(0x8f, 0xa4, 0xea)), Color::LightBlue);
        assert_eq!(nearest_ansi(rgb(0xff, 0xff, 0xff)), Color::White);
        // 命名色原样返回
        assert_eq!(nearest_ansi(Color::LightGreen), Color::LightGreen);
    }

    #[test]
    fn resolve_honours_terminal_capabilities() {
        let truecolor = |k: &str| match k {
            "COLORTERM" => Some("truecolor".to_string()),
            _ => None,
        };
        let p = Palette::resolve_with(Some("shadcn-dark"), &truecolor);
        assert_eq!(p, SHADCN_DARK, "真彩终端保留 Rgb");

        let plain = |_: &str| None;
        let p = Palette::resolve_with(Some("shadcn-dark"), &plain);
        assert_eq!(p, SHADCN_DARK.downgrade_basic(), "无真彩时降到 16 色");

        let no_color = |k: &str| (k == "NO_COLOR").then(|| "1".to_string());
        assert!(Palette::resolve_with(Some("shadcn-dark"), &no_color).is_mono());

        let dumb = |k: &str| (k == "TERM").then(|| "dumb".to_string());
        assert!(Palette::resolve_with(None, &dumb).is_mono());

        let forced = |k: &str| (k == "CM_TUI_COLOR").then(|| "basic".to_string());
        assert_eq!(
            Palette::resolve_with(Some("crabmate-light"), &forced),
            CRABMATE_LIGHT.downgrade_basic(),
            "CM_TUI_COLOR=basic 强制 16 色"
        );
    }

    #[test]
    fn mono_has_no_colors() {
        let m = Palette::mono();
        assert!(m.is_mono());
        assert_eq!(m.fg, Color::Reset);
        assert_eq!(m.accent, Color::Reset);
        // 无色模式下修饰位仍可用（调用方据此区分语义）。
        assert_eq!(m.downgrade_basic(), m);
    }

    #[test]
    fn transparent_theme_only_resets_the_two_background_slots() {
        let t = Palette::from_theme(Some("shadcn-transparent"));
        assert_eq!(t.bg, Color::Reset, "聊天区底色交给终端");
        assert_eq!(t.chrome_bg, Color::Reset, "条带底色也交给终端");
        assert_eq!(t.fg, SHADCN_DARK.fg, "前景沿用 shadcn-dark");
        assert_eq!(t.border, SHADCN_DARK.border);
        assert_ne!(t.border, Color::Reset, "边框必须是实色，否则整屏没有分区");
        assert_ne!(t.code_bg, Color::Reset, "代码块底色保持实色");
        assert!(!t.mono, "透明不等于无色：语义仍靠颜色区分");
    }

    #[test]
    fn transparent_slots_survive_the_16_colour_downgrade() {
        let p = SHADCN_TRANSPARENT.downgrade_basic();
        assert_eq!(p.bg, Color::Reset, "降级不该把「透出终端」变成某个索引色");
        assert_eq!(p.chrome_bg, Color::Reset);
        assert_ne!(p.border, Color::Reset);
    }

    #[test]
    fn every_listed_theme_name_maps_to_its_own_palette() {
        // 显式期望表：漏接 `from_theme` 的预设会落到 `Some(_)` 的深色兜底，等值断言立刻变红。
        let expected = [
            ("crabmate-light", CRABMATE_LIGHT),
            // 与 `tokens.css` 的 `:root` 深色基线同值；名字独立列入以便
            // `CM_TUI_THEME` 能显式选它。
            ("crabmate-dark", DEFAULT_DARK),
            ("shadcn-light", SHADCN_LIGHT),
            ("shadcn-dark", SHADCN_DARK),
            ("material-dark", MATERIAL_DARK),
            ("high-contrast-dark", HIGH_CONTRAST_DARK),
            ("shadcn-transparent", SHADCN_TRANSPARENT),
            ("default-dark", DEFAULT_DARK),
        ];
        assert_eq!(
            expected.len(),
            THEME_NAMES.len(),
            "THEME_NAMES 与期望表不同步"
        );
        for (name, want) in expected {
            assert_eq!(Palette::from_theme(Some(name)), want, "{name} 映射错了");
        }
    }

    /// 面板可选列表必须与 `THEME_NAMES` 一一对应。
    ///
    /// 两张表是手写的：只加名字忘了加选项，那个主题在面板里就选不到（`from_theme`
    /// 能映射、用户点不着）；只加选项忘了加名字，则选中后会被当成未知名静默回落到
    /// `default-dark`。上面那个测试只校验 `from_theme` 的映射，两种漏接都拦不住。
    #[test]
    fn theme_options_and_theme_names_stay_in_lockstep() {
        let options: Vec<&str> = THEME_OPTIONS.iter().flatten().copied().collect();
        assert_eq!(
            options.len() + 1,
            THEME_OPTIONS.len(),
            "除首项「默认」外，每个选项都该是一个具名主题"
        );
        assert_eq!(THEME_OPTIONS[0], None, "首项是清除 prefs 键的「默认」");
        for name in THEME_NAMES {
            assert!(
                options.contains(&name),
                "主题名 {name} 没有对应的面板选项，面板里选不到它"
            );
        }
        for name in &options {
            assert!(
                THEME_NAMES.contains(name),
                "面板选项 {name} 不在 THEME_NAMES 里，选中后会静默回落深色"
            );
        }
    }

    #[test]
    fn cm_tui_theme_overrides_the_serve_pref_but_only_for_known_names() {
        let env = |theme: &'static str| {
            move |k: &str| match k {
                "CM_TUI_THEME" => Some(theme.to_string()),
                "CM_TUI_COLOR" => Some("truecolor".to_string()),
                _ => None,
            }
        };
        // 覆盖生效：serve 给的是浅色，本地仍拿到透明深色。
        let p = Palette::resolve_with(Some("shadcn-light"), &env("shadcn-transparent"));
        assert_eq!(p, SHADCN_TRANSPARENT);

        // 未列出的主题名不生效：仍用 serve 的 prefs，而不是静默退默认深色。
        // 这里刻意不用「拼写接近的错名」——pre-commit 的 typos 钩子带自动修正
        // （`--write-changes`），会把错名当笔误改对，用例就失去意义了。
        let p = Palette::resolve_with(Some("crabmate-light"), &env("no-such-theme"));
        assert_eq!(p, CRABMATE_LIGHT, "未知主题名应忽略覆盖");
    }
    #[test]
    fn colourless_mode_outranks_a_theme_choice() {
        // `CM_TUI_COLOR=none` 的判定在主题之后：连透明主题也要退成 mono。
        // 这条守着判定顺序，避免将来把主题解析挪到色深之后。
        let env = |k: &str| match k {
            "CM_TUI_THEME" => Some("shadcn-transparent".to_string()),
            "CM_TUI_COLOR" => Some("none".to_string()),
            _ => None,
        };
        let p = Palette::resolve_with(Some("shadcn-dark"), &env);
        assert!(p.is_mono(), "无色模式应压过主题选择");
    }
}
