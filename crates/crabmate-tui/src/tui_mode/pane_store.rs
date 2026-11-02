//! 两栏列宽的**本机**落盘（issue #407 的 P2）。
//!
//! 存什么：只有 `left` / `right` 两个字符列数。**不**复用 Desktop 的
//! `user-data/prefs.side_width` —— 那边是像素、这边是字符列，同一个键会让两端互相覆盖
//! （见 issue #407 的「不需要 Server 改动」）。也**不**往 `user-data` 里加新键：那要先在
//! Server 仓定义契约，而这件事没有任何跨端语义。
//!
//! 为什么落在 `XDG_STATE_HOME` 而不是 `XDG_CONFIG_HOME`：宽度是「上次看成的样子」，
//! 属于可丢弃的运行状态，不是用户手写维护的配置 —— 文件丢了 / 手改坏了只该回到默认版面，
//! 那里不该出现一份要人维护的「配置」。
//!
//! 读侧**不报错**：任何失败（没有 HOME、没有文件、JSON 坏了、字段缺失）都回落到默认宽度。
//! 写侧把原因交回调用方，由它决定怎么告诉用户 —— 这里是纯 IO，不碰 `UiState`。

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pane_width::{
    MAX_SIDEBAR_COLS, MAX_WORKSPACE_COLS, MIN_SIDEBAR_COLS, MIN_WORKSPACE_COLS, PaneWidths,
};

/// 覆盖落盘路径的环境变量（排查、或多套配置各存一份时用）；空串按未设置处理。
pub(crate) const PATH_ENV: &str = "CM_TUI_PANES_FILE";

/// `XDG_STATE_HOME` / `~/.local/state` 下的相对文件名。
const FILE_NAME: &str = "tui-panes.json";

/// 落盘格式。
///
/// 两栏都用 `Option` 而不是裸整数：手改文件只写一半时，缺的那一栏回落**默认**宽度，
/// 而不是被 serde 填 0 —— 0 会被夹到下限，看起来像「宽度自己变窄了」。
///
/// 值读成 `i64` 而不是 `u16`：手改成 `-1` 或 `100000` 时，`u16` 反序列化会失败，
/// 于是**整份**文件被当成读不懂、两栏一起回默认 —— 一处手写的越界值不该把另一栏也扔了。
/// 读成 `i64` 后越界只是被夹进各自范围（见 [`Stored::into_widths`]）。
#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    left: Option<i64>,
    right: Option<i64>,
}

/// 把一个手写值夹进 `[lo, hi]`（越界夹取，不报错）。
fn clamp_col(value: i64, lo: u16, hi: u16) -> u16 {
    value.clamp(i64::from(lo), i64::from(hi)) as u16
}

impl Stored {
    /// 缺字段回落各自的默认值，其余按**各自范围**夹取。
    ///
    /// 这里不能用 [`PaneWidths::clamped`]：它还要按终端宽度让位，而此刻终端宽度还没
    /// 测出来（读盘发生在建窗前）。让位留到绘制时再算，读盘只负责「给出合法值」。
    fn into_widths(self) -> PaneWidths {
        let default = PaneWidths::DEFAULT;
        PaneWidths {
            left: self.left.map_or(default.left, |v| {
                clamp_col(v, MIN_SIDEBAR_COLS, MAX_SIDEBAR_COLS)
            }),
            right: self.right.map_or(default.right, |v| {
                clamp_col(v, MIN_WORKSPACE_COLS, MAX_WORKSPACE_COLS)
            }),
        }
    }
}

/// 解析落盘内容；`None` = 读不懂（坏 JSON / 不是对象），调用方回落默认宽度。
///
/// 未知字段一律忽略（serde 默认），这样将来加字段时旧版本不会因为「多了个键」而丢宽度。
fn parse(text: &str) -> Option<PaneWidths> {
    serde_json::from_str::<Stored>(text)
        .ok()
        .map(Stored::into_widths)
}

/// 落盘路径：`CM_TUI_PANES_FILE` ＞ `$XDG_STATE_HOME` ＞ `$HOME/.local/state`。
///
/// 三个值都当参数传进来（而不是在函数里读环境变量），单测才能直接喂值 ——
/// 测试里改进程环境与并行用例共享，属于数据竞争。
///
/// 三者都拿不到时返回 `None`（不落盘）：容器里既无 HOME 也无 XDG 时，猜一个路径写进去
/// 比「本次运行内宽度照常生效、只是不保存」更糟。
pub(crate) fn resolve_path(
    override_: Option<&OsStr>,
    state_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(path) = override_.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let base = state_home
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        })?;
    Some(base.join("crabmate").join(FILE_NAME))
}

