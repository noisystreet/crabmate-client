//! 会话草稿的本机落盘（issue #430）。
//!
//! 存什么：`api_base 分区 → conversation_id → 输入框全文`。
//!
//! **为什么分区**：`Drafts` 挂在每条 lane 上（一条连接一份），而 `conversation_id` 只在
//! **某一个** serve 内唯一。同时连两个 serve（`--connect`）时两边的 id 完全可能相同，
//! 扁平存成一张 `<id> → 文本` 的表会把 A 服务的草稿喂进 B 服务的输入框 —— 而且静默。
//! 分区键用 `api_base` 而不是 lane 名：lane 名可变（`/lane detach` 再 `attach` 同一个
//! serve 可以是另一个名字），`api_base` 才是这条连接的身份。
//!
//! **为什么另开一个文件**（而不是并进 `tui-panes.json`）：两者损坏时的正确处理不一样 ——
//! 列宽读不懂就回默认版面，草稿读不懂只能丢掉那几段字；塞进同一个文件会让「宽度坏了」
//! 连累「草稿也读不懂」，反之亦然。
//!
//! 目录、原子写、读侧静默回落沿用 [`pane_store`](super::super::pane_store) 的套路：落 `XDG_STATE_HOME`
//! 而不是 `XDG_CONFIG_HOME`（草稿是「上次打到一半的样子」，属可丢弃的运行状态，不是要
//! 人维护的配置），`CM_TUI_DRAFTS_FILE` 可覆盖路径，临时文件 + `rename` 落盘。
//!
//! 读侧**不报错**：没有 HOME / 没有文件 / JSON 坏了 / 字段缺失都回落空表。写侧把原因交回
//! 调用方，由它决定怎么告诉用户 —— 这里是纯 IO，不碰 `UiState`。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crabmate_client_api::url::normalize_api_base;

/// 覆盖落盘路径的环境变量（排查、或几套配置各存一份时用）；空串按未设置处理。
pub(crate) const PATH_ENV: &str = "CM_TUI_DRAFTS_FILE";

/// `XDG_STATE_HOME` / `~/.local/state` 下的相对文件名。
const FILE_NAME: &str = "tui-drafts.json";

/// 一个 `api_base` 分区：`conversation_id → 草稿全文`。
///
/// 用 `BTreeMap` 而不是 `HashMap`：写出去的文件因此按 id 排序，人工 `cat`、版本间 diff
/// 都稳定（同样的内容不会每次落盘就换个顺序）。
pub(crate) type Partition = BTreeMap<String, String>;

/// 落盘格式。
///
/// `#[serde(default)]`：手写成 `{}`、或将来加别的键时，缺 `drafts` 这一段不该让整份文件
/// 被判成「读不懂」。
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    drafts: BTreeMap<String, Partition>,
}

/// 分区键：`api_base` 归一后（去尾斜杠）。
///
/// 归一失败（空串 / 非 `http(s)://` —— 例如单测里随手写的 `lane-a`）就退回 trim 后的原串：
/// 有键总比所有 lane 挤进同一个 `""` 分区强。大小写不归一是**已知的**代价 ——
/// `HTTPS://…` 与 `https://…` 会分成两个分区，后果是丢一次草稿，不会串到另一个 serve。
fn partition_key(api_base: &str) -> String {
    let normalized = normalize_api_base(api_base);
    if normalized.is_empty() {
        api_base.trim().to_string()
    } else {
        normalized
    }
}

/// 解析落盘内容；读不懂就当作空表（首次运行、文件被删、手改坏都走这条）。
fn parse(text: &str) -> BTreeMap<String, Partition> {
    serde_json::from_str::<Stored>(text)
        .map(|s| s.drafts)
        .unwrap_or_default()
}

