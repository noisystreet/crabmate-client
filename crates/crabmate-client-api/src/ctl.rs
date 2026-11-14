//! `crabmatectl` 本地控制面（unix socket 上**行分隔 JSON**）的纯逻辑部分。
//!
//! 只负责三件事：请求行怎么拼、响应怎么解、**能不能连**怎么判。真正的 socket IO
//! 在各端自己那层（TUI 在 `crates/crabmate-tui/src/serve/ctl.rs`）—— 本 crate 无 IO，
//! 也不允许有（`AGENTS.md` 的 client-api 纯度约束）。
//!
//! 契约出处：Server `src/cm_ctl/control.rs` + `docs/命令行与路由.md` 的 `--socket` 一节。
//! **权威在 Server**：这里只镜像它的字段与错误码，不复制它的判断。
//!
//! 三条本模块刻意固化的语义（都由 Server 侧文档明确）：
//!
//! 1. **`observed` 才是可连性判据**，不是 `state`。`state` 是落盘四态；`observed` 是
//!    读取时派生的观测态。只看 `state` 会把死实例当活的，连过去打空端口。
//! 2. **错误码是契约、`error` 文案不是**。分类只读 `code`，从不解析人读文案
//!    —— 与 `workspace::classify_workspace_set_failure` 同一口径。
//! 3. **认不出的值一律不猜**：未知 `observed` / 未知 `code` 各自落 `Unknown` / `Other`，
//!    而不是折进某个已知分支。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::messages::top_level_error_code;

/// `op` 取值（契约，不要改写大小写）。
pub const OP_ADD: &str = "add";
/// 见 [`OP_ADD`]。
pub const OP_REMOVE: &str = "remove";
/// 见 [`OP_ADD`]。
pub const OP_LIST: &str = "list";
/// 见 [`OP_ADD`]。
pub const OP_DISABLE: &str = "disable";
/// 见 [`OP_ADD`]。
pub const OP_ENABLE: &str = "enable";
/// 见 [`OP_ADD`]：**一条 lane** 的详情（响应体就是 `status --json` 的那一行）。
pub const OP_STATUS: &str = "status";
/// 见 [`OP_ADD`]：同步重启**一条 lane**（down → up → 等就绪才回一行）。
pub const OP_RESTART: &str = "restart";

/// `restart` 的 `grace` + `timeout` 之和上限（秒）。
///
/// 契约在 Server：超过就回 `restart_budget_exceeded`，更大的宽限只能走 CLI。
/// 客户端**不**据此提前拒绝（拒绝文案与错误码都是 Server 的权威），但 IO 侧的读超时
/// 必须明显长于它 —— 这个 op 是同步的，响应可能真占满整个预算。
pub const RESTART_BUDGET_SECS: u64 = 50;

