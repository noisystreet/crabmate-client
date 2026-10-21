//! 终端**像素图能力探测**：只回答「这台终端能画什么」，不做任何渲染。
//!
//! TUI 想画背景/行内位图，可选档位按代价从低到高是：
//!
//! | 档位 | 分辨率 | 能否当背景层 |
//! |---|---|---|
//! | 半块字符 `▀` | 每格 1×2 像素 | 能，但与文字抢同一格 |
//! | iTerm2 inline image | 真实像素 | 难（在 ratatui `Buffer` 之外） |
//! | Sixel | 真实像素 | 难（同上） |
//! | Kitty graphics protocol | 真实像素 | 能（可置于文字下方） |
//!
//! 探测**只看环境变量，不做主动查询**（DA1 / `XTGETTCAP` / Kitty 的 `a=q`）。
//! 主动查询要先切 raw mode、再带超时读回应答，而 stdin 同时被全屏事件循环和
//! readline 抢着读：读串行就会吃掉用户按键或把启动挂住。所以这里全部按终端名 +
//! 已知支持矩阵**猜**，并且**宁可漏报也不误报**——误报 Sixel 会在屏幕上留下乱码，
//! 比没有背景图糟得多。
//!
//! 由此带来两条保守规则：
//!
//! 1. 处在 tmux / screen 里时，默认**不用**像素协议（透传是否开启无从确认），
//!    退回半块；确知已开 `allow-passthrough` 可用 `CM_TUI_IMAGE=kitty` 强制。
//! 2. 探测结果目前只被 `/background` 消费：`Kitty` 走真像素，其余档退到「每格
//!    底色」（不依赖终端协议）。transcript 里的 Markdown 图片**仍**降级成
//!    `[图] alt` 文本（见 `md_image.rs`），不走本模块。
//!
//! 颜色深度判定与 [`super::palette`] 共用 [`color_depth`]，避免两处各判一次真彩而漂移。

use std::fmt::Write as _;

/// 终端颜色深度（由低到高；顺序即 `PartialOrd` 的比较序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ColorDepth {
    /// `NO_COLOR` / `TERM=dumb`：颜色一律不用。
    Mono,
    /// 16 色。半块画出来糊成一团，不如不画。
    Ansi16,
    /// 256 色。
    Ansi256,
    /// 24 位真彩。
    TrueColor,
}

impl ColorDepth {
    /// 有颜色可用？（无色模式下任何依赖颜色的像素方案都不成立。）
    fn is_color(self) -> bool {
        self != Self::Mono
    }

    /// 半块字符要靠 `fg` + `bg` 两个颜色拼出「两个像素」，16 色下色阶太少，
    /// 画出来是噪声而不是图，故门槛定在 256 色。
    pub(crate) fn can_half_block(self) -> bool {
        self >= Self::Ansi256
    }

    fn name(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Ansi16 => "ansi16",
            Self::Ansi256 => "ansi256",
            Self::TrueColor => "truecolor",
        }
    }
}

/// 终端复用器：图像要透传才能穿过去，而透传是否开启我们无从确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Multiplexer {
    Tmux,
    Screen,
}

impl Multiplexer {
    fn name(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Screen => "screen",
        }
    }
}

/// 可用的像素图协议（声明顺序即优先级：Kitty 最强，且唯一支持置于文字下方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageProtocol {
    Kitty,
    Sixel,
    Iterm2,
}

impl ImageProtocol {
    fn name(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Sixel => "sixel",
            Self::Iterm2 => "iterm2",
        }
    }
}

/// 探测结论：最终该用哪一档画图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImagePick {
    Kitty,
    Sixel,
    Iterm2,
    /// 半块字符（纯 ratatui 可画，需 256 色以上）。
    HalfBlock,
    /// 不画图：保持当前的 `[图] alt` 文本降级。
    Off,
}

impl ImagePick {
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Sixel => "sixel",
            Self::Iterm2 => "iterm2",
            Self::HalfBlock => "halfblock",
            Self::Off => "none",
        }
    }
}

