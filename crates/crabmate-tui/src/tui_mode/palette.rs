//! 终端配色板：一个 Desktop 主题 → 一组语义化终端颜色。
//!
//! 值取自 `frontend/styles/tokens.css`（默认深色基底）与 `frontend/themes/*.css`；
//! 主题名与 `UserPrefsDto.theme` 同一枚举，TUI 只读、不自建主题格式。
//! 浅色主题的聊天区底色取 `--surface-hover` 而非 `--surface`：整屏纯白在终端里刺眼，
//! 压暗一档后正文/次要信息都还够对比度。
//!
//! 终端能力差异由 [`Palette::resolve`] 兜底：无真彩时把 `Rgb` 降到最接近的 16 色，
//! `NO_COLOR` / `TERM=dumb` 时退到无色模式（靠修饰位区分，不靠颜色）。

use ratatui::style::Color;

/// 语义化配色板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// 正文（`--text`）。
    pub fg: Color,
    /// 次要信息：消息前缀、工具 note、提示（`--muted`）。
    pub muted: Color,
    /// 强调：链接、用户消息基色（`--accent`）。
    pub accent: Color,
    /// 搜索命中（非锚定行）；锚定行用它的反色块。独立于 `accent`，
    /// 否则会与用户消息的强调色混淆。
    pub hit: Color,
    /// 聊天区底色。
    pub bg: Color,
    /// 顶栏 / 状态栏 / 侧栏 / composer 底色（比 `bg` 再压一档，形成条带）。
    pub chrome_bg: Color,
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
    hit: rgb(0xe6, 0xc4, 0x5c),
    bg: rgb(0x16, 0x17, 0x27),
    chrome_bg: rgb(0x10, 0x11, 0x1f),
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
    hit: rgb(0xff, 0xe0, 0x66),
    bg: rgb(0x24, 0x24, 0x24),
    chrome_bg: rgb(0x1a, 0x1a, 0x1a),
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
    hit: rgb(0xe6, 0xc4, 0x5c),
    bg: rgb(0x1b, 0x1b, 0x1f),
    chrome_bg: rgb(0x14, 0x14, 0x17),
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
    hit: rgb(0xf5, 0xd0, 0x5a),
    bg: rgb(0x18, 0x18, 0x1b),
    chrome_bg: rgb(0x10, 0x10, 0x12),
    border: rgb(0x27, 0x27, 0x2a),
    code_bg: rgb(0x27, 0x27, 0x2a),
    ok: rgb(0x6c, 0xc4, 0x9a),
    err: rgb(0xf2, 0x77, 0x7a),
    mono: false,
};

/// `themes/crabmate-light.css`：聊天区底色用 `--surface-hover`（`#f0ece2`）压暗一档。
const CRABMATE_LIGHT: Palette = Palette {
    fg: rgb(0x1c, 0x19, 0x13),
    muted: rgb(0x6b, 0x63, 0x57),
    accent: rgb(0x55, 0x70, 0x5b),
    hit: rgb(0x8a, 0x6d, 0x00),
    bg: rgb(0xf0, 0xec, 0xe2),
    chrome_bg: rgb(0xf7, 0xf4, 0xec),
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
    hit: rgb(0x8a, 0x6d, 0x00),
    bg: rgb(0xf4, 0xf4, 0xf5),
    chrome_bg: rgb(0xec, 0xec, 0xed),
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
    hit: Color::Reset,
    bg: Color::Reset,
    chrome_bg: Color::Reset,
    border: Color::Reset,
    code_bg: Color::Reset,
    ok: Color::Reset,
    err: Color::Reset,
    mono: true,
};

impl Default for Palette {
    fn default() -> Self {
        DEFAULT_DARK
    }
}

impl Palette {
    /// Desktop 主题名 → 配色板；未知 / 空 → 默认深色（不回落到不可读的随机取值）。
    #[must_use]
    pub(crate) fn from_theme(theme: Option<&str>) -> Self {
        match theme.map(str::trim).unwrap_or_default() {
            "crabmate-light" => CRABMATE_LIGHT,
            "shadcn-light" => SHADCN_LIGHT,
            "shadcn-dark" => SHADCN_DARK,
            "material-dark" => MATERIAL_DARK,
            "high-contrast-dark" => HIGH_CONTRAST_DARK,
            _ => DEFAULT_DARK,
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
        p.hit = nearest_ansi(p.hit);
        p.bg = nearest_ansi(p.bg);
        p.chrome_bg = nearest_ansi(p.chrome_bg);
        p.border = nearest_ansi(p.border);
        p.code_bg = nearest_ansi(p.code_bg);
        p.ok = nearest_ansi(p.ok);
        p.err = nearest_ansi(p.err);
        p
    }

    /// 按主题 + 终端能力解析出最终配色板（启动时一次）。
    #[must_use]
    pub(crate) fn resolve(theme: Option<&str>) -> Self {
        Self::resolve_with(theme, &|k| std::env::var(k).ok())
    }

    /// [`Self::resolve`] 的可注入版本（单测不依赖真实环境变量）。
    ///
    /// 判定顺序：`CM_TUI_COLOR` 显式覆盖 → `NO_COLOR`/`TERM=dumb` → 真彩检测 → 降级。
    #[must_use]
    pub(crate) fn resolve_with(theme: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let lower = |k: &str| env(k).map(|v| v.trim().to_ascii_lowercase());
        if let Some(forced) = color_mode_override(lower("CM_TUI_COLOR").as_deref(), theme) {
            return forced;
        }
        if env("NO_COLOR").is_some_and(|v| !v.trim().is_empty()) || is_dumb_terminal(&lower) {
            return Self::mono();
        }
        let p = Self::from_theme(theme);
        if is_truecolor(&lower) {
            p
        } else {
            p.downgrade_basic()
        }
    }
}

/// `CM_TUI_COLOR` 显式覆盖（`none`/`basic`/`truecolor`）；未设置或未知值返回 `None`。
fn color_mode_override(mode: Option<&str>, theme: Option<&str>) -> Option<Palette> {
    match mode? {
        "none" | "mono" | "off" => Some(Palette::mono()),
        "basic" | "16" => Some(Palette::from_theme(theme).downgrade_basic()),
        "truecolor" | "24bit" | "256" => Some(Palette::from_theme(theme)),
        _ => None,
    }
}

fn is_dumb_terminal(lower: &dyn Fn(&str) -> Option<String>) -> bool {
    lower("TERM").is_some_and(|t| t == "dumb")
}

fn is_truecolor(lower: &dyn Fn(&str) -> Option<String>) -> bool {
    let colorterm =
        lower("COLORTERM").is_some_and(|v| v.contains("truecolor") || v.contains("24bit"));
    let term = lower("TERM")
        .is_some_and(|t| t.contains("truecolor") || t.contains("-direct") || t.contains("direct-"));
    colorterm || term
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
        // 未知 / 空 / None 一律回默认深色（不回落到不可读取值）。
        assert_eq!(Palette::from_theme(Some("nope")), DEFAULT_DARK);
        assert_eq!(Palette::from_theme(None), Palette::default());
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
}