/// 一条控制面请求：序列化后正好一行 JSON。
///
/// 字段集**必须**与 Server `Request::parse` 里的手写白名单一致（那边因为 serde 的
/// `deny_unknown_fields` 对 internally tagged enum 不生效而额外校验）：多一个字段
/// 会被 `bad_request` 拒掉，所以这里也绝不夹带 `config` 之类的额外键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtlRequest {
    /// `{"op":"list"}`；`include_unmanaged` 为真时并入「有记录但无声明」的实例。
    List {
        /// 是否把人工 `up` 起、声明里没有的实例也列出来（命令行 `up` 的视野）。
        include_unmanaged: bool,
    },
    /// `{"op":"add",...}`：新增一条声明。已存在时 Server 返回 `created:false` 且**不改动**。
    Add {
        /// 实例名（Server 侧过 `validate_name`）。
        name: String,
        /// 工作区路径。**必填**：Server 的 `AddRequest.workspace` 是 `String`（见模块注释与
        /// Server `08291db6` 的「socket `add` 仍必填」），待绑定的「省略」只对 TOML 声明成立。
        workspace: String,
        /// `profiles.toml` 里的预设名；不接受内联配置。
        profile: Option<String>,
        /// 指定端口；`None` 由端口池分配。
        port: Option<u16>,
        /// `false` = 声明保留但不该运行（Server 侧称之为「客户端摘 lane 的软删除语义」）。
        enabled: bool,
    },
    /// `{"op":"remove",...}`：删掉一条声明（不存在也返回成功，幂等）。
    Remove {
        /// 实例名。
        name: String,
    },
    /// `{"op":"disable","name":…}` / `{"op":"enable","name":…}`：只翻声明里的 `enabled`。
    ///
    /// 用一个变体带 `enabled` 而不是 `Disable` / `Enable` 两个变体：op 字符串是 `enabled`
    /// 的函数，拆成两支会让 [`Self::to_line`] 多一个分支（它贴着 CCN≤10）。
    ///
    /// Server 的字段白名单对这两个 op 只收 `op` + `name`（它们**不是** `update`）：
    /// 换 `workspace` / `profile` / `port` 仍走 `remove` + `add` 的显式两步。
    SetEnabled {
        /// 实例名（Server 侧过 `validate_name`）。
        name: String,
        /// `false` = 保留声明但不该运行（客户端摘 lane 的软删除）；`true` = 放开。
        enabled: bool,
    },
    /// `{"op":"status","name":…}`：**一条**实例的详情。
    ///
    /// 响应体就是 `status --json` 的那一行（字段集钉在 Server 的
    /// `fixtures/ctl_contract.json` 的 `status_row`），外加一个 `ok` —— 所以解析复用
    /// [`CtlInstance`]，**不**另写一套字段。拿单条详情不必再 `list` 取全量自己筛。
    Status {
        /// 实例名。
        name: String,
    },
    /// `{"op":"restart","name":…,"grace":…,"timeout":…}`：同步重启一条实例。
    ///
    /// `grace` / `timeout` 省略时各取 Server 的默认 10 s；两者之和不得超过
    /// [`RESTART_BUDGET_SECS`]（超出由 Server 回 `restart_budget_exceeded`）。
    /// **这个 op 是同步的**：Server 跑完 down → up → 等就绪才回这一行，所以调用方必须
    /// 把它放在 worker / 异步任务里，读超时也要按 [`RESTART_BUDGET_SECS`] 量级来设。
    Restart {
        /// 实例名。
        name: String,
        /// 优雅停止的宽限（秒）；`None` = Server 默认。
        grace: Option<u64>,
        /// 等新进程就绪的上限（秒）；`None` = Server 默认。
        timeout: Option<u64>,
    },
}

impl CtlRequest {
    /// 拼成一行 JSON（不带换行；由调用方写 `\n`）。
    #[must_use]
    pub fn to_line(&self) -> String {
        let mut obj = serde_json::Map::new();
        match self {
            Self::List { include_unmanaged } => {
                obj.insert("op".to_string(), json!(OP_LIST));
                // 缺省即 false：只在真需要时写出，和 Server 的 `#[serde(default)]` 对上。
                if *include_unmanaged {
                    obj.insert("include_unmanaged".to_string(), json!(true));
                }
            }
            Self::Add {
                name,
                workspace,
                profile,
                port,
                enabled,
            } => {
                obj.insert("op".to_string(), json!(OP_ADD));
                obj.insert("name".to_string(), json!(name));
                obj.insert("workspace".to_string(), json!(workspace));
                if let Some(p) = profile {
                    obj.insert("profile".to_string(), json!(p));
                }
                if let Some(p) = port {
                    obj.insert("port".to_string(), json!(p));
                }
                // Server 侧 `enabled` 默认 true，且序列化时 true 省略；这里同样只在 false 时写出。
                if !*enabled {
                    obj.insert("enabled".to_string(), json!(false));
                }
            }
            Self::Remove { name } => {
                obj.insert("op".to_string(), json!(OP_REMOVE));
                obj.insert("name".to_string(), json!(name));
            }
            Self::SetEnabled { name, enabled } => {
                let op = if *enabled { OP_ENABLE } else { OP_DISABLE };
                obj.insert("op".to_string(), json!(op));
                obj.insert("name".to_string(), json!(name));
            }
            Self::Status { name } => {
                obj.insert("op".to_string(), json!(OP_STATUS));
                obj.insert("name".to_string(), json!(name));
            }
            Self::Restart {
                name,
                grace,
                timeout,
            } => {
                obj.insert("op".to_string(), json!(OP_RESTART));
                obj.insert("name".to_string(), json!(name));
                // 两个字段缺省即 Server 的默认 10 s：只在显式给了时才写出
                // （Server 对这两个 op 的字段白名单收它们，多写一个别的键就是 `bad_request`）。
                if let Some(secs) = grace {
                    obj.insert("grace".to_string(), json!(secs));
                }
                if let Some(secs) = timeout {
                    obj.insert("timeout".to_string(), json!(secs));
                }
            }
        }
        Value::Object(obj).to_string()
    }
}

