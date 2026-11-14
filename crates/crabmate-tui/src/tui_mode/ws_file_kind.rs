//! 工作区树条目分类：只看名字判定「这是什么」，供 `ws_sidebar` 取色与取字形。
//!
//! 为什么单独一个模块：
//!
//! 1. `ws_sidebar.rs` 已经自带一整套渲染测试、贴着 920 行门禁（与 `ws_truncate.rs`
//!    同一个拆分理由）；
//! 2. 分类是纯函数，能在不搭 `UiState` 的前提下把边界测干净（`CMakeLists.txt`、
//!    `.bashrc`、`README.MD`、无扩展名……）。
//!
//! 判定顺序是**目录 → 按名（锁 / 配置 / 文档）→ 按扩展名 → 兜底**，这个顺序有具体理由：
//! `CMakeLists.txt` 若先看扩展名会落进「文档」（`.txt`），而它其实是一份构建配置；
//! `Cargo.lock` 的 `.lock` 也会先被「配置」接住，所以锁文件必须在配置之前按名认出来。
//!
//! 类型 → 颜色在 [`super::palette::Palette::ws_entry_color`]（色板是 `LangColors`），
//! 类型 → 字形在 [`icon`]：两处都按 [`FileKind`] 分派，分类表只有这一份。

use std::sync::OnceLock;

use super::event_loop::env_flag_off;

/// 条目类别。
///
/// 比「代码 / 配置 / 文档 / 图片」四档细一截：目录树的价值之一就是「一眼看出这是哪种
/// 文件」，而所有源码共用一个颜色时，`main.rs` 与 `app.tsx` 只能靠逐字读扩展名分开。
/// 色相取自 Material Icon Theme（见 `palette::LangColors`）—— 本模块只分类，不碰颜色。
///
/// `Dir` 单独一类：目录名可能带点（`v1.2`），绝不能走扩展名判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileKind {
    Dir,
    Rust,
    Python,
    Js,
    Ts,
    Go,
    /// JVM 系：`java` / `kt` / `kts` / `scala` / `groovy` / Clojure。
    Jvm,
    /// C / C++ / Objective-C 与其它编译型（Swift、C#、Zig、Nim）。
    Native,
    /// shell 与其它脚本语言（Ruby、PHP、Lua、Perl、R、Dart、Elixir、Haskell、OCaml）。
    Shell,
    /// 前端资产：HTML、CSS 预处理器、单文件组件。
    Web,
    /// 配置：`toml` / `yaml` / `json` / `ini` / `Makefile` / dotfile。
    Config,
    /// 锁文件：按名识别（`.lock` / `.json` 的扩展名会先被「配置」接住）。
    Lock,
    /// 数据：`csv` / `xml` / `sql` 与序列化、schema 文件。
    Data,
    Doc,
    Image,
    /// 字体 / 音视频 / PDF：都是资产，一眼能看出不是源码。
    Media,
    /// 无扩展名、未知扩展名 —— 以及所有不属于上面几类的文件。
    Other,
}

/// 按名命中的锁文件（**优先于**配置与扩展名）。
const LOCK_NAMES: &[&str] = &[
    "cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "gemfile.lock",
    "composer.lock",
];

/// 按名命中的配置类文件（**优先于**扩展名判定）。
///
/// `cmakelists.txt` 在名单里是因为扩展名会把它判成文档；其余几项要么没有扩展名、
/// 要么按扩展名也能判对，但列在这里可以避免「靠扩展名侥幸判对」。dotfile 一并在内：
/// `.bashrc` 这种「点开头 + 无扩展名」的文件，只有按名表认得出来。
const CONFIG_NAMES: &[&str] = &[
    "makefile",
    "gnumakefile",
    "dockerfile",
    "cmakelists.txt",
    "justfile",
    "procfile",
    ".gitignore",
    ".gitattributes",
    ".dockerignore",
    ".editorconfig",
    ".env",
    ".npmrc",
    ".prettierrc",
    ".bashrc",
    ".zshrc",
    ".profile",
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
    "contributing",
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
    if LOCK_NAMES.contains(&lower.as_str()) {
        return FileKind::Lock;
    }
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
/// 把 `.bashrc` 的 `bashrc` 当扩展名，会让它命中「未知扩展名 → 其它」——虽然按名表
/// 已经先接住了它，但规则本身要说清楚：**点开头不是扩展名**。
fn extension_of(lower: &str) -> Option<&str> {
    let (stem, ext) = lower.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext)
}