/// 参与判定的环境变量快照（一次性读出，纯数据，便于展示与测试）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TermEnv {
    term: Option<String>,
    term_program: Option<String>,
    colorterm: Option<String>,
    lc_terminal: Option<String>,
    kitty_window_id: bool,
    /// Konsole 只设 `KONSOLE_VERSION`，不设 `TERM_PROGRAM`——只看后者会把它漏掉。
    konsole_version: Option<String>,
    tmux: bool,
    screen: bool,
    no_color: bool,
    /// `CM_TUI_COLOR` 显式覆盖（`none` / `basic` / `truecolor`）：用户的明确指定，
    /// 优先于任何探测。配色与背景档位都按它落，避免「配色无色、背景彩色」。
    color_mode: Option<String>,
}

impl TermEnv {
    /// 读快照；值统一 trim + 转小写 + 空串视为未设置（空 `TERM` 与缺失同义）。
    fn read(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let lower = |k: &str| {
            env(k)
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
        };
        let set = |k: &str| env(k).is_some_and(|v| !v.trim().is_empty());
        Self {
            term: lower("TERM"),
            term_program: lower("TERM_PROGRAM"),
            colorterm: lower("COLORTERM"),
            lc_terminal: lower("LC_TERMINAL"),
            kitty_window_id: set("KITTY_WINDOW_ID"),
            konsole_version: lower("KONSOLE_VERSION"),
            tmux: set("TMUX"),
            screen: set("STY"),
            no_color: set("NO_COLOR"),
            color_mode: lower("CM_TUI_COLOR"),
        }
    }

    fn is_dumb(&self) -> bool {
        self.term.as_deref() == Some("dumb")
    }

    fn is_truecolor(&self) -> bool {
        let colorterm = self
            .colorterm
            .as_deref()
            .is_some_and(|v| v.contains("truecolor") || v.contains("24bit"));
        let term = self.term.as_deref().is_some_and(|t| {
            t.contains("truecolor") || t.contains("-direct") || t.contains("direct-")
        });
        colorterm || term
    }

    /// 判定顺序：`CM_TUI_COLOR` 显式覆盖 → `NO_COLOR` / `TERM=dumb` → 真彩 → 256 → 16。
    ///
    /// `CM_TUI_COLOR` 判在这里而不是留在 `palette`：配色与背景档位必须同一份结论，
    /// 否则会出现「配色无色、背景彩色」这种自相矛盾的状态。
    fn depth(&self) -> ColorDepth {
        match self.color_mode.as_deref() {
            Some("none" | "mono" | "off") => return ColorDepth::Mono,
            Some("basic" | "16") => return ColorDepth::Ansi16,
            Some("truecolor" | "24bit" | "256") => return ColorDepth::TrueColor,
            _ => {}
        }
        if self.no_color || self.is_dumb() {
            return ColorDepth::Mono;
        }
        if self.is_truecolor() {
            return ColorDepth::TrueColor;
        }
        if self.term.as_deref().is_some_and(|t| t.contains("256")) {
            return ColorDepth::Ansi256;
        }
        ColorDepth::Ansi16
    }

    fn multiplexer(&self) -> Option<Multiplexer> {
        if self.tmux || self.term.as_deref().is_some_and(|t| t.starts_with("tmux")) {
            return Some(Multiplexer::Tmux);
        }
        if self.screen
            || self
                .term
                .as_deref()
                .is_some_and(|t| t.starts_with("screen"))
        {
            return Some(Multiplexer::Screen);
        }
        None
    }

    /// Kitty graphics：kitty / ghostty / Konsole / Warp，另有 `KITTY_WINDOW_ID` 直接证据。
    ///
    /// Konsole 与 Warp 都在 kitty 官方「已实现该协议」清单里，但**都不设** `TERM_PROGRAM`
    /// 的通用值：Konsole 只给 `KONSOLE_VERSION`，Warp 给 `TERM_PROGRAM=WarpTerminal`。
    /// 只按 `TERM` 猜会把它们漏成半块（用户实测踩过）。
    ///
    /// 这一档可以比 Sixel 宽松：kitty 协议走 `ESC _G`（APC），不认识的终端**直接忽略**，
    /// 猜错的代价只是「没有背景图」；Sixel 猜错会往屏幕上糊乱码。Konsole 的 kitty 支持
    /// 也是版本相关的（22.04 起有 Sixel，kitty 协议更晚），老版本上会静默不显示。
    ///
    /// **WezTerm 不算**：它确实实现了 kitty 协议，但 ratatui-image 的官方 QA 矩阵把它
    /// 记成「kitty / sixel 都支持、却只有 iTerm2 真正无 bug」，所以这里让位给 `has_iterm2`。
    fn has_kitty(&self) -> bool {
        self.kitty_window_id
            || self.konsole_version.is_some()
            || self
                .term
                .as_deref()
                .is_some_and(|t| t.contains("kitty") || t.contains("ghostty"))
            || self
                .term_program
                .as_deref()
                .is_some_and(|p| matches!(p, "ghostty" | "xterm-kitty" | "warpterminal"))
    }

