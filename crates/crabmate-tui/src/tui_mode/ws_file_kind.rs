//! 工作区树条目分类：只看名字判定「这是什么」，供 `ws_sidebar` 取色与取字形。
//!
//! 为什么单独一个模块：
//!
//! 1. `ws_sidebar.rs` 已经自带一整套渲染测试、贴着 920 行门禁（与 `ws_truncate.rs`
//!    同一个拆分理由）；
//! 2. 分类是纯函数，能在不搭 `UiState` 的前提下把边界测干净（`CMakeLists.txt`、
//!    `.bashrc`、`README.MD`、无扩展名……）。
//!
//! 判定顺序是**按名 → 按扩展名 → 兜底**，这个顺序有具体理由：`CMakeLists.txt` 若先看
//! 扩展名会落进「文档」（`.txt`），而它其实是一份构建配置；`Cargo.toml` 两种走法都对，
//! 但按名表里也列着，免得将来扩展名表调整时漏掉。
//!
//! 类型 → 颜色在 [`super::palette::Palette::ws_entry_color`]，类型 → 字形在 [`icon`]：
//! 三处各写一份分类表，迟早会出现「这个文件是代码色 + 文档图标」。

use std::sync::OnceLock;

use super::event_loop::env_flag_off;

/// 条目类别。
///
/// `Dir` 单独一类：目录名可能带点（`v1.2`），绝不能走扩展名判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileKind {
    Dir,
    Code,
    Config,
    Doc,
    Image,
    /// 无扩展名、未知扩展名 —— 以及所有不属于上面几类的文件。
    Other,
}

/// 源码类扩展名（大小写无关，见 [`file_kind`] 统一转小写）。
const CODE_EXTS: &[&str] = &[
    "rs", "py", "pyi", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "c", "h", "cc", "cpp", "cxx",
    "hpp", "hxx", "java", "kt", "kts", "rb", "sh", "bash", "zsh", "fish", "lua", "php", "swift",
    "cs", "scala", "hs", "ml", "ex", "exs", "erl", "clj", "zig", "nim", "sql", "vue", "svelte",
];

/// 配置类扩展名。
const CONFIG_EXTS: &[&str] = &[
    "toml",
    "yaml",
    "yml",
    "json",
    "jsonc",
    "ini",
    "cfg",
    "conf",
    "env",
    "lock",
    "properties",
];

/// 文档类扩展名。
const DOC_EXTS: &[&str] = &["md", "markdown", "rst", "adoc", "txt"];

/// 图片类扩展名。
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"];

/// 按名命中的配置类文件（**优先于**扩展名判定）。
///
/// `cmakelists.txt` 在名单里是因为扩展名会把它判成文档；其余几项要么没有扩展名、
/// 要么按扩展名也能判对，但列在这里可以避免「靠扩展名侥幸判对」。
const CONFIG_NAMES: &[&str] = &[
    "makefile",
    "gnumakefile",
    "dockerfile",
    "cmakelists.txt",
    ".gitignore",
    ".gitattributes",
    ".dockerignore",
    ".editorconfig",
    ".env",
    ".npmrc",
    ".prettierrc",
    "cargo.lock",
];

/// 按名命中的文档类文件（**优先于**扩展名判定）。
///
/// `license` / `changelog` 这类约定俗成的无扩展名文件：不列出来的话，它们在树里
/// 会与 `data.bin` 一样是「其它」，而它们恰恰是常在仓库根一眼要找的东西。
const DOC_NAMES: &[&str] = &[
    "license",
    "licence",
    "copying",
    "notice",
    "changelog",
    "authors",
    "readme",
];

/// 名字 → 类别。
///
/// 目录直接返回 [`FileKind::Dir`]，不看名字（`v1.2` 这种目录会被 `extension_of`
/// 当成 `.2` 扩展名）。
#[must_use]
pub(super) fn file_kind(name: &str, is_dir: bool) -> FileKind {
    if is_dir {
        return FileKind::Dir;
    }
    let lower = name.trim().to_ascii_lowercase();
    if CONFIG_NAMES.contains(&lower.as_str()) {
        return FileKind::Config;
    }
    if DOC_NAMES.contains(&lower.as_str()) {
        return FileKind::Doc;
    }
    extension_of(&lower)
        .and_then(kind_of_extension)
        .unwrap_or(FileKind::Other)
}

/// 小写扩展名；点开头且只有一个点的名字（`.bashrc`）按「无扩展名」处理。
///
/// 把 `.bashrc` 的 `bashrc` 当扩展名，会让它命中「未知扩展名 → 其它」——虽然最终
/// 结果与该文件应是「配置」只差一档颜色，但规则本身要说清楚：**点开头不是扩展名**。
fn extension_of(lower: &str) -> Option<&str> {
    let (stem, ext) = lower.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext)
}

/// 扩展名 → 类别（不在四张表里则 `None`）。
fn kind_of_extension(ext: &str) -> Option<FileKind> {
    if CODE_EXTS.contains(&ext) {
        return Some(FileKind::Code);
    }
    if CONFIG_EXTS.contains(&ext) {
        return Some(FileKind::Config);
    }
    if DOC_EXTS.contains(&ext) {
        return Some(FileKind::Doc);
    }
    if IMAGE_EXTS.contains(&ext) {
        return Some(FileKind::Image);
    }
    None
}