/// 本进程的落盘路径（按 [`resolve_path`] 的优先级读环境变量）。
pub(crate) fn store_path() -> Option<PathBuf> {
    let override_ = std::env::var_os(PATH_ENV);
    let state_home = std::env::var_os("XDG_STATE_HOME");
    let home = std::env::var_os("HOME");
    resolve_path(override_.as_deref(), state_home.as_deref(), home.as_deref())
}

/// 读指定文件；文件不存在 / 读不了 / 内容读不懂都返回 `None`。
pub(crate) fn load_from(path: &Path) -> Option<PaneWidths> {
    parse(&fs::read_to_string(path).ok()?)
}

/// 落盘目的地。
///
/// 路径在**装配 lane 时解析一次**，之后随 lane 持有：拖拽结束是高频动作，不该每次都去
/// 读一遍环境变量；而且「有没有地方可存」是启动时的一次性事实，中途不会变。
///
/// 这同时解决了测试的隔离问题 —— 单测装配出来的 lane 显式指向临时文件，不会把宽度写进
/// 跑测试那个人的 `~/.local/state`。
#[derive(Debug, Clone)]
pub(crate) enum PaneStore {
    /// 落盘到这个文件（来自 [`store_path`]）。
    At(PathBuf),
    /// 环境里没有任何可用目录：**想存但存不了**。
    ///
    /// 与「不落盘」分开是因为下一步完全不同：这里值得告诉用户一次
    /// （否则「拖完、重启、回到默认」查不出原因），而没有文件可读时沉默才是对的。
    Unavailable,
}

impl PaneStore {
    /// 按环境变量解析（启动时调用一次）。
    pub(crate) fn from_env() -> Self {
        match store_path() {
            Some(path) => Self::At(path),
            None => Self::Unavailable,
        }
    }

    /// 指定路径（单测用）。
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self::At(path)
    }

    /// 读回上次的宽度；读不到就是默认版面（首次运行、文件被删、内容坏了都走这条）。
    pub(crate) fn load(&self) -> PaneWidths {
        match self {
            Self::At(path) => load_from(path).unwrap_or_default(),
            Self::Unavailable => PaneWidths::DEFAULT,
        }
    }

    /// 落盘；`Err` 是**给用户看**的原因（已带路径）。
    pub(crate) fn save(&self, widths: PaneWidths) -> Result<(), String> {
        match self {
            Self::At(path) => save_to(path, widths).map_err(|e| format!("{}：{e}", path.display())),
            Self::Unavailable => Err(format!(
                "没有可用的本机状态目录（{PATH_ENV} / XDG_STATE_HOME / HOME 都为空）"
            )),
        }
    }
}

