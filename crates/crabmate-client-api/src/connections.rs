//! 多条 `serve` 连接的**身份**与**状态**（Client 侧共用纯数据）。
//!
//! 为什么要有它：Desktop 与 TUI 走两条不同的多连接路线 —— Desktop 是「多窗口、
//! 每窗口一条连接」（`tauri_multi_server.md` §9.4.1 推荐的路线，每窗口一个独立 JS
//! heap，因此**不需要**给那 12 个 `thread_local` 做分区），TUI 是 lane。两条路线在
//! **显示层**必须用同一套词汇回答同一组问题：这条连接叫什么、此刻在做什么、一共几条、
//! 几条在等审批。否则「切到那条在等审批的连接」在两端就是两套说法。
//!
//! 它刻意只含数据：
//! - **没有 IPC、没有窗口 API**。Tauri 的窗口注册表（谁存在、焦点给谁、怎么广播）
//!   属于平台能力，在 `desktop-tauri`；本 crate 一旦引入 `tauri` 就会被
//!   `scripts/check-boundaries.sh` 的纯度门禁拒掉。
//! - **没有渲染**。终端画 `·` `●` `▲` `×`（宽度必须是 1 列，无色模式下符号就是
//!   唯一信息），Web 画 CSS 圆点。共享状态，不共享像素。
//! - **没有存储**。key 是将来「按连接分开存」的主键，但怎么存不在这里。

/// `ConnectionKey` 的最大长度。
///
/// 64 是给 Tauri window label 留的余量：label 会进窗口标题、日志与 IPC 参数，
/// 太长的派生名在人读的地方只会被截断。超过就截断而不是报错——派生是个纯函数，
/// 没有「向用户报错」的通道，而截断后的名字仍然可用（同一基址前缀相同的两条连接
/// 会撞名，这正是下面说「唯一性由调用方负责」的那个点）。
const MAX_KEY_LEN: usize = 64;

/// 产不出任何 slug 时的兜底名字。
const FALLBACK_KEY: &str = "serve";

/// 一条连接的稳定身份。
///
/// 两个消费者：TUI 的 lane（用户给的 lane 名）、Desktop 的窗口（窗口 label）。
/// 它同时是将来「按连接分区存储」的主键 —— 所以它必须**稳定**（同一条连接重连后
/// 不变）且**可作为 Tauri window label**。
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(from = "String")]
pub struct ConnectionKey(String);

impl ConnectionKey {
    /// 从用户给的名字（若有）或基址派生一个稳定 key。
    ///
    /// 规则与 TUI `/lane attach` 的口径一致——**名字优先，没名字才用基址**：
    /// 1. 有 `preferred` 就先把它 slug 化；
    /// 2. slug 为空（纯中文名、全是符号）时退回基址的 authority
    ///    （`127.0.0.1:8080` → `127-0-0-1-8080`，走 [`crate::url::api_base_authority`]，
    ///    因此 userinfo 同样不会漏进 key）；
    /// 3. 两者都产不出（空基址、空名字）→ [`FALLBACK_KEY`]。
    ///
    /// **只保证「同样输入给同样输出」，不保证唯一。** 两个窗口连同一台 serve 会得到
    /// 同一个 key —— 去重必须由调用方做：`desktop-tauri` 的注册表知道已有集合，可以
    /// 加 `-2` 后缀；这里没有那个集合，硬造序号只会产出一个不确定的值（而 key 一旦
    /// 不确定，「同一连接重连后状态还在不在」就无从回答）。
    ///
    /// 输出只含 `[a-z0-9-]`：这是 Tauri window label 合法字符集的一个子集，取子集是
    /// 为了不被各版本的字符集差异咬到，也不必在下游再校验一次。
    #[must_use]
    pub fn derive(preferred: Option<&str>, api_base: &str) -> Self {
        let from_name = preferred.map(slugify).filter(|s| !s.is_empty());
        let key = from_name.unwrap_or_else(|| {
            let authority = crate::url::api_base_authority(api_base);
            let from_base = slugify(&authority);
            if from_base.is_empty() {
                String::from(FALLBACK_KEY)
            } else {
                from_base
            }
        });
        Self(truncate(key))
    }