    /// Sixel 白名单刻意收窄：`xterm` 只有在编进 sixel 支持时才会把 `TERM` 写成
    /// `xterm-sixel`，而 konsole / VSCode 之类「版本相关、默认可能关着」的终端
    /// 一律不算，避免误报后把乱码糊满屏。WezTerm 同理排除（理由见 `has_kitty`）。
    fn has_sixel(&self) -> bool {
        const TERM_HINTS: [&str; 4] = ["sixel", "mlterm", "yaft", "foot"];
        const PROGRAM_HINTS: [&str; 3] = ["ghostty", "mintty", "foot"];
        let term_hit = self
            .term
            .as_deref()
            .is_some_and(|t| TERM_HINTS.iter().any(|k| t.contains(k)));
        let program_hit = self
            .term_program
            .as_deref()
            .is_some_and(|p| PROGRAM_HINTS.iter().any(|k| p.contains(k)));
        term_hit || program_hit
    }

    /// iTerm2 inline image：iTerm2 本体，以及只在这一档无 bug 的 WezTerm。
    fn has_iterm2(&self) -> bool {
        self.term_program
            .as_deref()
            .is_some_and(|p| p == "iterm.app" || p == "wezterm")
            || self
                .lc_terminal
                .as_deref()
                .is_some_and(|l| l.contains("iterm2"))
    }

    fn protocols(&self) -> Vec<ImageProtocol> {
        let mut out = Vec::new();
        if self.has_kitty() {
            out.push(ImageProtocol::Kitty);
        }
        if self.has_sixel() {
            out.push(ImageProtocol::Sixel);
        }
        if self.has_iterm2() {
            out.push(ImageProtocol::Iterm2);
        }
        out
    }
}

/// 一次探测的完整结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TermImageCaps {
    env: TermEnv,
    depth: ColorDepth,
    protocols: Vec<ImageProtocol>,
    mux: Option<Multiplexer>,
    /// `CM_TUI_IMAGE` 显式指定（`auto` / 未知值 → `None`）。
    forced: Option<ImagePick>,
    /// `CM_TUI_IMAGE` 的原始值：`auto` 与拼错的值都不会进 `forced`，但诊断要看得见。
    raw_image: Option<String>,
}

impl TermImageCaps {
    /// 读真实环境变量。
    #[must_use]
    pub(crate) fn probe() -> Self {
        Self::probe_with(&|k| std::env::var(k).ok())
    }

    /// [`Self::probe`] 的可注入版本（单测不依赖真实环境变量）。
    #[must_use]
    pub(crate) fn probe_with(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let snapshot = TermEnv::read(env);
        let depth = snapshot.depth();
        let protocols = snapshot.protocols();
        let mux = snapshot.multiplexer();
        let raw_image = env("CM_TUI_IMAGE")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let forced = parse_override(raw_image.as_deref());
        Self {
            env: snapshot,
            depth,
            protocols,
            mux,
            forced,
            raw_image,
        }
    }

    #[must_use]
    pub(crate) fn depth(&self) -> ColorDepth {
        self.depth
    }

    /// 当前终端**报称**支持的协议（已按优先级排序；不含强制覆盖）。
    #[must_use]
    pub(crate) fn protocols(&self) -> &[ImageProtocol] {
        &self.protocols
    }

    #[must_use]
    pub(crate) fn mux(&self) -> Option<Multiplexer> {
        self.mux
    }