/// 控制面实例的**观测态**（`observed`），可连性的唯一判据。
///
/// 与落盘态 [`CtlInstance::state`] 并列而**不等价**：`state` 说「记录里写着什么」，
/// 这里说「此刻真的连得上吗」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtlObserved {
    /// 进程在跑且健康检查通过：**唯一**可以连的状态。
    Ready,
    /// 启动中的中间态（Server 侧 `up` 自己的就绪等待才是它的裁判）。
    Starting,
    /// 记录说在跑，但连不上 / 进程身份不符（Server 刻意不落盘，只在读取时派生）。
    Stale,
    /// 已退出。
    Stopped,
    /// 启动失败，原因在 [`CtlInstance::last_error`]。
    Failed,
    /// 被保留但当前不应运行。
    Held,
    /// 响应里没有 `observed` 字段（旧版 ctl 或缺记录）。
    Absent,
    /// 认不出的取值：原样带着，不折进上面任何一支。
    Unknown(String),
}

impl CtlObserved {
    /// 从线上的 `observed` 字符串解析。空白与缺省都落 [`CtlObserved::Absent`]。
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self::Absent;
        };
        match text {
            "ready" => Self::Ready,
            "starting" => Self::Starting,
            "stale" => Self::Stale,
            "stopped" => Self::Stopped,
            "failed" => Self::Failed,
            "held" => Self::Held,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// 这条实例现在能不能连。
    ///
    /// **只有 [`CtlObserved::Ready`] 为真**：`stale` 是「记录说在跑但连不上」，
    /// `starting` 是「还起不来」，其余更不必说；`Absent` / `Unknown` 一律按不可连处理
    /// —— 拿不准的时候不连，比连到空端口再报错好排查。
    #[must_use]
    pub fn can_connect(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// `list` 响应里的一条实例。字段全部可选：Server 可能新增字段，客户端不得因此解析失败。
///
/// 刻意**不加** `deny_unknown_fields`：加字段是兼容变更，客户端必须容忍。
///
/// `Serialize` 是给壳用的：Desktop 的 `ctl_list_instances` 直接把它回给前端
/// （WASM 侧反序列化回同一个类型，再调 [`CtlInstance::can_connect`]），形状只留这一处。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CtlInstance {
    /// 实例名（`instances.toml` 的键）。
    pub name: String,
    /// 落盘态（四态）。**不要**用它判可连性，见 [`CtlObserved`]。
    #[serde(default)]
    pub state: Option<String>,
    /// 声明期望的态（`running` / `stopped`）：`state` 与它不一致说明 ctl 正在收敛路上。
    ///
    /// 只有 `status` / `restart` 的响应带它（`list` 的行里 Server 也给，缺省即 `None`），
    /// 所以这里按可选处理 —— 详情视图据此说得清「为什么它是 `starting`」。
    #[serde(default)]
    pub desired_state: Option<String>,
    /// 观测态；判可连性用它。
    #[serde(default)]
    pub observed: Option<String>,
    /// 已就绪时的 serve 根 URL。
    #[serde(default)]
    pub base_url: Option<String>,
    /// 端口。
    #[serde(default)]
    pub port: Option<u16>,
    /// 进程号。
    #[serde(default)]
    pub pid: Option<u32>,
    /// 最近一次失败原因（`failed` 时最有意义）。
    #[serde(default)]
    pub last_error: Option<String>,
    /// 声明里写的工作区（可能为 `None` = 待绑定）。
    #[serde(default)]
    pub workspace: Option<String>,
    /// `profiles.toml` 预设名。
    #[serde(default)]
    pub profile: Option<String>,
    /// 声明上是否启用。
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 是否来自声明（`false` = 只有状态记录，人工 `up` 起的）。
    #[serde(default)]
    pub declared: Option<bool>,
    /// 谁拉起的（`cli` / `supervise`）。
    #[serde(default)]
    pub managed_by: Option<String>,
}

impl CtlInstance {
    /// 解析后的观测态。
    #[must_use]
    pub fn observed_state(&self) -> CtlObserved {
        CtlObserved::parse(self.observed.as_deref())
    }

    /// 便捷判据：等价于 `self.observed_state().can_connect()`。
    #[must_use]
    pub fn can_connect(&self) -> bool {
        self.observed_state().can_connect()
    }
}

/// 控制面错误的机器可读分类（按 `code`，不看 `error` 文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtlErrorKind {
    /// `bad_request`：JSON / 字段 / `op` 不合法。
    BadRequest,
    /// `bad_workspace`：`add` 的 `workspace` 不存在或不是目录。
    BadWorkspace,
    /// `socket_in_use`：该 socket 已有 supervise 在服务（一个 socket 只能一个写入者）。
    SocketInUse,
    /// `supervisor_gone`：收敛主循环已退出。
    SupervisorGone,
    /// `empty_request`：空行。
    EmptyRequest,
    /// `handle_timeout`：主循环在超时内没处理完（可能卡在起实例）。
    HandleTimeout,
    /// `instance_not_found`：`status` / `restart` 找不到该实例的**状态记录**。
    ///
    /// 与「声明里没有它」是两件事：声明可能还在、只是从没起过（停掉的实例状态文件会消失）。
    InstanceNotFound,
    /// `restart_budget_exceeded`：`grace + timeout` 超过 [`RESTART_BUDGET_SECS`]。
    ///
    /// 更大的宽限只能走 CLI（`crabmatectl restart <name> --grace …`），客户端不放宽上限。
    RestartBudgetExceeded,
    /// `port_in_use`：重启时要绑的端口被占（Server 在重启内部冒出）。
    PortInUse,
    /// `identity_mismatch`：端口上的进程不是 ctl 起的那个（Server 在重启内部冒出）。
    IdentityMismatch,
    /// 响应体不是合法 JSON / 结构对不上（**客户端侧**判定，Server 不会发这个码）。
    Malformed,
    /// 认不出的 `code`（Server 将来新增）：原样带上，不折进上面任何一支。
    Other,
}