    /// 直接用一个已知合法的 key（从配置文件 / IPC 参数读回来的场景）。
    ///
    /// 仍然过一遍 [`slugify`]：IPC 参数来自另一个 WebView，不能假设它守法——
    /// 一个含空格或中文的 label 会在 Tauri 创建窗口时才炸，而那时用户已经点了
    /// 「新连接」，失败点离原因太远。
    ///
    /// **不是恒等函数**：`"My-Prod"` 会变成 `"my-prod"`。所以「读回一个 key 再判断
    /// 是不是同一条连接」不能靠字符串相等，要先经这里规范化再比。反序列化走的也是
    /// 这条路（见 `From<String>`），所以 IPC 与配置两条入口的规范化口径一致。
    #[must_use]
    pub fn from_raw(raw: &str) -> Self {
        let slug = slugify(raw);
        let key = if slug.is_empty() {
            String::from(FALLBACK_KEY)
        } else {
            slug
        };
        Self(truncate(key))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ConnectionKey {
    /// 反序列化 / IPC 入参的入口：一律经 [`ConnectionKey::from_raw`] 规范化。
    ///
    /// 用 `from` 而不是 `try_from`：不合法的 key 只有两处来源——我们自己早先版本
    /// 写下的、或用户手改的配置文件。规范化比拒绝更有用（拒绝会让一个窗口直接打不
    /// 开），而规范化之后的 key 仍然满足 label 约束。
    fn from(raw: String) -> Self {
        Self::from_raw(&raw)
    }
}

impl Default for ConnectionKey {
    /// **只服务测试 fixture**（配合 `ConnectionSnapshot::default()`）。
    ///
    /// 生产代码必须走 [`ConnectionKey::derive`] / [`ConnectionKey::from_raw`]：
    /// 默认值只是一条「无名连接」的占位，不代表任何真实 serve。给 `Default` 的原因是
    /// TUI 的 `LaneEntry` 当初就这么做的、且理由仍然成立——快照字段只会越加越多，
    /// 每加一个就改一遍所有测试构造函数，迟早漏掉一处。
    fn default() -> Self {
        Self(String::from(FALLBACK_KEY))
    }
}

impl std::fmt::Display for ConnectionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 一条连接此刻在做什么。
///
/// 用枚举而不是「busy / 待审批」两个 bool：后者允许「空闲又在等审批」这种非法组合，
/// 分支也会随标志数扩散（TUI 的 `LaneStatus` 已经用同一理由做过这个选择，这里只是
/// 把它提到共享层）。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ConnectionStatus {
    /// 没有在跑的回合。
    #[default]
    Idle,
    /// 回合进行中（审批之外的工作）。
    Running,
    /// 有审批在等决策。
    ///
    /// 这个状态值得进共享层，因为它是**唯一**能在别处提醒用户「那条连接卡住了」的
    /// 信号：TUI 的后台 lane 画不到主屏（issue #337），Desktop 的其他窗口同理。
    /// 没有它，一条连接会静默停摆到审批超时。
    WaitingApproval,
    /// 客户端正在退出（TUI：`/quit` 在等这一轮结束，issue #338）。
    ///
    /// **Desktop 的多窗口不产生这个状态**：一个窗口要走就真的走了，没有「留在原地等
    /// 结束」的阶段。留在共享枚举里是为了让 TUI 直接复用它（不必维护一份并行枚举再
    /// 转换），**不是**暗示 Desktop 该去实现它。
    Quitting,
}

impl ConnectionStatus {
    /// IPC / JSON 里的稳定字面量。
    ///
    /// 前端把状态上报给桌面壳时走的是字符串（Tauri 的 IPC 参数就是 JSON），壳侧按
    /// serde 派生的外部标记反序列化。两边的字面量必须逐字一致 —— 写死在这里，是让
    /// 「改枚举变体名」变成一处编译期可见的改动，而不是前端某个闭包里一句无声的
    /// 字符串。一致性由单测钉住（见 `status_wire_literals_match_the_serde_shape`）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running => "Running",
            Self::WaitingApproval => "WaitingApproval",
            Self::Quitting => "Quitting",
        }
    }
}

