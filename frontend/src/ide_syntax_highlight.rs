//! IDE 按路径选择语法语言（供 CodeMirror 语言包映射）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdeSyntaxLang {
    Rust,
    Toml,
    Yaml,
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Markdown,
    Shell,
    Go,
}

/// 后缀 → 语言（顺序有意义：如 `.h` 归 Cpp，须先于 C）。
/// Shell / `go.mod` / `.go` 在表外处理，以保持原匹配顺序。
const SUFFIX_LANG_TABLE: &[(&[&str], IdeSyntaxLang)] = &[
    (&[".rs", ".rs.in"], IdeSyntaxLang::Rust),
    (&[".toml", ".lock"], IdeSyntaxLang::Toml),
    (
        &[".yaml", ".yml", ".yaml.tpl", ".yml.tpl"],
        IdeSyntaxLang::Yaml,
    ),
    (
        &[
            ".cpp", ".cc", ".cxx", ".c++", ".hpp", ".hh", ".hxx", ".h++", ".h",
        ],
        IdeSyntaxLang::Cpp,
    ),
    (&[".c", ".i", ".mi"], IdeSyntaxLang::C),
    (&[".py", ".pyi", ".pyw", ".py.in"], IdeSyntaxLang::Python),
    (
        &[".ts", ".tsx", ".mts", ".cts", ".ts.in", ".tsx.in"],
        IdeSyntaxLang::TypeScript,
    ),
    (
        &[".js", ".jsx", ".mjs", ".cjs", ".js.in", ".jsx.in"],
        IdeSyntaxLang::JavaScript,
    ),
    (
        &[".json", ".jsonc", ".json5", ".jsonl"],
        IdeSyntaxLang::Json,
    ),
    (
        &[".md", ".markdown", ".mdx", ".mdown", ".mkd"],
        IdeSyntaxLang::Markdown,
    ),
];

#[must_use]
pub fn ide_syntax_lang_for_path(path: Option<&str>) -> Option<IdeSyntaxLang> {
    let lower = path?.to_ascii_lowercase();
    ide_syntax_lang_for_lower_path(&lower)
}

fn path_ends_with_any(lower: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| lower.ends_with(suffix))
}

fn lang_from_suffixes(
    lower: &str,
    suffixes: &[&str],
    lang: IdeSyntaxLang,
) -> Option<IdeSyntaxLang> {
    path_ends_with_any(lower, suffixes).then_some(lang)
}

fn shell_lang_for_path(lower: &str) -> Option<IdeSyntaxLang> {
    if path_ends_with_any(
        lower,
        &[".sh", ".bash", ".zsh", ".ksh", ".env", ".env.example"],
    ) || lower == "dockerfile"
        || lower.ends_with("/dockerfile")
    {
        Some(IdeSyntaxLang::Shell)
    } else {
        None
    }
}

fn ide_syntax_lang_for_lower_path(lower: &str) -> Option<IdeSyntaxLang> {
    for &(suffixes, lang) in SUFFIX_LANG_TABLE {
        if let Some(hit) = lang_from_suffixes(lower, suffixes, lang) {
            return Some(hit);
        }
    }
    shell_lang_for_path(lower)
        .or_else(|| lang_from_suffixes(lower, &[".go"], IdeSyntaxLang::Go))
        .or_else(|| lower.ends_with("go.mod").then_some(IdeSyntaxLang::Go))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages_by_extension() {
        assert_eq!(
            ide_syntax_lang_for_path(Some("config/default_config.toml")),
            Some(IdeSyntaxLang::Toml)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("docker-compose.yml")),
            Some(IdeSyntaxLang::Yaml)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("src/lib.rs")),
            Some(IdeSyntaxLang::Rust)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("src/main.c")),
            Some(IdeSyntaxLang::C)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("include/foo.h")),
            Some(IdeSyntaxLang::Cpp)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("scripts/run.py")),
            Some(IdeSyntaxLang::Python)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("app/index.ts")),
            Some(IdeSyntaxLang::TypeScript)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("app/index.js")),
            Some(IdeSyntaxLang::JavaScript)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("package.json")),
            Some(IdeSyntaxLang::Json)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("README.md")),
            Some(IdeSyntaxLang::Markdown)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("scripts/run.sh")),
            Some(IdeSyntaxLang::Shell)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("cmd/main.go")),
            Some(IdeSyntaxLang::Go)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("Dockerfile")),
            Some(IdeSyntaxLang::Shell)
        );
        assert_eq!(
            ide_syntax_lang_for_path(Some("go.mod")),
            Some(IdeSyntaxLang::Go)
        );
        assert_eq!(ide_syntax_lang_for_path(Some("notes.txt")), None);
    }
}