/// 落盘路径：`CM_TUI_DRAFTS_FILE` ＞ `$XDG_STATE_HOME` ＞ `$HOME/.local/state`。
///
/// 三个值都当参数传进来（而不是在函数里读环境变量），单测才能直接喂值 —— 测试里改进程
/// 环境与并行用例共享，属于数据竞争。三者都拿不到时返回 `None`（不落盘）：容器里既无
/// HOME 也无 XDG 时，猜一个路径写进去比「本次运行内草稿照常生效、只是不保存」更糟。
fn resolve_path(
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
fn store_path() -> Option<PathBuf> {
    let override_ = std::env::var_os(PATH_ENV);
    let state_home = std::env::var_os("XDG_STATE_HOME");
    let home = std::env::var_os("HOME");
    resolve_path(override_.as_deref(), state_home.as_deref(), home.as_deref())
}

/// 读整个文件（全部分区）；任何失败都是空表。
fn load_all_from(path: &Path) -> BTreeMap<String, Partition> {
    fs::read_to_string(path)
        .ok()
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// 落盘目的地。
///
/// 路径在**装配 lane 时**解析一次，之后随 lane 持有：切会话是高频动作，不该每次都去读
/// 一遍环境变量；而且「有没有地方可存」是启动时的一次性事实，中途不会变。
///
/// 这也顺带解决了测试隔离 —— 单测装配出来的 lane 显式指向临时文件，不会把草稿写进跑
/// 测试那个人的 `~/.local/state`。
#[derive(Debug, Clone)]
pub(crate) enum DraftStore {
    /// 落盘到这个文件（来自 [`store_path`]）。
    At(PathBuf),
    /// 环境里没有任何可用目录：**想存但存不了**。
    ///
    /// 与「不落盘」分开是因为下一步完全不同：这里值得告诉用户一次（否则「切走、重启、
    /// 草稿没了」查不出原因），而没有文件可读时沉默才是对的。
    Unavailable,
}

impl DraftStore {
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

    /// 读回某个 serve 的草稿分区；读不到就是空表。
    ///
    /// 只回这一个分区：**别的 serve 的草稿不该在这个输入框里出现**，哪怕 id 撞上了。
    pub(crate) fn load_partition(&self, api_base: &str) -> Partition {
        match self {
            Self::At(path) => {
                let mut all = load_all_from(path);
                all.remove(&partition_key(api_base)).unwrap_or_default()
            }
            Self::Unavailable => Partition::new(),
        }
    }

    /// 落盘某个 serve 的草稿分区；`Err` 是**给用户看**的原因（已带路径）。
    ///
    /// 空分区会把这个分区从文件里删掉（不是留一个 `{}`）：草稿全清空之后，文件不该继续
    /// 挂着一条空记录。
    pub(crate) fn save_partition(&self, api_base: &str, drafts: &Partition) -> Result<(), String> {
        match self {
            Self::At(path) => {
                save_to(path, api_base, drafts).map_err(|e| format!("{}：{e}", path.display()))
            }
            Self::Unavailable => Err(format!(
                "没有可用的本机状态目录（{PATH_ENV} / XDG_STATE_HOME / HOME 都为空）"
            )),
        }
    }
}

/// 原子写入指定文件（临时文件 + `rename`，同目录所以是同设备）。
///
/// **读-改-写**：文件里还有别的 serve 的分区（每条 lane 各写自己那份），直接覆盖会让
/// 「最后退出的那条 lane」独吞整个文件。
///
/// 读不懂时按空表处理并覆盖 —— 那是**自愈**：坏文件本来就取不出任何草稿，留着一份坏 JSON
/// 只会让之后的每次写入都从头开始。代价是坏文件里其它分区的内容一并丢掉，可接受。
pub(crate) fn save_to(path: &Path, api_base: &str, drafts: &Partition) -> std::io::Result<()> {
    let mut all = load_all_from(path);
    let key = partition_key(api_base);
    if drafts.is_empty() {
        all.remove(&key);
    } else {
        all.insert(key, drafts.clone());
    }
    write_atomic(path, &all)
}

/// 把整份表写到 `path`：`create_dir_all` + 临时文件 + `sync_all` + `rename`。
///
/// 为什么不用 `fs::write`：写一半被打断留下的半截 JSON 比「没有文件」更糟 —— 下次启动读到
/// 坏内容、静默回落空表，而用户以为草稿存下来了。
fn write_atomic(path: &Path, all: &BTreeMap<String, Partition>) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_vec(&Stored {
        drafts: all.clone(),
    })
    .map_err(std::io::Error::other)?;
    // 临时名带 pid：两个 `crabmate-tui` 同时退出时共用一个名字会互相覆盖 —— `rename` 不会
    // 留下半截文件，但会静默丢掉其中一次更新，失败分支的清理还可能删掉对方正在写的那个。
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
    use std::path::PathBuf;

    use super::{
        DraftStore, FILE_NAME, Partition, load_all_from, parse, partition_key, resolve_path,
        save_to,
    };

    /// 每个用例一个独立目录（并行跑测试不会互删）。
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crabmate-tui-drafts-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmp dir");
        dir
    }

    fn os(s: &str) -> Option<&OsStr> {
        Some(OsStr::new(s))
    }

    fn part(pairs: &[(&str, &str)]) -> Partition {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn explicit_path_wins_over_the_xdg_dirs() {
        assert_eq!(
            resolve_path(os("/tmp/d.json"), os("/state"), os("/home/u")),
            Some(PathBuf::from("/tmp/d.json"))
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

    /// 空串按「未设置」处理：`XDG_STATE_HOME=` 在 sudo / systemd 环境里很常见，当路径用会
    /// 拼出一个相对路径（写进当前目录）。
    #[test]
    fn blank_values_count_as_unset() {
        assert_eq!(resolve_path(os(""), os(""), os("")), None);
    }

    /// 一个路径都拼不出来时不落盘。
    #[test]
    fn no_directory_means_no_file() {
        assert_eq!(resolve_path(None, None, None), None);
    }

    /// 归一后同一个 serve：尾斜杠不能把它拆成两个分区。
    #[test]
    fn a_trailing_slash_does_not_split_the_partition() {
        assert_eq!(
            partition_key("http://127.0.0.1:8080/"),
            partition_key("http://127.0.0.1:8080")
        );
    }

    /// 非 URL（单测里随手写的 lane 名）也要有个稳定键，不能所有 lane 挤进 `""`。
    #[test]
    fn a_non_url_falls_back_to_the_trimmed_text() {
        assert_eq!(partition_key("  lane-a  "), "lane-a");
        assert_ne!(partition_key("lane-a"), partition_key("lane-b"));
    }

    /// **本模块存在的理由**：两个 serve 的草稿绝不互相串。
    #[test]
    fn partitions_do_not_leak_between_serves() {
        let dir = tmp_dir("isolation");
        let path = dir.join(FILE_NAME);
        save_to(&path, "http://a:1", &part(&[("c1", "写给 A")])).expect("save a");
        save_to(&path, "http://b:2", &part(&[("c1", "写给 B")])).expect("save b");

        assert_eq!(
            DraftStore::at(path.clone()).load_partition("http://a:1"),
            part(&[("c1", "写给 A")])
        );
        assert_eq!(
            DraftStore::at(path.clone()).load_partition("http://b:2"),
            part(&[("c1", "写给 B")]),
            "同一个 conversation_id 在两个 serve 下必须是两份草稿"
        );
        assert!(
            DraftStore::at(path).load_partition("http://c:3").is_empty(),
            "没存过的 serve 不该捡到别人的草稿"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 写自己那一份不能抹掉别条 lane 已经存下的分区（读-改-写）。
    #[test]
    fn saving_one_partition_keeps_the_others() {
        let dir = tmp_dir("keep-others");
        let path = dir.join(FILE_NAME);
        save_to(&path, "http://a:1", &part(&[("c1", "A 的")])).expect("save a");
        save_to(&path, "http://b:2", &part(&[("c9", "B 的")])).expect("save b");

        let all = load_all_from(&path);
        assert_eq!(all.len(), 2, "两条 lane 的分区都该在：{all:?}");
        assert_eq!(
            all.get("http://a:1").cloned(),
            Some(part(&[("c1", "A 的")]))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 空分区 = 这个 serve 没有草稿了 → 从文件里删掉，不留 `{}`。
    #[test]
    fn an_empty_partition_is_removed_from_the_file() {
        let dir = tmp_dir("empty");
        let path = dir.join(FILE_NAME);
        save_to(&path, "http://a:1", &part(&[("c1", "有内容")])).expect("save");
        save_to(&path, "http://a:1", &Partition::new()).expect("clear");
        assert!(load_all_from(&path).is_empty(), "空分区不该留下记录");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 写完再读回来是同一份值 —— 「切走、重启、草稿还在」的最小闭环。
    #[test]
    fn save_then_load_round_trips() {
        let dir = tmp_dir("roundtrip");
        let store = DraftStore::at(dir.join(FILE_NAME));
        assert!(
            store.load_partition("http://a:1").is_empty(),
            "首次运行空表"
        );

        let drafts = part(&[("c1", "第一句"), ("c2", "第二句")]);
        store.save_partition("http://a:1", &drafts).expect("save");
        assert_eq!(store.load_partition("http://a:1"), drafts);

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

    /// 文件坏了：读侧沉默回落空表 —— 这是**正常**路径（手改过 / 上次写到一半被杀）。
    #[test]
    fn a_corrupt_file_loads_as_an_empty_partition() {
        let dir = tmp_dir("corrupt");
        let path = dir.join(FILE_NAME);
        fs::write(&path, b"{ not json").expect("write corrupt");
        assert!(DraftStore::at(path).load_partition("http://a:1").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 手写的 `{}` / 缺字段都读得懂（不该因为少一个键就丢掉别的分区）。
    #[test]
    fn an_empty_object_and_unknown_keys_are_readable() {
        assert!(parse("{}").is_empty());
        assert!(parse(r#"{"future":true}"#).is_empty());
        assert_eq!(
            parse(r#"{"drafts":{"http://a:1":{"c1":"x"}}}"#).get("http://a:1"),
            Some(&part(&[("c1", "x")]))
        );
    }

    /// 坏文件在下次写入时被自愈替换掉（不留着一份永远读不懂的 JSON）。
    #[test]
    fn a_corrupt_file_is_replaced_on_the_next_save() {
        let dir = tmp_dir("self-heal");
        let path = dir.join(FILE_NAME);
        fs::write(&path, b"{ not json").expect("write corrupt");
        save_to(&path, "http://a:1", &part(&[("c1", "重新开始")])).expect("save");
        assert_eq!(
            DraftStore::at(path).load_partition("http://a:1"),
            part(&[("c1", "重新开始")])
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 没有可用目录时，写侧必须报出来（带变量名）：否则「切走、重启、草稿没了」无从解释。
    #[test]
    fn an_unavailable_store_explains_itself_on_save() {
        let err = DraftStore::Unavailable
            .save_partition("http://a:1", &part(&[("c1", "x")]))
            .expect_err("没有目录就存不了");
        assert!(err.contains("XDG_STATE_HOME"), "{err}");
        // 读侧反过来：没地方读就是空表，不打扰用户。
        assert!(
            DraftStore::Unavailable
                .load_partition("http://a:1")
                .is_empty()
        );
    }
}