/// 一条连接的**渲染快照**：只含显示需要的字段。
///
/// 这是 TUI 的 `LaneEntry` 与 Desktop 连接列表的公共形状 —— 单连接就是长度为 1 的
/// 切片，所以两端的渲染代码消费的是同一个输入类型。
///
/// 刻意**不含**：
/// - **连接内部状态**（transcript / 草稿 / 游标）：渲染层拿不到 `&mut`，也就不可能在
///   某条连接的绘制里改到别的连接（TUI 侧的原注释就是这么写的，这里延续它）。
/// - **动画相位**（TUI 的 `pulse`）：终端靠「呼吸」字形，Web 靠 CSS transition，相位
///   是各自的实现细节，进共享层只会互相牵制。
/// - **基址原文**：留 [`Self::authority`] 就够；完整 URL 归设置页与该窗口自己的标题。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectionSnapshot {
    /// 稳定身份，同时是 Tauri window label。只含 `[a-z0-9-]`。
    pub key: ConnectionKey,
    /// 显示给用户的名字，**可以与 `key` 不同**。
    ///
    /// 分开的理由是它们受不同约束：`key` 必须是窗口 label（只能 ASCII slug），而用户
    /// 会给连接起中文名。合成一个字段就只剩两个坏选择——禁止中文名，或者让 label
    /// 变成一串乱码 slug 显示给用户。名字归名字，label 归 label。
    ///
    /// **改名请走 [`Self::rename`]，不要重建快照。** `new()` 会拿名字参与 `key` 的
    /// 派生（创建那一刻定身份，与 TUI 的 `/lane attach <name>` 同口径），所以重建
    /// 等于换一条连接：窗口 label 变、原 key 上的会话绑定与草稿变成孤儿。
    /// 只有直接赋值本字段或调 `rename` 才保得住 `key`。
    pub display_name: String,
    /// `host:port`（[`crate::url::api_base_authority`] 的结果，已去 userinfo）。
    pub authority: String,
    /// 是否是用户正在看的这一条。
    ///
    /// Desktop 单窗口恒 `true`；跨窗口列表里只有自己那条是 `true`。
    /// TUI 对应 `LaneEntry::active`。
    pub active: bool,
    /// 此刻在做什么。
    pub status: ConnectionStatus,
    /// 后台产生了新输出、用户还没切过去看（切过去即清零）。
    pub unread: bool,
    /// 上下文用量短文本（`62%` / `~1%`）；没有实测也没有基线时 `None`。
    pub context: Option<String>,
    /// 在途回合已耗时（秒）；空闲时 `None`。
    pub busy_secs: Option<u64>,
}

impl ConnectionSnapshot {
    /// 由「显示名 + 基址」建一条空闲连接。
    ///
    /// 走构造器而不是逐字段赋值：`key` / `authority` / `display_name` 三者必须出自
    /// 同一份输入。手写它们最容易写出的 bug 是「改了基址忘了改 key」，而它的症状
    /// ——点 lane 切错了窗口——离原因很远。
    ///
    /// 名字**参与** `key` 的派生：这是创建时定身份，与 TUI 的 `/lane attach <name>`
    /// 同口径。之后要改名用 [`Self::rename`]。
    ///
    /// 空名字回落 `authority`：`new("", base)` 若留一个空 `display_name`，渲染层会画出
    /// 「无名条目」，用户点它时无从判断是哪台 serve。回落后的名字与
    /// `ConnectionKey::derive(None, base)` 得到同一个 key，所以「没给名字」与「给了空
    /// 名字」不会变成两条不同的连接。
    #[must_use]
    pub fn new(display_name: impl Into<String>, api_base: &str) -> Self {
        let authority = crate::url::api_base_authority(api_base);
        let given = display_name.into();
        let display_name = if given.trim().is_empty() {
            if authority.is_empty() {
                String::from(FALLBACK_KEY)
            } else {
                authority.clone()
            }
        } else {
            given
        };
        let key = ConnectionKey::derive(Some(&display_name), api_base);
        Self {
            key,
            display_name,
            authority,
            active: false,
            status: ConnectionStatus::Idle,
            unread: false,
            context: None,
            busy_secs: None,
        }
    }

    /// 改名：**只动 `display_name`，不动 `key`**。
    ///
    /// 与 [`Self::new`] 的分工是「改一个名字」和「换一条连接」的区别，这个界面上
    /// 很容易混——所以做成两个方法而不是一个带 flag 的构造。
    ///
    /// 空名字被忽略（保留旧名）而不是把 `display_name` 清空：清空会让渲染层画出
    /// 无名条目，而调用方传空多半是输入框还没填完，不是真想清名。
    pub fn rename(&mut self, display_name: impl Into<String>) {
        let given = display_name.into();
        if !given.trim().is_empty() {
            self.display_name = given;
        }
    }