/// 原子写入指定文件（临时文件 + `rename`，同目录所以是同设备）。
///
/// 为什么不用 `fs::write`：写一半被打断留下的半截 JSON 比「没有文件」更糟 ——
/// 下次启动读到坏内容，静默回落默认，而用户以为宽度存下来了。
pub(crate) fn save_to(path: &Path, widths: PaneWidths) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_vec(&Stored {
        left: Some(i64::from(widths.left)),
        right: Some(i64::from(widths.right)),
    })
    .map_err(std::io::Error::other)?;
    // 临时名带 pid：两个 `crabmate-tui` 同时拖拽时共用一个名字会互相覆盖 ——
    // `rename` 不会留下半截文件，但会静默丢掉其中一次更新，失败分支的清理还可能删掉
    // 对方正在写的那个。
    let mut tmp = OsString::from(path.as_os_str());
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&body)?;
        // 先落内容再改名字：`rename` 只保证「换名字」这一件事的原子性。
        file.sync_all()?;
    }
    let renamed = fs::rename(&tmp, path);
    if renamed.is_err() {
        // 改名失败（目录被删、权限变了）时别把临时文件留在那里。
        let _ = fs::remove_file(&tmp);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::super::pane_width::{
        MAX_SIDEBAR_COLS, MAX_WORKSPACE_COLS, MIN_SIDEBAR_COLS, MIN_WORKSPACE_COLS, PaneWidths,
    };
    use super::{FILE_NAME, PaneStore, load_from, parse, resolve_path, save_to};

    /// 每个用例一个独立目录（并行跑测试不会互删）。
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crabmate-tui-panes-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmp dir");
        dir
    }

    fn os(s: &str) -> Option<&OsStr> {
        Some(OsStr::new(s))
    }

    /// 显式路径最优先：用户在 `CM_TUI_PANES_FILE` 里点了名，就不该再拼 XDG / HOME。
    #[test]
    fn explicit_path_wins_over_the_xdg_dirs() {
        assert_eq!(
            resolve_path(os("/tmp/x.json"), os("/state"), os("/home/u")),
            Some(PathBuf::from("/tmp/x.json"))
        );
    }

    #[test]
    fn xdg_state_home_wins_over_home() {
        assert_eq!(
            resolve_path(None, os("/state"), os("/home/u")),
            Some(PathBuf::from("/state/crabmate").join(FILE_NAME))
        );
    }

    /// 没有 XDG 时的回落：`~/.local/state`（不是 `~/.config`，理由见模块注释）。
    #[test]
    fn home_falls_back_to_the_local_state_dir() {
        assert_eq!(
            resolve_path(None, None, os("/home/u")),
            Some(
                PathBuf::from("/home/u")
                    .join(".local")
                    .join("state")
                    .join("crabmate")
                    .join(FILE_NAME)
            )
        );
    }

    /// 空串按「未设置」处理：`XDG_STATE_HOME=` 在 sudo / systemd 环境里很常见，
    /// 当路径用会拼出一个相对路径 `crabmate/tui-panes.json`（写进当前目录）。
    #[test]
    fn blank_values_count_as_unset() {
        assert_eq!(resolve_path(os(""), os(""), os("")), None);
        assert_eq!(
            resolve_path(os(""), os(""), os("/home/u")),
            Some(
                PathBuf::from("/home/u")
                    .join(".local")
                    .join("state")
                    .join("crabmate")
                    .join(FILE_NAME)
            )
        );
    }

    /// 一个路径都拼不出来时不落盘 —— 猜一个目录写进去比「本次生效但不保存」更糟。
    #[test]
    fn no_directory_means_no_file() {
        assert_eq!(resolve_path(None, None, None), None);
    }

    #[test]
    fn parses_both_columns() {
        assert_eq!(
            parse(r#"{"left":40,"right":50}"#),
            Some(PaneWidths {
                left: 40,
                right: 50
            })
        );
    }

    /// 手改坏 / 空文件 → 读不懂，调用方回落默认。
    #[test]
    fn garbage_is_not_a_width() {
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("null"), None);
        assert_eq!(parse(r#"{"left":"wide"}"#), None, "类型不对也算读不懂");
    }

    /// 数组形状会被 serde 当成 `[left, right]` 接受（derive 的 `visit_seq`）。
    ///
    /// 不是我们写出去的格式，但既然读得懂就不拦：手改的人按顺序填了两个数，
    /// 报「文件坏了」只会让人以为自己填错了。
    #[test]
    fn an_array_reads_as_left_then_right() {
        assert_eq!(
            parse("[40,50]"),
            Some(PaneWidths {
                left: 40,
                right: 50
            })
        );
    }

    /// 缺的那一栏回落**默认**（不是 0）：0 被夹到下限，看起来像宽度自己变窄了。
    #[test]
    fn a_missing_column_falls_back_to_its_default() {
        assert_eq!(
            parse(r#"{"left":40}"#),
            Some(PaneWidths {
                left: 40,
                right: PaneWidths::DEFAULT.right
            })
        );
        assert_eq!(parse("{}"), Some(PaneWidths::DEFAULT));
    }

    /// 手改出来的越界值按**各自范围**夹取：读盘不按终端宽度让位（那时还不知道多宽）。
    #[test]
    fn out_of_range_values_are_clamped_to_each_range() {
        let got = parse(r#"{"left":1,"right":60000}"#).expect("合法 JSON");
        assert_eq!(got.left, MIN_SIDEBAR_COLS);
        assert_eq!(got.right, MAX_WORKSPACE_COLS);

        let got = parse(r#"{"left":60000,"right":1}"#).expect("合法 JSON");
        assert_eq!(got.left, MAX_SIDEBAR_COLS);
        assert_eq!(got.right, MIN_WORKSPACE_COLS);
    }

    /// 未知字段忽略：将来加字段时，旧版本不该因此把宽度丢了。
    #[test]
    fn unknown_keys_do_not_lose_the_widths() {
        assert_eq!(
            parse(r#"{"left":40,"right":50,"future":true}"#),
            Some(PaneWidths {
                left: 40,
                right: 50
            })
        );
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = tmp_dir("missing");
        assert_eq!(load_from(&dir.join("nope.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 写完再读回来是同一份值 —— 这是「重启后宽度还在」的最小闭环。
    #[test]
    fn save_then_load_round_trips() {
        let dir = tmp_dir("roundtrip");
        let path = dir.join("tui-panes.json");
        let widths = PaneWidths {
            left: 44,
            right: 52,
        };
        save_to(&path, widths).expect("save");
        assert_eq!(load_from(&path), Some(widths));
        // 临时文件不该留在旁边（否则下次读到的可能是它，而它可能不是最新的）。
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 手改成负数 / 超大值：夹进各自范围，**不是**整份文件读不懂 ——
    /// 一处越界不该把另一栏也一起扔回默认。
    #[test]
    fn negative_and_huge_values_are_clamped_not_rejected() {
        let got = parse(r#"{"left":-1,"right":100000}"#).expect("夹取后仍是合法宽度");
        assert_eq!(got.left, MIN_SIDEBAR_COLS);
        assert_eq!(got.right, MAX_WORKSPACE_COLS, "另一栏按自己的上限夹取");
    }

    /// 目录还没有（首次运行）时自己建，并且不要求父目录预先存在。
    #[test]
    fn saving_creates_the_state_directory() {
        let dir = tmp_dir("mkdir");
        let path = dir.join("crabmate").join("tui-panes.json");
        assert!(!path.parent().expect("parent").exists());
        save_to(&path, PaneWidths::DEFAULT).expect("save");
        assert_eq!(load_from(&path), Some(PaneWidths::DEFAULT));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 覆盖写：第二次写进去的值就是读回来的值（不是两份拼起来）。
    #[test]
    fn a_second_save_replaces_the_first() {
        let dir = tmp_dir("overwrite");
        let path: PathBuf = dir.join("tui-panes.json");
        save_to(
            &path,
            PaneWidths {
                left: 30,
                right: 30,
            },
        )
        .expect("first");
        let second = PaneWidths {
            left: 45,
            right: 55,
        };
        save_to(&path, second).expect("second");
        assert_eq!(load_from(&path), Some(second));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 落盘内容是人也能读的 JSON：排查时 `cat` 一眼能看出存了什么。
    #[test]
    fn the_file_is_readable_json() {
        let dir = tmp_dir("readable");
        let path = dir.join("tui-panes.json");
        save_to(
            &path,
            PaneWidths {
                left: 33,
                right: 44,
            },
        )
        .expect("save");
        let text = fs::read_to_string(&path).expect("read");
        assert_eq!(text, r#"{"left":33,"right":44}"#);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 父路径是个**文件**时写不进去 —— 错误要冒出来（由调用方提示），不能假装成功。
    #[test]
    fn an_impossible_path_reports_an_error() {
        let dir = tmp_dir("impossible");
        let blocker: &Path = &dir.join("blocker");
        fs::write(blocker, b"x").expect("write blocker");
        assert!(save_to(&blocker.join("tui-panes.json"), PaneWidths::DEFAULT).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    /// `PaneStore` 是 lane 实际持有的那层：读一次、写一次，路径在构造时就定死。
    #[test]
    fn the_store_round_trips_through_one_path() {
        let dir = tmp_dir("store");
        let store = PaneStore::at(dir.join("tui-panes.json"));
        // 还没有文件 → 默认版面（首次运行不该报错、也不该是 0）。
        assert_eq!(store.load(), PaneWidths::DEFAULT);
        let widths = PaneWidths {
            left: 38,
            right: 48,
        };
        store.save(widths).expect("save");
        assert_eq!(store.load(), widths);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件坏了：读侧沉默回落默认 —— 这是**正常**路径（用户手改过 / 上次写到一半被杀）。
    #[test]
    fn a_corrupt_file_loads_as_the_default_layout() {
        let dir = tmp_dir("corrupt");
        let path = dir.join("tui-panes.json");
        fs::write(&path, b"{ not json").expect("write corrupt");
        assert_eq!(PaneStore::at(path).load(), PaneWidths::DEFAULT);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 没有任何可用目录时，写侧必须报出来（带变量名）：否则「拖完、重启、回默认」无从解释。
    #[test]
    fn an_unavailable_store_explains_itself_on_save() {
        let err = PaneStore::Unavailable
            .save(PaneWidths::DEFAULT)
            .expect_err("没有目录就存不了");
        assert!(err.contains("XDG_STATE_HOME"), "{err}");
        // 读侧反过来：没地方读就是默认版面，不打扰用户。
        assert_eq!(PaneStore::Unavailable.load(), PaneWidths::DEFAULT);
    }
}
