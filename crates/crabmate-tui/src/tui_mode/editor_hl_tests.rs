//! `editor_hl` 的单测：词法器的正确性必须脱离渲染验证（issue #510 的 P1）。
//!
//! 重点不是「常见的代码长得对」，而是**那几种认错就会连累后面一整段**的边界：
//! Rust 的 raw string 与嵌套块注释、Python/TOML 的三引号、C++ 的预处理行。
//! 认错的表现不是「这里少了个颜色」，而是「从这行起整屏都是字符串色」。

use super::*;

fn spans_of(line: &str, lang: Language) -> Vec<(usize, usize, SyntaxCat)> {
    let (spans, _) = classify_line(line, LexState::Normal, lang);
    spans.iter().map(|s| (s.start, s.end, s.cat)).collect()
}

fn spans_from(
    line: &str,
    state: LexState,
    lang: Language,
) -> (Vec<(usize, usize, SyntaxCat)>, LexState) {
    let (spans, end) = classify_line(line, state, lang);
    (spans.iter().map(|s| (s.start, s.end, s.cat)).collect(), end)
}

#[test]
fn paths_map_to_the_six_supported_languages() {
    use Language::*;
    for (path, want) in [
        ("src/main.rs", Some(Rust)),
        ("a/b/lib.rs", Some(Rust)),
        ("main.cpp", Some(Cpp)),
        ("include/x.hpp", Some(Cpp)),
        ("legacy.c", Some(Cpp)),
        ("tool.py", Some(Python)),
        ("types.pyi", Some(Python)),
        ("conf.json", Some(Json)),
        ("conf.jsonc", Some(Json)),
        ("Cargo.toml", Some(Toml)),
        ("README.md", Some(Markdown)),
        ("doc/GUIDE.MARKDOWN", Some(Markdown)),
        ("data.bin", None),
        ("Makefile", None),
        ("noext", None),
    ] {
        assert_eq!(language_for_path(path), want, "{path}");
    }
}

#[test]
fn rust_keywords_numbers_and_calls() {
    // `let` 关键字、`1` 数字、`foo(` 是函数；`x` / `=` / `;` 保持正文色。
    assert_eq!(
        spans_of("let x = foo(1);", Language::Rust),
        vec![
            (0, 3, SyntaxCat::Keyword),
            (8, 11, SyntaxCat::Function),
            (12, 13, SyntaxCat::Number),
        ]
    );
}

#[test]
fn rust_upper_camel_is_a_type_and_snake_is_not() {
    assert_eq!(
        spans_of("let v: Vec<u8> = x;", Language::Rust)
            .iter()
            .filter(|s| s.2 == SyntaxCat::Type)
            .count(),
        2,
        "Vec 与 u8 都算类型"
    );
    assert!(
        spans_of("let abc = 1;", Language::Rust)
            .iter()
            .all(|s| s.2 != SyntaxCat::Type),
        "全小写标识符不是类型"
    );
}

#[test]
fn rust_line_comment_runs_to_end_of_line() {
    let line = "let x = 1; // 收尾注释";
    let spans = spans_of(line, Language::Rust);
    let last = spans.last().expect("应有注释区间");
    assert_eq!(last.2, SyntaxCat::Comment);
    assert_eq!(last.1, line.chars().count(), "注释吃到行尾");
}

#[test]
fn rust_macro_bang_is_a_keyword_but_neq_is_not() {
    assert_eq!(
        spans_of("println!(x);", Language::Rust)[0],
        (0, 8, SyntaxCat::Keyword),
        "`println!` 连感叹号一起算关键字"
    );
    // `a != b` 的 `!` 后面是 `=`，不能当宏。
    assert!(
        spans_of("if a != b {}", Language::Rust)
            .iter()
            .all(|s| s.2 != SyntaxCat::Keyword || s.0 == 0),
        "只有 if 是关键字"
    );
}

