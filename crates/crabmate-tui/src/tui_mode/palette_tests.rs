//! `palette.rs` 的单测（`#[path]` 子模块）：主题映射、16 色降级、语法色槽与无色兜底。
//!
//! 拆出去是为了守住 `fn-nloc` 的 920 行上限 —— 主题表 + 语法色槽 + 语言色板已经让
//! `palette.rs` 顶到那条线上，用例留在里面谁都再难加东西（与
//! `ws_sidebar_file_kind_tests.rs` / `render_tests.rs` 是同一套做法）。

use super::*;

/// 折叠光标：整行套选中样式，**内容一字不改**（它只是把手，不是新的状态行）。
///
/// 只染行首那一格是常见的手滑 —— 那时光标看起来像「半个字符的强调」，而不是
/// 「当前这条」。无色模式走 `REVERSED`，色值本身由其它用例管。
#[test]
fn row_style_patches_every_span_and_keeps_the_text() {
    let line = Line::from(vec![Span::raw("▸ "), Span::raw("2 个工具 · 完成")]);
    let styled = with_row_style(line, Style::new().bg(Color::Red));

    assert_eq!(styled.spans.len(), 2);
    assert!(
        styled.spans.iter().all(|s| s.style.bg == Some(Color::Red)),
        "每个 span 都要带上底色"
    );
    let text: String = styled.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "▸ 2 个工具 · 完成", "内容不该被改写");
}

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
    let l = p.lang;
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
        l.rust,
        l.python,
        l.js,
        l.ts,
        l.go,
        l.jvm,
        l.native,
        l.shell,
        l.web,
        l.config,
        l.lock,
        l.data,
        l.doc,
        l.image,
        l.media,
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
    // 语言色板也必须全抹平：漏一档就是「`NO_COLOR` 下目录树仍然带色」。
    for (name, c) in lang_slots(&m.lang) {
        assert_eq!(c, Color::Reset, "{name} 未抹平");
    }
    // 无色模式下修饰位仍可用（调用方据此区分语义）。
    assert_eq!(m.downgrade_basic(), m);
}

/// 十五个语言 / 资产槽（带名字，断言失败时能报出是哪一档）。
fn lang_slots(l: &LangColors) -> [(&'static str, Color); 15] {
    [
        ("rust", l.rust),
        ("python", l.python),
        ("js", l.js),
        ("ts", l.ts),
        ("go", l.go),
        ("jvm", l.jvm),
        ("native", l.native),
        ("shell", l.shell),
        ("web", l.web),
        ("config", l.config),
        ("lock", l.lock),
        ("data", l.data),
        ("doc", l.doc),
        ("image", l.image),
        ("media", l.media),
    ]
}

/// 同一套色板里十五档必须两两不同：撞色等于把两种文件重新并成一类，而用户看不见
/// 「这里本该有区别」。深色 / 浅色两套各查一遍。
#[test]
fn no_two_language_slots_share_a_colour() {
    for (theme, lang) in [
        ("default-dark", DEFAULT_DARK.lang),
        ("crabmate-light", CRABMATE_LIGHT.lang),
    ] {
        let slots = lang_slots(&lang);
        for (i, (name_a, a)) in slots.iter().enumerate() {
            for (name_b, b) in slots.iter().skip(i + 1) {
                assert_ne!(a, b, "{theme}: `{name_a}` 与 `{name_b}` 撞色");
            }
        }
    }
}

/// 浅色主题的每一档都必须比深色主题的同名档更暗：浅底上照抄深色值会糊成一片。
#[test]
fn light_language_slots_are_darker_than_the_dark_ones() {
    for ((name, dark), (_, light)) in lang_slots(&LANG_DARK)
        .into_iter()
        .zip(lang_slots(&LANG_LIGHT))
    {
        assert!(
            luma(light) < luma(dark),
            "`{name}`：浅色档不比深色档暗（{light:?} vs {dark:?}）"
        );
    }
}

/// 深色底上每一档都要比背景亮、浅色底上每一档都要比背景暗 —— 否则等于看不见。
#[test]
fn every_language_slot_contrasts_with_its_background() {
    for (name, c) in lang_slots(&DEFAULT_DARK.lang) {
        assert!(luma(c) > luma(DEFAULT_DARK.bg), "`{name}` 太暗");
    }
    for (name, c) in lang_slots(&CRABMATE_LIGHT.lang) {
        assert!(luma(c) < luma(CRABMATE_LIGHT.bg), "`{name}` 太亮");
    }
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
