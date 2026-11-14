//! `ws_sidebar.rs` 的文件类型着色与标记单测（`#[path]` 子模块）。
//!
//! 拆出去是为了守住 `fn-nloc` 的 920 行上限：这组用例要覆盖「六种类型 + 图标开关 + 窄栏
//! 预算 + 无色模式」，与同文件那些版面 / 窗口用例挤在一条 900 行的边上，谁都再难加东西。
//! 与 `workspace_tree_tests.rs` 是同一套做法。

use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

use super::*;

fn row(indent: u8, rel: &str, name: &str, is_dir: bool, expanded: bool) -> WsRow {
    WsRow {
        indent,
        rel: rel.to_string(),
        name: name.to_string(),
        is_dir,
        expanded,
        loading: false,
        placeholder: false,
    }
}

/// 一棵最小的树；配色显式落深色 —— `UiState::new()` 会读 `NO_COLOR` 等环境，
/// 测试不能依赖运行环境是不是无色终端。
fn state() -> UiState {
    let mut st = UiState::new();
    st.set_palette(Palette::from_theme(Some("shadcn-dark")));
    st.workspace_path = Some("/data/proj".to_string());
    st.ws_ready = true;
    st.ws_rows = vec![row(0, "src", "src", true, false)];
    st
}

/// 每种文件类型一档颜色：细分之后目录树要能「一眼看出这是哪种文件」。
#[test]
fn workspace_rows_colour_entries_by_file_kind() {
    let mut st = state();
    st.ws_rows = vec![
        row(0, "src", "src", true, false),
        row(1, "src/main.rs", "main.rs", false, false),
        row(1, "src/model.py", "model.py", false, false),
        row(1, "src/index.mjs", "index.mjs", false, false),
        row(1, "src/app.tsx", "app.tsx", false, false),
        row(1, "src/cmd.go", "cmd.go", false, false),
        row(1, "src/vm.java", "vm.java", false, false),
        row(1, "src/lib.c", "lib.c", false, false),
        row(1, "src/deploy.sh", "deploy.sh", false, false),
        row(1, "src/App.vue", "App.vue", false, false),
        row(1, "src/Cargo.toml", "Cargo.toml", false, false),
        row(1, "src/Cargo.lock", "Cargo.lock", false, false),
        row(1, "src/rows.csv", "rows.csv", false, false),
        row(1, "src/README.md", "README.md", false, false),
        row(1, "src/logo.png", "logo.png", false, false),
        row(1, "src/Inter.woff2", "Inter.woff2", false, false),
        row(1, "src/data.bin", "data.bin", false, false),
    ];
    // 非焦点态：选中只换底色，第 0 个 span 就是整行（因此也承载前景色）。
    let rows = workspace_sidebar_rows(&st, 60);
    let fg = |i: usize| rows[i].spans[0].style.fg;
    let lang = &st.palette.lang;
    assert_eq!(fg(1), Some(st.palette.accent), "目录沿用 accent");
    assert_eq!(fg(2), Some(lang.rust), "Rust");
    assert_eq!(fg(3), Some(lang.python), "Python");
    assert_eq!(fg(4), Some(lang.js), "JavaScript");
    assert_eq!(fg(5), Some(lang.ts), "TypeScript");
    assert_eq!(fg(6), Some(lang.go), "Go");
    assert_eq!(fg(7), Some(lang.jvm), "JVM");
    assert_eq!(fg(8), Some(lang.native), "C / C++");
    assert_eq!(fg(9), Some(lang.shell), "脚本");
    assert_eq!(fg(10), Some(lang.web), "前端资产");
    assert_eq!(fg(11), Some(lang.config), "配置");
    assert_eq!(fg(12), Some(lang.lock), "锁文件");
    assert_eq!(fg(13), Some(lang.data), "数据");
    assert_eq!(fg(14), Some(lang.doc), "文档");
    assert_eq!(fg(15), Some(lang.image), "图片");
    assert_eq!(fg(16), Some(lang.media), "字体 / 音视频");
    assert_eq!(fg(17), Some(st.palette.fg), "其它沿用正文色");
}

/// 图标默认开：行首两列是类别标记；关掉后同样的两列是空白。
#[test]
fn file_kind_icons_are_drawn_only_when_enabled() {
    let entry = row(0, "src/main.rs", "main.rs", false, false);
    let (with_icon, _) = ws_row_text(&entry, 40, true);
    assert!(with_icon.starts_with("{ "), "{with_icon}");
    let (without, _) = ws_row_text(&entry, 40, false);
    assert!(without.starts_with("  main.rs"), "{without}");
    // 名字起点相同：两列预算是恒定的，开关不该改变对齐。
    assert_eq!(
        UnicodeWidthStr::width(with_icon.as_str()),
        UnicodeWidthStr::width(without.as_str())
    );
}

/// 目录始终用折叠箭头，不叠加类别标记（一列不承载两个语义）。
#[test]
fn directories_keep_the_fold_arrow_instead_of_an_icon() {
    let collapsed = row(0, "src", "src", true, false);
    assert!(ws_row_text(&collapsed, 40, true).0.starts_with("▸ "));
    let open = row(0, "src", "src", true, true);
    assert!(ws_row_text(&open, 40, true).0.starts_with("▾ "));
    // 图标开关对目录无影响。
    assert!(ws_row_text(&open, 40, false).0.starts_with("▾ "));
}

/// 图标吃掉两列后名字仍不得溢出；极窄时只画字形，不留「只剩空白」的空洞。
#[test]
fn the_icon_never_pushes_the_name_past_the_budget() {
    let entry = row(
        0,
        "a_very_long_module_name.rs",
        "a_very_long_module_name.rs",
        false,
        false,
    );
    let (text, clipped) = ws_row_text(&entry, 12, true);
    assert!(clipped, "12 列放不下这么长的名字");
    assert!(UnicodeWidthStr::width(text.as_str()) <= 12, "{text}");
    // 宽度只够字形：名字整段让位，但字形本身还在。
    let (narrow, clipped) = ws_row_text(&entry, 2, true);
    assert!(clipped);
    assert_eq!(UnicodeWidthStr::width(narrow.as_str()), 2, "{narrow}");
}

/// 无色模式：颜色全 `Reset`，类别标记仍在 —— 它不依赖配色，也不依赖字体。
#[test]
fn mono_palette_still_marks_the_kind() {
    let mut st = state();
    st.set_palette(Palette::mono());
    st.ws_rows = vec![row(0, "src/main.rs", "main.rs", false, false)];
    let rows = workspace_sidebar_rows(&st, 60);
    assert_eq!(rows[1].spans[0].style.fg, Some(Color::Reset));
    assert!(
        rows[1].to_string().contains("{ main.rs"),
        "{}",
        rows[1].to_string()
    );
}