#[test]
fn rust_nested_block_comment_closes_at_the_right_depth() {
    // 单行内嵌套：两个 `/*` 要两个 `*/` 才闭合。
    let (spans, end) = spans_from("/* a /* b */ c */ x", LexState::Normal, Language::Rust);
    assert_eq!(end, LexState::Normal, "行内闭合，不跨行");
    assert_eq!(spans[0].2, SyntaxCat::Comment);
    assert_eq!(spans[0].1, 17, "注释区间覆盖到最外层收尾");
}

#[test]
fn rust_block_comment_carries_across_lines() {
    let (spans, end) = spans_from("let a = 1; /* open", LexState::Normal, Language::Rust);
    assert_eq!(end, LexState::BlockComment(1));
    assert_eq!(spans.last().unwrap().2, SyntaxCat::Comment);
    // 续行整行仍是注释，并在闭合处结束。
    let (spans, end) = spans_from(
        "still */ let b = 2;",
        LexState::BlockComment(1),
        Language::Rust,
    );
    assert_eq!(spans[0], (0, 8, SyntaxCat::Comment));
    assert_eq!(end, LexState::Normal);
}

/// 一行里连开**两层**且都没闭合时，下一行必须还欠两个 `*/`。
///
/// 这是 review 抓到的一条真 bug 的回归守卫：把剩余深度写成常量 1 的话，第二行的第一个
/// `*/` 就会被当闭合，剩下的注释被当代码 —— 而嵌套注释最常见的用处正是「把一段含注释
/// 的代码整块注释掉」。
#[test]
fn rust_nested_block_comment_keeps_its_depth_across_lines() {
    let (_, end) = spans_from("/* outer /* inner", LexState::Normal, Language::Rust);
    assert_eq!(end, LexState::BlockComment(2), "两层都还开着");
    // 只闭内层：还剩一层，且整行都还在注释里。
    let (spans, end) = spans_from("*/ still outer", LexState::BlockComment(2), Language::Rust);
    assert_eq!(spans[0].0, 0);
    assert_eq!(spans[0].2, SyntaxCat::Comment);
    assert_eq!(end, LexState::BlockComment(1));
    // 再闭一层才回到正常。
    let (_, end) = spans_from("*/ done", LexState::BlockComment(1), Language::Rust);
    assert_eq!(end, LexState::Normal);
}

#[test]
fn rust_raw_string_takes_the_hash_run_into_account() {
    // `r#"a "b" c"#`：中间的 `"` 不是收尾。
    let (spans, end) = spans_from(
        r##"let s = r#"a "b" c"#;"##,
        LexState::Normal,
        Language::Rust,
    );
    assert_eq!(end, LexState::Normal);
    let str_span = spans
        .iter()
        .find(|s| s.2 == SyntaxCat::Str)
        .expect("字符串");
    assert_eq!(str_span.0, 8, "含 `r#` 前缀");
    assert_eq!(str_span.1, 20, "含收尾 `\"#`");
}