/// 类别标记：**恒 2 列**（一个符号 + 一个空格）。
///
/// `Dir` 返回 `None` —— 目录已经有 `▸` / `▾` 折叠箭头，再叠一个图标等于一列放两个语义。
///
/// 全部是单列宽的 ASCII / 拉丁符号，**刻意不用 Nerd Font**：字体装没装探测不到，缺字体
/// 就是豆腐块；更要命的是部分终端把图标渲染成 2 列，右栏对齐会直接错位，而这里的每一行
/// 都要和鼠标命中共用同一份宽度预算。
#[must_use]
pub(super) fn icon(kind: FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Dir => None,
        FileKind::Code => Some("{ "),
        FileKind::Config => Some("= "),
        FileKind::Doc => Some("\" "),
        FileKind::Image => Some("* "),
        FileKind::Other => Some("· "),
    }
}

/// 类别标记开关：默认开；`CM_TUI_FILE_ICONS=off|0|false|none` 关掉（只留颜色）。
///
/// 读一次就缓存：这是**每帧每行**都会问的问题，而环境变量在进程生命周期内不会变。
/// 测试不经过这里——`ws_row_text` 收显式的 `icons: bool`，所以开关状态不会渗进渲染单测。
#[must_use]
pub(super) fn icons_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| !env_flag_off("CM_TUI_FILE_ICONS"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn kind(name: &str) -> FileKind {
        file_kind(name, false)
    }

    #[test]
    fn a_directory_is_a_directory_whatever_its_name_looks_like() {
        // `v1.2` 有「扩展名」`.2`，但它仍是目录。
        assert_eq!(file_kind("v1.2", true), FileKind::Dir);
        assert_eq!(file_kind("src", true), FileKind::Dir);
    }

    #[test]
    fn source_files_are_code_regardless_of_case() {
        for name in ["main.rs", "app.tsx", "Main.Java", "script.sh", "query.sql"] {
            assert_eq!(kind(name), FileKind::Code, "{name}");
        }
    }

    #[test]
    fn config_and_docs_and_images_are_told_apart() {
        assert_eq!(kind("Cargo.toml"), FileKind::Config);
        assert_eq!(kind("ci.yaml"), FileKind::Config);
        assert_eq!(kind("README.md"), FileKind::Doc);
        assert_eq!(kind("notes.TXT"), FileKind::Doc);
        assert_eq!(kind("logo.webp"), FileKind::Image);
        // 图标（矢量）也归图片：它是图形资产，不是文档。
        assert_eq!(kind("icon.svg"), FileKind::Image);
        // `lock` 归配置：与 `Cargo.lock` / `package-lock.json` 同类。
        assert_eq!(kind("yarn.lock"), FileKind::Config);
    }

    /// 按名命中的那几张表**优先于**扩展名：否则 `.txt` / 无扩展名会被判错。
    #[test]
    fn well_known_names_win_over_the_extension() {
        assert_eq!(
            kind("CMakeLists.txt"),
            FileKind::Config,
            "`.txt` 会误判成文档"
        );
        assert_eq!(kind("Makefile"), FileKind::Config);
        assert_eq!(kind("LICENSE"), FileKind::Doc);
        assert_eq!(kind("CHANGELOG"), FileKind::Doc);
        assert_eq!(kind("Cargo.lock"), FileKind::Config);
    }

    #[test]
    fn dotfiles_are_not_extensions() {
        // `.bashrc` / `.gitignore`：点开头只有一个点时，`bashrc` 不是扩展名。
        assert_eq!(kind(".gitignore"), FileKind::Config, "按名表命中");
        assert_eq!(kind(".bashrc"), FileKind::Other, "无扩展名 + 不在名表");
        assert_eq!(kind(".env"), FileKind::Config, "按名表命中");
    }

    #[test]
    fn unknown_and_missing_extensions_fall_into_other() {
        assert_eq!(kind("data.bin"), FileKind::Other);
        assert_eq!(kind("notes"), FileKind::Other);
        assert_eq!(kind("trailing."), FileKind::Other, "空扩展名不算扩展名");
    }

    /// 标记必须恒 2 列：宽度预算（`ws_row_text`）与鼠标命中共用同一份算术，
    /// 一旦某个终端把某符号渲染成 2 列宽，这一列就会整体错位。
    #[test]
    fn every_icon_is_exactly_two_columns() {
        for kind in [
            FileKind::Code,
            FileKind::Config,
            FileKind::Doc,
            FileKind::Image,
            FileKind::Other,
        ] {
            let icon = icon(kind).expect("非目录都有标记");
            assert_eq!(
                UnicodeWidthStr::width(icon),
                2,
                "{kind:?} 的标记宽度必须是 2"
            );
        }
        assert_eq!(icon(FileKind::Dir), None, "目录用折叠箭头，不叠图标");
    }
}