/// 扩展名 → 类别（不在表里则 `None`）。
///
/// 写成 `match` 而不是几张 `&[&str]` 表：分支数已经到几十个，线性 `contains` 会在
/// 「每帧每行问一次」的热路径上退化，而 `match` 编译成跳转表。
fn kind_of_extension(ext: &str) -> Option<FileKind> {
    Some(match ext {
        "rs" => FileKind::Rust,
        "py" | "pyi" | "ipynb" => FileKind::Python,
        "js" | "jsx" | "mjs" | "cjs" => FileKind::Js,
        "ts" | "tsx" | "mts" | "cts" => FileKind::Ts,
        "go" => FileKind::Go,
        "java" | "kt" | "kts" | "scala" | "groovy" | "gradle" | "clj" | "cljs" => FileKind::Jvm,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "m" | "mm" | "swift" | "cs" | "zig"
        | "nim" => FileKind::Native,
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" | "rb" | "php" | "lua" | "pl"
        | "r" | "dart" | "ex" | "exs" | "erl" | "hs" | "ml" | "tcl" | "awk" => FileKind::Shell,
        "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" | "astro" => {
            FileKind::Web
        }
        "toml" | "yaml" | "yml" | "json" | "jsonc" | "ini" | "cfg" | "conf" | "env"
        | "properties" => FileKind::Config,
        "lock" => FileKind::Lock,
        "csv" | "tsv" | "xml" | "sql" | "jsonl" | "ndjson" | "parquet" | "avro" | "proto"
        | "graphql" | "gql" => FileKind::Data,
        "md" | "markdown" | "rst" | "adoc" | "txt" => FileKind::Doc,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tif" | "tiff"
        | "avif" => FileKind::Image,
        "mp4" | "webm" | "mkv" | "mov" | "mp3" | "wav" | "flac" | "ogg" | "opus" | "ttf"
        | "otf" | "woff" | "woff2" | "pdf" => FileKind::Media,
        _ => return None,
    })
}