    /// 最终该用哪一档。`caps` 子命令用它做诊断，`/background` 用它选落点
    /// （`Kitty` → 真像素，其余 → 每格底色）。
    #[must_use]
    pub(crate) fn pick(&self) -> ImagePick {
        match self.forced {
            // 强制值一律尊重（用户可能正想自测），唯半块在无 256 色时是**做不到**
            // 而非「不合口味」，退回 Off 而不是画出噪声。
            Some(ImagePick::HalfBlock) if !self.depth.can_half_block() => ImagePick::Off,
            Some(forced) => forced,
            None => self.auto_pick(),
        }
    }

    /// 协议列表的一行文本，供 `describe` 与 `summary` 共用（避免两处格式漂移）。
    fn protocols_label(&self) -> String {
        if self.protocols().is_empty() {
            "(none)".to_string()
        } else {
            self.protocols()
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// 单行摘要：写进「为什么不能用」的提示，省得用户另开窗口跑 `caps`。
    #[must_use]
    pub(crate) fn summary(&self) -> String {
        format!(
            "TERM={} · TERM_PROGRAM={} · mux={} · protocols={} → picked={}",
            self.env.term.as_deref().unwrap_or("(unset)"),
            self.env.term_program.as_deref().unwrap_or("(unset)"),
            self.mux().map_or("(none)", Multiplexer::name),
            self.protocols_label(),
            self.pick().name(),
        )
    }

    fn auto_pick(&self) -> ImagePick {
        if !self.depth.is_color() {
            return ImagePick::Off;
        }
        // 复用器里透传未确认：默认不冒乱码风险，退回半块。
        if self.mux.is_none() {
            match self.protocols.first() {
                Some(ImageProtocol::Kitty) => return ImagePick::Kitty,
                Some(ImageProtocol::Sixel) => return ImagePick::Sixel,
                Some(ImageProtocol::Iterm2) => return ImagePick::Iterm2,
                None => {}
            }
        }
        if self.depth.can_half_block() {
            ImagePick::HalfBlock
        } else {
            ImagePick::Off
        }
    }

    /// 人类可读的多行诊断（`crabmate-tui caps` 直接打印）。
    #[must_use]
    pub(crate) fn describe(&self) -> String {
        fn line(out: &mut String, key: &str, value: &str) {
            // 忽略写入错误：目标是 String，`write!` 不会失败。
            let _ = writeln!(out, "{key:<16} {value}");
        }
        fn or_unset(v: Option<&str>) -> &str {
            v.unwrap_or("(unset)")
        }

        let mut out = String::new();
        line(&mut out, "color depth", self.depth().name());
        line(&mut out, "picked", self.pick().name());
        line(&mut out, "TERM", or_unset(self.env.term.as_deref()));
        line(
            &mut out,
            "TERM_PROGRAM",
            or_unset(self.env.term_program.as_deref()),
        );
        line(
            &mut out,
            "COLORTERM",
            or_unset(self.env.colorterm.as_deref()),
        );
        line(
            &mut out,
            "LC_TERMINAL",
            or_unset(self.env.lc_terminal.as_deref()),
        );
        line(
            &mut out,
            "KONSOLE_VERSION",
            or_unset(self.env.konsole_version.as_deref()),
        );
        line(
            &mut out,
            "multiplexer",
            self.mux().map_or("(none)", Multiplexer::name),
        );
        line(&mut out, "protocols", &self.protocols_label());
        line(
            &mut out,
            "CM_TUI_IMAGE",
            or_unset(self.raw_image.as_deref()),
        );
        if self.mux().is_some() {
            line(
                &mut out,
                "note",
                "复用器内不确认透传：默认退回半块，必要时用 CM_TUI_IMAGE=kitty 强制",
            );
        }
        out
    }
}

/// 环境变量判定出的颜色深度（与 [`super::palette`] 共用）。
#[must_use]
pub(crate) fn color_depth(env: &dyn Fn(&str) -> Option<String>) -> ColorDepth {
    TermEnv::read(env).depth()
}

/// `CM_TUI_IMAGE` 显式覆盖（`none`/`halfblock`/`sixel`/`kitty`/`iterm2`）；
/// `auto` / 未设置 / 未知值返回 `None`（交给自动判定）。
fn parse_override(raw: Option<&str>) -> Option<ImagePick> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "0" => Some(ImagePick::Off),
        "half" | "halfblock" | "blocks" => Some(ImagePick::HalfBlock),
        "sixel" => Some(ImagePick::Sixel),
        "kitty" => Some(ImagePick::Kitty),
        "iterm" | "iterm2" | "inline" => Some(ImagePick::Iterm2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用固定键值表造一个 env 闭包（未列出的键一律 `None`）。
    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    fn caps(pairs: &'static [(&'static str, &'static str)]) -> TermImageCaps {
        TermImageCaps::probe_with(&env_of(pairs))
    }

    #[test]
    fn kitty_terminal_picks_kitty() {
        let c = caps(&[
            ("TERM", "xterm-kitty"),
            ("COLORTERM", "truecolor"),
            ("KITTY_WINDOW_ID", "1"),
        ]);
        assert_eq!(c.depth(), ColorDepth::TrueColor);
        assert_eq!(c.protocols(), &[ImageProtocol::Kitty]);
        assert_eq!(c.mux(), None);
        assert_eq!(c.pick(), ImagePick::Kitty);
    }

    #[test]
    fn wezterm_falls_back_to_iterm2() {
        // WezTerm 三种协议都实现了，但 ratatui-image 的 QA 矩阵只把 iTerm2 记为无 bug
        // （kitty / sixel 都「支持」却有问题），所以它不抢 Kitty 的首选位。
        let c = caps(&[("TERM_PROGRAM", "WezTerm"), ("COLORTERM", "truecolor")]);
        assert_eq!(c.protocols(), &[ImageProtocol::Iterm2]);
        assert_eq!(c.pick(), ImagePick::Iterm2);
    }

    #[test]
    fn konsole_is_detected_by_konsole_version() {
        // Konsole 不设 TERM_PROGRAM，只给 KONSOLE_VERSION；只看 TERM_PROGRAM 会把它漏成半块。
        let c = caps(&[
            ("TERM", "xterm-256color"),
            ("KONSOLE_VERSION", "240800"),
            ("COLORTERM", "truecolor"),
        ]);
        assert_eq!(c.protocols(), &[ImageProtocol::Kitty]);
        assert_eq!(c.pick(), ImagePick::Kitty);
    }

    #[test]
    fn warp_is_detected_by_term_program() {
        // Warp 用自己的 TERM_PROGRAM 值，且 TERM 不带任何关键词。
        let c = caps(&[
            ("TERM", "xterm-256color"),
            ("TERM_PROGRAM", "WarpTerminal"),
            ("COLORTERM", "truecolor"),
        ]);
        assert_eq!(c.protocols(), &[ImageProtocol::Kitty]);
        assert_eq!(c.pick(), ImagePick::Kitty);
    }

    #[test]
    fn summary_names_the_reason_it_fell_back() {
        // 半块降级的典型场景：复用器里像素协议不予采信。
        let c = caps(&[
            ("TERM", "screen-256color"),
            ("TMUX", "/tmp/tmux-1000/default,1,0"),
        ]);
        let s = c.summary();
        assert!(s.contains("TERM=screen-256color"), "要带 TERM：{s}");
        assert!(s.contains("mux=tmux"), "要带复用器：{s}");
        assert!(s.contains("picked=halfblock"), "要带最终判定：{s}");
    }

    #[test]
    fn iterm2_reports_inline_protocol() {
        let c = caps(&[
            ("TERM_PROGRAM", "iTerm.app"),
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
        ]);
        assert_eq!(c.protocols(), &[ImageProtocol::Iterm2]);
        assert_eq!(c.pick(), ImagePick::Iterm2);
    }

    #[test]
    fn plain_256_color_falls_back_to_half_block() {
        let c = caps(&[("TERM", "xterm-256color")]);
        assert_eq!(c.depth(), ColorDepth::Ansi256);
        assert!(c.protocols().is_empty());
        assert_eq!(c.pick(), ImagePick::HalfBlock);
    }

    #[test]
    fn ansi16_without_protocol_has_no_image() {
        // 16 色下半块只会画出噪声，宁可不画。
        let c = caps(&[("TERM", "xterm")]);
        assert_eq!(c.depth(), ColorDepth::Ansi16);
        assert_eq!(c.pick(), ImagePick::Off);
    }

    #[test]
    fn no_color_and_dumb_disable_images() {
        let c = caps(&[
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("NO_COLOR", "1"),
        ]);
        assert_eq!(c.depth(), ColorDepth::Mono);
        assert_eq!(c.pick(), ImagePick::Off);

        let c = caps(&[("TERM", "dumb"), ("COLORTERM", "truecolor")]);
        assert_eq!(c.depth(), ColorDepth::Mono);
        assert_eq!(c.pick(), ImagePick::Off);
    }

    #[test]
    fn multiplexer_backs_off_from_pixel_protocols() {
        // kitty 在 tmux 里，但透传没确认 → 不冒乱码风险。
        let c = caps(&[
            ("TERM", "xterm-kitty"),
            ("COLORTERM", "truecolor"),
            ("TMUX", "/tmp/tmux-1000/default,123,0"),
        ]);
        assert_eq!(c.mux(), Some(Multiplexer::Tmux));
        assert_eq!(c.protocols(), &[ImageProtocol::Kitty]);
        assert_eq!(c.pick(), ImagePick::HalfBlock);
    }

    #[test]
    fn screen_is_detected_from_term_prefix() {
        let c = caps(&[("TERM", "screen-256color")]);
        assert_eq!(c.mux(), Some(Multiplexer::Screen));
        assert_eq!(c.pick(), ImagePick::HalfBlock);
    }

    #[test]
    fn override_wins_over_probe() {
        let c = TermImageCaps::probe_with(&|k| match k {
            "TERM" => Some("xterm-256color".into()),
            "CM_TUI_IMAGE" => Some("none".into()),
            _ => None,
        });
        assert_eq!(c.pick(), ImagePick::Off, "CM_TUI_IMAGE=none 关闭画图");

        let c = TermImageCaps::probe_with(&|k| match k {
            "TERM" => Some("xterm-256color".into()),
            "CM_TUI_IMAGE" => Some("kitty".into()),
            _ => None,
        });
        assert_eq!(c.pick(), ImagePick::Kitty, "强制值即使探不到也照办");

        // 强制 halfblock 但只有 16 色 → 做不到，退回 Off。
        let c = TermImageCaps::probe_with(&|k| match k {
            "TERM" => Some("xterm".into()),
            "CM_TUI_IMAGE" => Some("halfblock".into()),
            _ => None,
        });
        assert_eq!(c.pick(), ImagePick::Off);

        // 未知值 / auto 交回自动判定。
        assert_eq!(
            caps(&[("TERM", "xterm-256color")]).pick(),
            ImagePick::HalfBlock
        );
    }

    #[test]
    fn describe_reports_pick_and_env() {
        let text = caps(&[("TERM", "xterm-kitty"), ("COLORTERM", "truecolor")]).describe();
        assert!(text.contains("picked           kitty"), "{text}");
        assert!(text.contains("TERM             xterm-kitty"), "{text}");
        assert!(text.contains("TERM_PROGRAM     (unset)"), "{text}");
        assert!(text.contains("color depth      truecolor"), "{text}");
    }

    #[test]
    fn color_mode_override_drives_both_palette_and_images() {
        // 配色与背景档位读同一个函数：`CM_TUI_COLOR=none` 必须让两者都无色。
        let c = caps(&[("TERM", "xterm-256color"), ("CM_TUI_COLOR", "none")]);
        assert_eq!(c.depth(), ColorDepth::Mono);
        assert_eq!(c.pick(), ImagePick::Off, "无色终端不该铺背景");

        // 显式 basic（16 色）画不出格子背景：色阶太少会变噪声。
        let c = caps(&[("TERM", "xterm-256color"), ("CM_TUI_COLOR", "basic")]);
        assert_eq!(c.depth(), ColorDepth::Ansi16);
        assert_eq!(c.pick(), ImagePick::Off);

        // 显式 truecolor 即使在 16 色的 TERM 上也按真彩走（用户的明确指定）。
        let c = caps(&[("TERM", "xterm"), ("CM_TUI_COLOR", "truecolor")]);
        assert_eq!(c.depth(), ColorDepth::TrueColor);
        assert_eq!(c.pick(), ImagePick::HalfBlock);
    }
}