/// `code` → 分类的**唯一**映射表。
///
/// 表而不是 `match`：`from_code` 已经贴着 CCN 门禁（每多一个码就多一个分支），而这张表
/// 只是数据 —— 新增一个码是加一行，不会把函数推过 10。
const CODE_KINDS: &[(&str, CtlErrorKind)] = &[
    ("bad_request", CtlErrorKind::BadRequest),
    ("bad_workspace", CtlErrorKind::BadWorkspace),
    ("socket_in_use", CtlErrorKind::SocketInUse),
    ("supervisor_gone", CtlErrorKind::SupervisorGone),
    ("empty_request", CtlErrorKind::EmptyRequest),
    ("handle_timeout", CtlErrorKind::HandleTimeout),
    ("instance_not_found", CtlErrorKind::InstanceNotFound),
    (
        "restart_budget_exceeded",
        CtlErrorKind::RestartBudgetExceeded,
    ),
    ("port_in_use", CtlErrorKind::PortInUse),
    ("identity_mismatch", CtlErrorKind::IdentityMismatch),
];

impl CtlErrorKind {
    /// 按线上 `code` 分类；表里没有的一律 [`CtlErrorKind::Other`]（不猜成某个已知原因）。
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        CODE_KINDS
            .iter()
            .find(|(known, _)| *known == code)
            .map_or(Self::Other, |(_, kind)| *kind)
    }
}

/// 一条控制面错误：分类 + 原样保留的 `code` 与服务端文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtlError {
    /// 机器可读分类。
    pub kind: CtlErrorKind,
    /// 线上 `code`（`Malformed` 时为 `None`）。
    pub code: Option<String>,
    /// 面向人的文案：**不承诺措辞**，调用方可换成本地化默认句。
    pub message: String,
}