    /// 有没有在跑的工作（含等审批）。
    ///
    /// 把 `Running | WaitingApproval` 收在一处：两端都要用「是否占着」来决定要不要
    /// 画耗时、要不要呼吸；各写一份 `matches!` 迟早漏掉 `WaitingApproval`，
    /// 而漏掉它的后果正是「那条连接看起来空闲」。
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(
            self.status,
            ConnectionStatus::Running | ConnectionStatus::WaitingApproval
        )
    }
}

/// 由「回合是否在跑」与「是否有审批在等」合成一条连接此刻的状态。
///
/// 优先级是**等审批 > 运行中 > 空闲**。审批期间的回合确实还在跑，但「卡住等人」
/// 是更强、也是唯一需要跳出窗口提醒的那个信号（被盖住的窗口看不见自己的审批框），
/// 所以它压过 `Running`。写反的后果很具体：左栏那行的 `▲` 永远画不出来，而它正是
/// 用户从别的窗口发现「那条连接在等我」的唯一线索。
#[must_use]
pub fn connection_status_from_turn(turn_busy: bool, waiting_approval: bool) -> ConnectionStatus {
    if waiting_approval {
        ConnectionStatus::WaitingApproval
    } else if turn_busy {
        ConnectionStatus::Running
    } else {
        ConnectionStatus::Idle
    }
}

/// 未读标记的**下一个值**。
///
/// 三条规则，顺序即优先级：
/// 1. 窗口在前台 → 清零。用户正看着它，`•` 只剩噪音。
/// 2. 窗口在后台且回合在跑 → 置位。后台确实在产生输出。
/// 3. 其余（后台、已空闲）→ **保持原值**。回合结束后用户还没回来看，那个 `•` 是
///    「这儿有东西」的唯一线索，不能因为此刻没在跑就抹掉。
///
/// 返回「下一值」而不是「要不要变」：调用方每次重算即得终值，不必自己记住
/// 「这个 `•` 是不是我置的」才能决定该不该清 —— 那正是最容易与真实状态走散的地方。
#[must_use]
pub fn connection_unread_after(focused: bool, turn_busy: bool, was_unread: bool) -> bool {
    if focused {
        false
    } else if turn_busy {
        true
    } else {
        was_unread
    }
}

/// 连接计数：一共几条、其中几条在等审批。
///
/// 一个结构带两个数，是为了让两端都用同一份「怎么数」：TUI 的左栏标题 `连接 [n]`、
/// 状态栈的 `lanes` 段（`2/8 ▲1`），Desktop 的列表标题，数出来的必须是同一个数字。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectionTally {
    /// 已连接数：**含正在退出的那条**。
    ///
    /// 与 TUI 口径一致（`LaneTally::from_entries` 用 `entries.len()`，退出中的 lane
    /// 仍在列表里）：一条 lane 在 `/quit` 之后到真正退出之间可能还占着一整轮生成，
    /// 从列表里提前抹掉会让「我在等谁结束」这个问题失去答案。
    pub connected: usize,
    /// 正在等审批的连接数（0 时不显示审批徽标）。
    pub pending_approvals: usize,
}

impl ConnectionTally {
    /// 从一组状态数出计数。
    #[must_use]
    pub fn from_statuses(statuses: impl IntoIterator<Item = ConnectionStatus>) -> Self {
        let mut tally = Self::default();
        for status in statuses {
            tally.connected += 1;
            if status == ConnectionStatus::WaitingApproval {
                tally.pending_approvals += 1;
            }
        }
        tally
    }

    /// 从一组快照数出计数。
    ///
    /// 两端手上有的都是快照；各自再写一遍 `iter().filter(...)` 就等于把「怎么数」
    /// 复制成两份，而 `▲N` 与标题里的 `[n]` 对不上正是那种复制工程的典型症状。
    #[must_use]
    pub fn from_snapshots(snapshots: &[ConnectionSnapshot]) -> Self {
        Self::from_statuses(snapshots.iter().map(|snapshot| snapshot.status))
    }
}