/// 类别标记：**恒 2 列**（一个符号 + 一个空格）。
///
/// `Dir` 返回 `None` —— 目录已经有 `▸` / `▾` 折叠箭头，再叠一个图标等于一列放两个语义。
///
/// 标记只按**大类**分组（代码 / 配置 / 文档 / 图片 / 其它），细分只体现在颜色上：
/// 十五种语言各配一个 ASCII 符号既记不住，也会把「按行命中」的宽度预算搅乱。
///
/// 全部是单列宽的 ASCII / 拉丁符号，**刻意不用 Nerd Font**：字体装没装探测不到，缺字体
/// 就是豆腐块；更要命的是部分终端把图标渲染成 2 列，右栏对齐会直接错位，而这里的每一行
/// 都要和鼠标命中共用同一份宽度预算。
#[must_use]
pub(super) fn icon(kind: FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Dir => None,
        FileKind::Rust
        | FileKind::Python
        | FileKind::Js
        | FileKind::Ts
        | FileKind::Go
        | FileKind::Jvm
        | FileKind::Native
        | FileKind::Shell
        | FileKind::Web
        | FileKind::Data => Some("{ "),
        FileKind::Config | FileKind::Lock => Some("= "),
        FileKind::Doc => Some("\" "),
        FileKind::Image | FileKind::Media => Some("* "),
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

    /// 每种语言自成一类：这正是「颜色比原来多」的意义所在。
    #[test]
    fn each_language_gets_its_own_kind_regardless_of_case() {
        for (name, want) in [
            ("main.rs", FileKind::Rust),
            ("Main.Java", FileKind::Jvm),
            ("build.gradle", FileKind::Jvm),
            ("app.tsx", FileKind::Ts),
            ("index.mjs", FileKind::Js),
            ("model.pyi", FileKind::Python),
            ("cmd.go", FileKind::Go),
            ("lib.c", FileKind::Native),
            ("vec.hpp", FileKind::Native),
            ("deploy.sh", FileKind::Shell),
            ("Gemfile.rb", FileKind::Shell),
            ("app.vue", FileKind::Web),
            ("theme.scss", FileKind::Web),
            ("rows.csv", FileKind::Data),
            ("schema.sql", FileKind::Data),
            ("query.graphql", FileKind::Data),
        ] {
            assert_eq!(kind(name), want, "{name}");
        }
    }

    /// 老的四档仍然认得（回归：细分不该把大类弄丢）。
    #[test]
    fn the_four_coarse_buckets_survive_the_split() {
        assert_eq!(kind("Cargo.toml"), FileKind::Config);
        assert_eq!(kind("ci.yaml"), FileKind::Config);
        assert_eq!(kind("README.md"), FileKind::Doc);
        assert_eq!(kind("notes.TXT"), FileKind::Doc);
        assert_eq!(kind("logo.webp"), FileKind::Image);
        // 图标（矢量）也归图片：它是图形资产，不是文档。
        assert_eq!(kind("icon.svg"), FileKind::Image);
        // 字体与音视频同属「资产」，另开一档。
        assert_eq!(kind("Inter.woff2"), FileKind::Media);
        assert_eq!(kind("demo.mp4"), FileKind::Media);
    }

    /// 锁文件必须**先于**配置被认出来：`.lock` / `.json` 的扩展名会落进「配置」。
    #[test]
    fn lock_files_win_over_config_extensions() {
        assert_eq!(kind("Cargo.lock"), FileKind::Lock);
        assert_eq!(kind("package-lock.json"), FileKind::Lock);
        assert_eq!(kind("yarn.lock"), FileKind::Lock);
        // 普通 `.lock` 走扩展名，也是锁文件。
        assert_eq!(kind("poetry.lock"), FileKind::Lock);
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
        assert_eq!(kind("Dockerfile"), FileKind::Config);
        assert_eq!(kind("LICENSE"), FileKind::Doc);
        assert_eq!(kind("CHANGELOG"), FileKind::Doc);
    }

    #[test]
    fn dotfiles_are_not_extensions() {
        // `.gitignore` / `.bashrc` / `.env`：点开头只有一个点时，`bashrc` 不是扩展名；
        // 它们靠按名表落进「配置」。
        assert_eq!(kind(".gitignore"), FileKind::Config);
        assert_eq!(kind(".bashrc"), FileKind::Config);
        assert_eq!(kind(".env"), FileKind::Config);
        // 不认识的 dotfile 仍是「其它」，不要凭空给颜色。
        assert_eq!(kind(".weirdrc"), FileKind::Other);
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
        for kind in ALL_KINDS {
            match icon(kind) {
                Some(icon) => assert_eq!(
                    UnicodeWidthStr::width(icon),
                    2,
                    "{kind:?} 的标记宽度必须是 2"
                ),
                None => assert_eq!(kind, FileKind::Dir, "只有目录没有标记"),
            }
        }
    }

    /// 所有变体（新增变体时这里会提醒你补上标记与颜色的对照）。
    const ALL_KINDS: [FileKind; 17] = [
        FileKind::Dir,
        FileKind::Rust,
        FileKind::Python,
        FileKind::Js,
        FileKind::Ts,
        FileKind::Go,
        FileKind::Jvm,
        FileKind::Native,
        FileKind::Shell,
        FileKind::Web,
        FileKind::Config,
        FileKind::Lock,
        FileKind::Data,
        FileKind::Doc,
        FileKind::Image,
        FileKind::Media,
        FileKind::Other,
    ];
}