impl CtlError {
    /// 响应体不可解析 / 结构不符。
    fn malformed(detail: impl Into<String>) -> Self {
        Self {
            kind: CtlErrorKind::Malformed,
            code: None,
            message: detail.into(),
        }
    }
}

/// 从 `{"ok":false,"code":…,"error":…}` 里解出一条 [`CtlError`]。
///
/// `code` 用**顶层**读取（[`top_level_error_code`]）：控制面是扁平错误体，
/// 与受保护 API 的嵌套 `{"error":{"code":…}}` 不是同一个形状、也不互相回退。
fn error_from_value(val: &Value) -> CtlError {
    let code = top_level_error_code(val).map(str::to_string);
    let kind = code
        .as_deref()
        .map_or(CtlErrorKind::Other, CtlErrorKind::from_code);
    let message = val
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    CtlError {
        kind,
        code,
        message,
    }
}

/// 解析 `list` 的**一行**响应。
///
/// 成功时返回实例列表（顺序沿用 Server 给的声明顺序）；`ok:false` 时按 `code` 分类
/// 成 [`CtlError`]；整体不是 JSON 或 `instances` 结构不符时为
/// [`CtlErrorKind::Malformed`]。
pub fn parse_list_response(line: &str) -> Result<Vec<CtlInstance>, CtlError> {
    let val = parse_ok_body(line)?;
    // `instances` 缺省按空列表（Server 成功响应总会带，但少一次失败更好）。
    let items = val
        .get("instances")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(items)
        .map_err(|e| CtlError::malformed(format!("instances 结构不符：{e}")))
}

/// 解析一行响应并确认 `ok: true`，返回整个对象（成功体的字段由调用方各取所需）。
///
/// 三种失败（不是 JSON / `ok:false` / 结构不符）都在这里收口，调用方只处理成功形状。
fn parse_ok_body(line: &str) -> Result<Value, CtlError> {
    let val: Value = serde_json::from_str(line.trim())
        .map_err(|e| CtlError::malformed(format!("响应不是 JSON：{e}")))?;
    if val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        Ok(val)
    } else {
        Err(error_from_value(&val))
    }
}

/// `add` 的响应（`{"ok":true,"name":…,"created":…}`）。
///
/// `created: false` **不是失败**：Server 对已声明的实例幂等返回且不改动它（覆盖会
/// 悄悄丢掉手工写在声明里的 `env`），此时 `note` 说明接下来该怎么做。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddOutcome {
    /// 实例名（回显 Server 给的值，不用请求里那份——两者不同就说明契约变了）。
    pub name: String,
    /// 本次是否真的新建了声明。
    pub created: bool,
    /// Server 的补充说明；不承诺措辞。
    pub note: Option<String>,
}

/// 解析 `add` 的**一行**响应。失败分类口径与 [`parse_list_response`] 相同。
pub fn parse_add_response(line: &str) -> Result<AddOutcome, CtlError> {
    let val = parse_ok_body(line)?;
    Ok(AddOutcome {
        name: val
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created: val.get("created").and_then(Value::as_bool).unwrap_or(false),
        note: val
            .get("note")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string),
    })
}

/// `disable` / `enable` 的响应（`{"ok":true,"name":…,"enabled":…,"changed":…}`）。
///
/// 三条语义都来自 Server，客户端不要自己重推：
///
/// 1. `changed: false` 是**幂等命中**（值本就一致，或声明里没有这条），不是失败 ——
///    重试安全，所以 UI 该说「本来就是这样」，而不是报错；
/// 2. `enable` 会顺手收回 `down` 留下的 `desired_state = stopped`（`cleared_down`），
///    否则收敛永远 `Skipped`：声明写着 `enabled = true`，实例却起不来；
/// 3. 那半步失败时 Server 仍回 `ok:true`（`enabled` 已写盘），用 `warning` 说明 ——
///    必须让它露出来，否则客户端会干等一个不会启动的实例。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleOutcome {
    /// 实例名（回显 Server 给的值）。
    pub name: String,
    /// 期望的 `enabled`；响应缺该字段时为 `None`（调用方按请求值兜底）。
    pub enabled: Option<bool>,
    /// 本次是否真的改了声明。
    pub changed: bool,
    /// `enable` 是否顺手清掉了 `down` 标记。
    pub cleared_down: bool,
    /// Server 的补充说明（如「声明中不存在该实例」）；不承诺措辞。
    pub note: Option<String>,
    /// Server 的警告（半边写成了、半边没做成）；不承诺措辞。
    pub warning: Option<String>,
}