#[test]
fn rust_unclosed_raw_string_resumes_next_line() {
    let (_, end) = spans_from(r##"let s = r#"open"##, LexState::Normal, Language::Rust);
    assert_eq!(end, LexState::RawString(1));
    let (spans, end) = spans_from(r##"still"# done"##, LexState::RawString(1), Language::Rust);
    assert_eq!(end, LexState::Normal);
    assert_eq!(spans[0].2, SyntaxCat::Str);
}

#[test]
fn cpp_preprocessor_line_is_one_keyword_span() {
    let spans = spans_of("  #include <string>", Language::Cpp);
    assert_eq!(spans, vec![(2, 19, SyntaxCat::Keyword)], "整行都是预处理");
}

#[test]
fn cpp_builtin_types_and_std_names() {
    let spans = spans_of("std::string s = \"x\";", Language::Cpp);
    assert!(spans.contains(&(5, 11, SyntaxCat::Type)), "{spans:?}");
    assert!(spans.iter().any(|s| s.2 == SyntaxCat::Str), "{spans:?}");
}

#[test]
fn cpp_block_comment_is_not_nested() {
    // C++ 不嵌套：第一个 `*/` 就闭合。
    let (spans, end) = spans_from("/* a /* b */ int x;", LexState::Normal, Language::Cpp);
    assert_eq!(end, LexState::Normal);
    assert_eq!(spans[0].1, 12, "到第一个 */ 为止");
}

#[test]
fn python_comment_and_def() {
    let spans = spans_of("def foo(x):  # 说明", Language::Python);
    assert!(spans.contains(&(0, 3, SyntaxCat::Keyword)), "{spans:?}");
    assert!(spans.contains(&(4, 7, SyntaxCat::Function)), "{spans:?}");
    assert_eq!(spans.last().unwrap().2, SyntaxCat::Comment);
}

#[test]
fn python_triple_quoted_string_carries_across_lines() {
    let (_, end) = spans_from("s = \"\"\"start", LexState::Normal, Language::Python);
    assert_eq!(end, LexState::Triple('"'));
    let (spans, end) = spans_from("middle", LexState::Triple('"'), Language::Python);
    assert_eq!(spans[0].2, SyntaxCat::Str);
    assert_eq!(end, LexState::Triple('"'));
    let (_, end) = spans_from("end\"\"\"", LexState::Triple('"'), Language::Python);
    assert_eq!(end, LexState::Normal);
}

#[test]
fn json_strings_numbers_and_literals() {
    let spans = spans_of(r#"{"a": 1, "b": true}"#, Language::Json);
    assert_eq!(
        spans,
        vec![
            (1, 4, SyntaxCat::Str),
            (6, 7, SyntaxCat::Number),
            (9, 12, SyntaxCat::Str),
            (14, 18, SyntaxCat::Keyword),
        ]
    );
}

#[test]
fn toml_table_header_and_key_names() {
    assert_eq!(
        spans_of("[profile.release]", Language::Toml)[0].2,
        SyntaxCat::Keyword
    );
    // 键名上色，值是数字。
    let spans = spans_of("edition = 2021", Language::Toml);
    assert!(spans.contains(&(0, 7, SyntaxCat::Type)), "{spans:?}");
    assert!(spans.iter().any(|s| s.2 == SyntaxCat::Number), "{spans:?}");
    // 点号路径里只有最后一段是真键名。
    let dotted = spans_of("a.b = 1", Language::Toml);
    assert_eq!(
        dotted.iter().filter(|s| s.2 == SyntaxCat::Type).count(),
        1,
        "{dotted:?}"
    );
}

#[test]
fn toml_multiline_string_carries_across_lines() {
    let (_, end) = spans_from("desc = \"\"\"line1", LexState::Normal, Language::Toml);
    assert_eq!(end, LexState::Triple('"'));
    let (spans, end) = spans_from("line2\"\"\"", LexState::Triple('"'), Language::Toml);
    assert_eq!(spans[0].2, SyntaxCat::Str);
    assert_eq!(end, LexState::Normal);
}

#[test]
fn markdown_heading_fence_and_list_marker() {
    assert_eq!(
        spans_of("# 标题", Language::Markdown)[0].2,
        SyntaxCat::Keyword
    );
    // 围栏：开 → 内容 → 收尾，状态依次传递。
    let (_, end) = spans_from("```rust", LexState::Normal, Language::Markdown);
    assert_eq!(end, LexState::MdFence('`', 3));
    let (spans, end) = spans_from("let x = 1;", LexState::MdFence('`', 3), Language::Markdown);
    assert_eq!(spans[0].2, SyntaxCat::Comment, "围栏内整行");
    assert_eq!(end, LexState::MdFence('`', 3));
    let (_, end) = spans_from("```", LexState::MdFence('`', 3), Language::Markdown);
    assert_eq!(end, LexState::Normal, "收尾行闭合");
    // 列表标记只染标记本身。
    assert_eq!(
        spans_of("- 第一项", Language::Markdown)[0],
        (0, 2, SyntaxCat::Keyword)
    );
    // `#hashtag` 不是标题。
    assert!(spans_of("#hashtag", Language::Markdown).is_empty());
}

/// 围栏**内**贴一行 ```rust 是内容，不是收尾。
///
/// 讲 markdown 语法的文档里这一行很常见。收尾必须「marker 之后只有空白」——
/// 少了那个条件，围栏会提前结束，后面整块错位成正文态（review 抓到的第二条）。
#[test]
fn markdown_fence_inside_keeps_a_labelled_fence_line_as_content() {
    let (spans, end) = spans_from("```rust", LexState::MdFence('`', 3), Language::Markdown);
    assert_eq!(spans[0].2, SyntaxCat::Comment, "围栏内的 ```rust 是内容");
    assert_eq!(end, LexState::MdFence('`', 3));
    // 裸收尾、带尾随空格仍算收尾。
    let (_, end) = spans_from("```  ", LexState::MdFence('`', 3), Language::Markdown);
    assert_eq!(end, LexState::Normal);
    // 更长的收尾也能收 3 个的围栏。
    let (_, end) = spans_from("`````", LexState::MdFence('`', 3), Language::Markdown);
    assert_eq!(end, LexState::Normal);
}

#[test]
fn markdown_inline_code_and_link_target() {
    let spans = spans_of("用 `cargo test` 跑", Language::Markdown);
    assert!(spans.iter().any(|s| s.2 == SyntaxCat::Str), "{spans:?}");
    // 链接目标：`(` 在 `]` 之后，区间从括号起。
    let link = spans_of("见 [文档](https://x.y)", Language::Markdown);
    assert!(
        link.iter().any(|s| s.2 == SyntaxCat::Str && s.0 == 6),
        "{link:?}"
    );
}

#[test]
fn scan_states_returns_one_state_per_line_plus_one() {
    let lines: Vec<String> = vec![
        "let a = 1;".to_string(),
        "/* open".to_string(),
        "still".to_string(),
        "close */".to_string(),
    ];
    let states = scan_states(&lines, Language::Rust);
    assert_eq!(states.len(), lines.len() + 1);
    assert_eq!(states[0], LexState::Normal);
    assert_eq!(states[1], LexState::Normal);
    assert_eq!(states[2], LexState::BlockComment(1), "第二行打开块注释");
    assert_eq!(states[3], LexState::BlockComment(1), "第三行仍在注释里");
    assert_eq!(states[4], LexState::Normal, "收尾态回到正常");
}

#[test]
fn blank_and_unrecognised_lines_produce_no_spans() {
    for line in ["", "   ", "\t\t", "普通中文", "→ ←"] {
        for lang in [
            Language::Rust,
            Language::Cpp,
            Language::Python,
            Language::Json,
            Language::Toml,
            Language::Markdown,
        ] {
            assert!(spans_of(line, lang).is_empty(), "{line:?} / {lang:?}");
        }
    }
}

/// 任何输入都不许 panic、不许产生越界或倒序区间 —— 词法器跑在渲染路径上，
/// 一次 panic 就是整个视图打不开。
#[test]
fn every_span_is_ordered_and_within_the_line() {
    let samples = [
        "let s = r#\"a\"#;",
        "/* a /* b */ c */",
        "\"\"\"unclosed",
        "```",
        "[t] a.b = 1 # c",
        "# H",
        "x = 'y'",
    ];
    for sample in samples {
        let len = sample.chars().count();
        for lang in [
            Language::Rust,
            Language::Cpp,
            Language::Python,
            Language::Json,
            Language::Toml,
            Language::Markdown,
        ] {
            let spans = spans_of(sample, lang);
            for span in spans {
                assert!(span.0 < span.1, "{sample:?}/{lang:?}: 空或倒序 {span:?}");
                assert!(span.1 <= len, "{sample:?}/{lang:?}: 越界 {span:?}");
            }
        }
    }
}