/// slug 化：小写，`[a-z0-9]` 之外一律折叠成单个 `-`，去首尾 `-`。
///
/// 单个函数、无状态：`derive` 与 `from_raw` 共用它，两处各写一份的话，
/// 「IPC 传来的 key 和派生的 key 长得不一样」会变成一类难查的 bug。
fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            // 只有真的往后面接字符时才落前一个分隔符：这样连续分隔符合成一个，
            // 且开头不会被写上 `-`。
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// 截到 [`MAX_KEY_LEN`]，并去掉截断处可能留下的尾部 `-`。
///
/// 去掉尾部 `-` 是必须的：以 `-` 结尾的 label 在人读的地方像是被截坏的，而且
/// `slugify` 的「不留首尾分隔符」承诺会在截断这一步被打破。
///
/// 循环最多真正删除一次——[`slugify`] 已经把连续分隔符合成单个、也不留首尾，
/// 所以截断点之后不可能有第二个 `-`。这里仍写成 `while` 而不是 `if`，是因为
/// 这条前提属于 `slugify` 的实现细节；写成 `while` 后即使那条前提将来被放宽，
/// 这里也不会留下尾巴。（`slugify` 本身不会产出纯 `-` 的串，因此不存在清空的路径。）
fn truncate(mut key: String) -> String {
    if key.len() > MAX_KEY_LEN {
        key.truncate(MAX_KEY_LEN);
        while key.ends_with('-') {
            key.pop();
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_from_the_authority_when_no_name_is_given() {
        let key = ConnectionKey::derive(None, "http://127.0.0.1:8080");
        assert_eq!(key.as_str(), "127-0-0-1-8080");
    }

    #[test]
    fn a_credential_in_the_base_never_reaches_the_key() {
        // 与 `api_base_authority` 同一条规则：key 会出现在窗口标题与日志里，
        // 一个拼错进基址的凭据不能从这条路漏出去。
        let key = ConnectionKey::derive(None, "http://user:pass@serve.internal:8443/x");
        assert_eq!(key.as_str(), "serve-internal-8443");
    }

    #[test]
    fn a_name_wins_over_the_base_and_is_slugified() {
        assert_eq!(
            ConnectionKey::derive(Some("My Prod"), "http://127.0.0.1:8080").as_str(),
            "my-prod"
        );
        // 连续分隔符合成一个，首尾不留。
        assert_eq!(
            ConnectionKey::derive(Some("  a__b--c  "), "http://x").as_str(),
            "a-b-c"
        );
    }

    #[test]
    fn a_name_with_no_ascii_falls_back_to_the_base() {
        // 纯中文名 slug 化后是空串——不能产出空 key（空 label 会被 Tauri 拒绝），
        // 也不能把中文塞进 label。退到基址。
        let key = ConnectionKey::derive(Some("生产环境"), "http://127.0.0.1:9000");
        assert_eq!(key.as_str(), "127-0-0-1-9000");
    }

    #[test]
    fn nothing_usable_falls_back_to_the_constant() {
        assert_eq!(ConnectionKey::derive(None, "").as_str(), FALLBACK_KEY);
        assert_eq!(
            ConnectionKey::derive(Some("!!!"), "").as_str(),
            FALLBACK_KEY
        );
        assert_eq!(ConnectionKey::from_raw("  ").as_str(), FALLBACK_KEY);
    }

    #[test]
    fn from_raw_is_normalised_the_same_way_as_derive() {
        // IPC 参数来自另一个 WebView，不能假设它守法。
        assert_eq!(ConnectionKey::from_raw("Lane 2").as_str(), "lane-2");
        assert_eq!(ConnectionKey::from_raw("lane-2").as_str(), "lane-2");
    }

    #[test]
    fn keys_stay_within_the_label_budget() {
        let long = "a".repeat(MAX_KEY_LEN * 3);
        let key = ConnectionKey::derive(Some(&long), "http://x");
        assert_eq!(key.as_str().len(), MAX_KEY_LEN);

        // 截断处正好落在分隔符上时必须去掉它，否则会产出以 `-` 结尾的 label。
        // 用 `"a "` 重复：slug 是 `a-a-a-…`，第 MAX_KEY_LEN 位恰好是分隔符。
        let mut spaced = String::new();
        for _ in 0..(MAX_KEY_LEN + 8) {
            spaced.push_str("a ");
        }
        let key = ConnectionKey::derive(Some(&spaced), "http://x");
        assert!(!key.as_str().ends_with('-'), "{key}");
        assert!(
            key.as_str().len() < MAX_KEY_LEN,
            "截断后应去掉尾部分隔符（比上限短一列）：{key}"
        );
    }

    #[test]
    fn every_key_is_a_legal_label_subset() {
        // 输出必须落在 `[a-z0-9-]`：这是 Tauri window label 的合法子集，
        // 也是本模块对下游唯一的承诺。
        for raw in [
            "My Prod / v2",
            "https://user:pw@host:8443/base",
            "生产环境",
            "a b\tc",
            "!!!",
        ] {
            let key = ConnectionKey::derive(Some(raw), raw);
            assert!(!key.as_str().is_empty(), "{raw}");
            assert!(
                key.as_str()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{raw} → {key}"
            );
            assert!(!key.as_str().starts_with('-') && !key.as_str().ends_with('-'));
        }
    }

    #[test]
    fn tally_counts_connections_and_pending_approvals() {
        use ConnectionStatus::{Idle, Running, WaitingApproval};
        let tally =
            ConnectionTally::from_statuses([Idle, WaitingApproval, Running, WaitingApproval]);
        assert_eq!(tally.connected, 4);
        assert_eq!(tally.pending_approvals, 2);
        assert_eq!(ConnectionTally::from_statuses([]).connected, 0);
    }

    #[test]
    fn a_snapshot_keeps_a_chinese_name_but_a_slug_key() {
        // 两个字段分开的理由：中文名要留住，key 必须是 ASCII。
        let snapshot = ConnectionSnapshot::new("生产环境", "http://user:pw@10.0.0.5:8080/base");
        assert_eq!(snapshot.display_name, "生产环境");
        assert_eq!(snapshot.key.as_str(), "10-0-0-5-8080");
        // 凭据既不进 key，也不进 authority。
        assert_eq!(snapshot.authority, "10.0.0.5:8080");
        assert!(!snapshot.is_busy());
    }

    #[test]
    fn the_same_name_and_base_yield_the_same_key() {
        // key 必须只由输入决定 —— 它是「同一连接重连后状态还在不在」的答案。
        let a = ConnectionSnapshot::new("prod", "http://127.0.0.1:8080");
        let b = ConnectionSnapshot::new("prod", "http://127.0.0.1:8080");
        assert_eq!(a.key, b.key);
    }

    #[test]
    fn renaming_keeps_the_key_but_rebuilding_does_not() {
        // 本模块最容易搞错的一处分界：`rename` 改名字，`new` 换身份。
        let mut snapshot = ConnectionSnapshot::new("prod", "http://127.0.0.1:8080");
        let key_before = snapshot.key.clone();

        snapshot.rename("production");
        assert_eq!(snapshot.key, key_before, "rename 不能动 key");
        assert_eq!(snapshot.display_name, "production");

        // 空名字被忽略、保留旧名：输入框还没填完 ≠ 想清名。
        snapshot.rename("   ");
        assert_eq!(snapshot.display_name, "production");

        // 重建就是「另一条连接」—— 这正是改名不能用 new 的理由
        // （key 一换，原 key 上的会话绑定与草稿就成了孤儿）。
        let rebuilt = ConnectionSnapshot::new("production", "http://127.0.0.1:8080");
        assert_ne!(rebuilt.key, key_before);
    }

    #[test]
    fn an_empty_name_falls_back_to_the_authority() {
        // 空名字会画出「无名条目」，用户点它时无从判断是哪台 serve。
        let snapshot = ConnectionSnapshot::new("   ", "http://127.0.0.1:8080");
        assert_eq!(snapshot.display_name, "127.0.0.1:8080");
        // 且必须与「没给名字」同一个 key，否则空名字会分裂成另一条连接。
        assert_eq!(
            snapshot.key,
            ConnectionKey::derive(None, "http://127.0.0.1:8080")
        );

        // 基址也空时退到兜底常量，不留空串。
        let empty = ConnectionSnapshot::new("", "");
        assert!(!empty.display_name.is_empty());
        assert_eq!(empty.key.as_str(), FALLBACK_KEY);
    }

    #[test]
    fn serde_round_trips_a_snapshot() {
        // 跨窗口广播全靠这一条：形状能在两个 WebView 之间走一个来回而不变。
        let mut snapshot = ConnectionSnapshot::new("生产环境", "http://127.0.0.1:8080");
        snapshot.status = ConnectionStatus::WaitingApproval;
        snapshot.unread = true;
        snapshot.context = Some(String::from("62%"));

        let json = serde_json::to_string(&snapshot).expect("serialize");
        let back: ConnectionSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, snapshot);
        assert_eq!(back.key.as_str(), "127-0-0-1-8080");
    }

    #[test]
    fn deserialising_a_key_normalises_it() {
        // 反序列化也走 from_raw：不合法的 label 不会从 IPC 那头漏进来。
        let key: ConnectionKey = serde_json::from_str("\"My Prod\"").expect("deserialize");
        assert_eq!(key.as_str(), "my-prod");
    }

    #[test]
    fn tally_counts_a_quitting_connection_as_connected() {
        // 与 TUI 口径一致：`/quit` 之后那条在真正退出前仍占一个位置。
        let tally = ConnectionTally::from_statuses([ConnectionStatus::Quitting]);
        assert_eq!(tally.connected, 1);
        assert_eq!(tally.pending_approvals, 0);
    }

    #[test]
    fn busy_covers_approval_waiting_too() {
        // 漏掉 WaitingApproval 的后果是「那条连接看起来空闲」—— 而它恰恰是那条连接
        // 最需要被看见的时刻。两端共用这一个判定，就不必各自 `matches!` 一遍。
        let mut snapshot = ConnectionSnapshot::new("x", "http://x");
        assert!(!snapshot.is_busy());
        snapshot.status = ConnectionStatus::Running;
        assert!(snapshot.is_busy());
        snapshot.status = ConnectionStatus::WaitingApproval;
        assert!(snapshot.is_busy());
        snapshot.status = ConnectionStatus::Idle;
        assert!(!snapshot.is_busy());
    }

    #[test]
    fn tally_from_snapshots_matches_from_statuses() {
        let mut running = ConnectionSnapshot::new("a", "http://a:1");
        running.status = ConnectionStatus::Running;
        let mut waiting = ConnectionSnapshot::new("b", "http://b:2");
        waiting.status = ConnectionStatus::WaitingApproval;
        let idle = ConnectionSnapshot::new("c", "http://c:3");

        let tally = ConnectionTally::from_snapshots(&[running, waiting, idle]);
        assert_eq!(tally.connected, 3);
        assert_eq!(tally.pending_approvals, 1);
    }

    #[test]
    fn a_pending_approval_outranks_a_running_turn() {
        // 审批期间回合也在跑，但 `▲` 是唯一能让用户从别的窗口发现「这条卡住了」的信号。
        assert_eq!(
            connection_status_from_turn(true, true),
            ConnectionStatus::WaitingApproval
        );
        assert_eq!(
            connection_status_from_turn(true, false),
            ConnectionStatus::Running
        );
        assert_eq!(
            connection_status_from_turn(false, false),
            ConnectionStatus::Idle
        );
    }

    #[test]
    fn unread_is_cleared_by_focus_and_kept_after_the_turn_ends() {
        // 前台：无论此前有没有 `•` 都清掉。
        assert!(!connection_unread_after(true, true, true));
        assert!(!connection_unread_after(true, false, true));
        // 后台且回合在跑：置位。
        assert!(connection_unread_after(false, true, false));
        // 后台且已空闲：保住原值（回合结束了，但用户还没回来看）。
        assert!(connection_unread_after(false, false, true));
        assert!(!connection_unread_after(false, false, false));
    }

    #[test]
    fn status_wire_literals_match_the_serde_shape() {
        // 壳侧按 serde 派生的外部标记反序列化 `report_connection_status(status)`，
        // 前端上报的是 `as_str()`：两边的字面量一旦走散，上报会在壳的反序列化那步
        // 失败（命令是 fire-and-forget，症状只会是「行不更新」）。
        for status in [
            ConnectionStatus::Idle,
            ConnectionStatus::Running,
            ConnectionStatus::WaitingApproval,
            ConnectionStatus::Quitting,
        ] {
            assert_eq!(
                serde_json::to_value(status).expect("serialize"),
                serde_json::Value::from(status.as_str())
            );
        }
    }
}