/// `disable` / `enable` 失败时带回的上下文：界面要能说清**哪条实例**、**往哪个方向**翻。
///
/// 与 [`CtlError`] 分工：[`CtlError`] 是协议/传输层的分类（连不上 / 读写失败 / 被拒绝），
/// 这里只是把请求的意图原样带回 —— 两者合起来才是用户看到的那一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleFailure {
    /// 实例名。
    pub name: String,
    /// 请求的意图：`false` = 关闭。
    pub enabled: bool,
    /// 面向用户的失败原因（已含 `CtlCallError::user_message` 的分类语义）。
    pub reason: String,
}

/// 解析 `disable` / `enable` 的**一行**响应。失败分类口径与 [`parse_list_response`] 相同。
pub fn parse_toggle_response(line: &str) -> Result<ToggleOutcome, CtlError> {
    let val = parse_ok_body(line)?;
    Ok(ToggleOutcome {
        name: val
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        enabled: val.get("enabled").and_then(Value::as_bool),
        changed: val.get("changed").and_then(Value::as_bool).unwrap_or(false),
        cleared_down: val
            .get("cleared_down")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        note: opt_text(&val, "note"),
        warning: opt_text(&val, "warning"),
    })
}

/// 取一个可选的说明字段：缺省、非字符串、空白都归一成 `None`。
fn opt_text(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `restart` 的响应：同一份 `status_row` + `restarted` + `notes`。
///
/// `notes` 是 CLI 打到 stderr 的那批提示（声明里的 env 丢了、profile 漂移、声明
/// `enabled = false` 等），**恒为数组**、可为空 —— 必须让用户看见：这些事实只有 Server
/// 知道，客户端既不复述也不再判断一遍。
///
/// `Serialize` 与 [`CtlInstance`] 同一个用途：Desktop 的 `ctl_restart_instance` 直接把它
/// 回给前端（WASM 侧反序列化回同一份形状），字段名只留这一处。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartOutcome {
    /// 重启**之后**的状态行（响应里已经是新的 pid / state）。
    pub instance: CtlInstance,
    /// Server 是否真的重启了它。
    pub restarted: bool,
    /// 重启发出的提示；可能为空。
    pub notes: Vec<String>,
}

/// 解析 `status` 的**一行**响应：整行就是一个 [`CtlInstance`]（外加 `ok`）。
///
/// 不为它另写一套字段：Server 的 `status_row` 与 `list` 的行**是同一份**
/// （字段集钉在 `fixtures/ctl_contract.json`），复制一遍只会让两处各自漂移。
pub fn parse_status_response(line: &str) -> Result<CtlInstance, CtlError> {
    instance_from_value(parse_ok_body(line)?)
}

/// 解析 `restart` 的**一行**响应。失败分类口径与 [`parse_list_response`] 相同
/// （`instance_not_found` / `restart_budget_exceeded` / `port_in_use` … 都按 `code`）。
pub fn parse_restart_response(line: &str) -> Result<RestartOutcome, CtlError> {
    let val = parse_ok_body(line)?;
    let restarted = val
        .get("restarted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let notes = notes_from(&val);
    let instance = instance_from_value(val)?;
    Ok(RestartOutcome {
        instance,
        restarted,
        notes,
    })
}

/// 一个扁平对象 → [`CtlInstance`]；结构不符归 `Malformed`（不是传输故障）。
fn instance_from_value(val: Value) -> Result<CtlInstance, CtlError> {
    serde_json::from_value(val).map_err(|e| CtlError::malformed(format!("实例结构不符：{e}")))
}

/// 取 `notes` 数组里的字符串。
///
/// 缺省 / 非数组 / 非字符串项都跳过：契约说它恒为数组，但不该为一个值让整条成功响应
/// 变成错误 —— 那些提示是附加信息，丢了比整条失败好。
fn notes_from(val: &Value) -> Vec<String> {
    val.get("notes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 轮询 `list` 时对**某一条实例**的判读：此刻能不能据此新增一条 lane。
///
/// 这层判读刻意放在纯逻辑里：`add` 之后「什么时候算起来了」是多端共用的契约
/// （只有 `observed == ready` 且带 `base_url` 才算），不该在 worker 的轮询循环里
/// 现写一遍 —— 那种写法既测不了，也会与 `/ctl` 的显示口径漂移。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneReadiness {
    /// 已就绪且带 `base_url`：可以连。
    Ready(String),
    /// 还在起 / 缺 URL / 记录与实际暂时不符：继续等。
    Pending,
    /// 起不来或不该连（`failed` / `stopped` / `held`）：再等也不会好。
    Failed(String),
    /// 声明里没有这条（`add` 之后极短窗口内、或名字写错）。
    Missing,
}

/// 在 `list` 结果里判读某条实例就绪与否。
///
/// `stopped` / `held` 归 `Failed` 而不是 `Pending`：它们不是中间态，
/// 继续轮询只会把「实例被停下」拖成 20 秒的超时，掩盖真正的结论。
pub fn lane_readiness(items: &[CtlInstance], name: &str) -> LaneReadiness {
    let Some(item) = items.iter().find(|i| i.name == name) else {
        return LaneReadiness::Missing;
    };
    // 声明被关掉是**事实**，不该靠 `observed` 去猜（issue #373）：Server 的 `disable`
    // 会停掉实例、状态文件随之消失，于是 `list` 对「有声明、无状态记录」只回
    // `observed = "starting"`。先判 `enabled` 再判 `observed`，否则 `/lane attach` 一个
    // 刚 close 的实例会说「还没就绪」，`/lane new` 撞上同名已关闭的声明要白等满 20 s。
    //
    // 优先于 `observed == ready`：两者冲突说明状态记录是陈旧的，而连到一个即将被停掉的
    // 实例比多问一句更糟。`enabled` 缺失（旧版 ctl 不返回该字段）时不进这一支，
    // 判读与加这个短路之前逐字一致。
    if item.enabled == Some(false) {
        return LaneReadiness::Failed(closed_note(name));
    }
    match item.observed_state() {
        CtlObserved::Ready => {
            match item
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
            {
                // 就绪却不给 URL：再等一轮，绝不按端口猜一个地址出来。
                None => LaneReadiness::Pending,
                Some(url) => LaneReadiness::Ready(url.to_string()),
            }
        }
        CtlObserved::Failed => LaneReadiness::Failed(reason_or(
            item,
            "实例启动失败（`/ctl` 可看详情；常见原因是工作区越界或端口冲突）",
        )),
        CtlObserved::Stopped => LaneReadiness::Failed(reason_or(
            item,
            "实例已停止（`crabmatectl up` 或 `enable` 后再连）",
        )),
        CtlObserved::Held => LaneReadiness::Failed(reason_or(
            item,
            "声明被设为 enabled=false（已摘下的 lane）：`enable` 之后再连",
        )),
        // `starting` / `stale` / 无 observed / 认不出的观测态：此刻连不上，但也还没定论。
        _ => LaneReadiness::Pending,
    }
}

/// 声明被 close 掉时的原因文案：`/lane attach` 与 `/lane new` 两条入口的动作相同
/// （先 `/lane open` 再连，或者换一个名字），所以一条文案两用。
///
/// 主句用控制面的 op 名（`enable`），TUI 命令名放括注：本 crate 是共享纯逻辑，不该把
/// 某个壳的命令名当成唯一说法（Desktop 将来复用时那句话就不成立了）。
fn closed_note(name: &str) -> String {
    format!(
        "实例 {name} 的声明已关闭（enabled = false）：先 `/lane open {name}`（控制面是 `enable`）再连，或换一个名字"
    )
}

/// 失败原因优先用 Server 的 `last_error`，没有才回落到本地文案。
fn reason_or(item: &CtlInstance, fallback: &str) -> String {
    item.last_error
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map_or_else(|| fallback.to_string(), str::to_string)
}

#[cfg(test)]
#[path = "ctl_tests.rs"]
mod tests;
