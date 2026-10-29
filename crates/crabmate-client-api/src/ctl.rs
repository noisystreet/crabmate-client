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

use serde::Deserialize;
use serde_json::{Value, json};

use crate::messages::top_level_error_code;

/// `op` 取值（契约，不要改写大小写）。
pub const OP_ADD: &str = "add";
/// 见 [`OP_ADD`]。
pub const OP_REMOVE: &str = "remove";
/// 见 [`OP_ADD`]。
pub const OP_LIST: &str = "list";

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
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct CtlInstance {
    /// 实例名（`instances.toml` 的键）。
    pub name: String,
    /// 落盘态（四态）。**不要**用它判可连性，见 [`CtlObserved`]。
    #[serde(default)]
    pub state: Option<String>,
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
    /// 响应体不是合法 JSON / 结构对不上（**客户端侧**判定，Server 不会发这个码）。
    Malformed,
    /// 认不出的 `code`（Server 将来新增）：原样带上，不折进上面任何一支。
    Other,
}

impl CtlErrorKind {
    /// 按线上 `code` 分类。
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code {
            "bad_request" => Self::BadRequest,
            "bad_workspace" => Self::BadWorkspace,
            "socket_in_use" => Self::SocketInUse,
            "supervisor_gone" => Self::SupervisorGone,
            "empty_request" => Self::EmptyRequest,
            "handle_timeout" => Self::HandleTimeout,
            _ => Self::Other,
        }
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
    let val: Value = serde_json::from_str(line.trim())
        .map_err(|e| CtlError::malformed(format!("响应不是 JSON：{e}")))?;
    let ok = val.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return Err(error_from_value(&val));
    }
    // `instances` 缺省按空列表（Server 成功响应总会带，但少一次失败更好）。
    let items = val
        .get("instances")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(items)
        .map_err(|e| CtlError::malformed(format!("instances 结构不符：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_request_omits_the_default_flag() {
        assert_eq!(
            CtlRequest::List {
                include_unmanaged: false
            }
            .to_line(),
            r#"{"op":"list"}"#
        );
        assert_eq!(
            CtlRequest::List {
                include_unmanaged: true
            }
            .to_line(),
            r#"{"include_unmanaged":true,"op":"list"}"#
        );
    }

    /// `add` 的字段集必须与 Server 的白名单对齐：多一个键就会被 `bad_request` 拒。
    #[test]
    fn add_request_carries_only_the_allowed_fields() {
        let line = CtlRequest::Add {
            name: "proj-a".to_string(),
            workspace: "/home/u/proj-a".to_string(),
            profile: None,
            port: None,
            enabled: true,
        }
        .to_line();
        assert_eq!(
            line,
            r#"{"name":"proj-a","op":"add","workspace":"/home/u/proj-a"}"#
        );
        // `enabled` 默认 true 时省略，false 时必须写出（否则语义丢失）。
        let disabled = CtlRequest::Add {
            name: "proj-a".to_string(),
            workspace: "/w".to_string(),
            profile: Some("heavy".to_string()),
            port: Some(8721),
            enabled: false,
        }
        .to_line();
        assert!(disabled.contains(r#""enabled":false"#), "{disabled}");
        assert!(disabled.contains(r#""profile":"heavy""#), "{disabled}");
        assert!(disabled.contains(r#""port":8721"#), "{disabled}");
        assert!(!disabled.contains("config"), "绝不夹带 config：{disabled}");
    }

    #[test]
    fn remove_request_carries_op_and_name() {
        assert_eq!(
            CtlRequest::Remove {
                name: "proj-a".to_string()
            }
            .to_line(),
            r#"{"name":"proj-a","op":"remove"}"#
        );
    }

    /// 可连性只认 `observed`，且只有 `ready` 为真 —— 这是本模块存在的首要理由。
    #[test]
    fn only_ready_is_connectable() {
        assert!(CtlObserved::parse(Some("ready")).can_connect());
        for other in ["starting", "stale", "stopped", "failed", "held", "???"] {
            assert!(
                !CtlObserved::parse(Some(other)).can_connect(),
                "{other} 不该被判为可连"
            );
        }
        assert!(!CtlObserved::parse(None).can_connect());
        assert!(!CtlObserved::parse(Some("   ")).can_connect());
    }

    /// `stale`：记录说在跑、实际连不上。看 `state` 会误判成可连，看 `observed` 才拦得住。
    #[test]
    fn stale_record_is_not_connectable_even_when_state_looks_ready() {
        let inst = CtlInstance {
            name: "proj-a".to_string(),
            state: Some("ready".to_string()),
            observed: Some("stale".to_string()),
            ..CtlInstance::default()
        };
        assert!(!inst.can_connect());
        assert_eq!(inst.observed_state(), CtlObserved::Stale);
    }

    #[test]
    fn unknown_observed_is_kept_verbatim() {
        assert_eq!(
            CtlObserved::parse(Some("draining")),
            CtlObserved::Unknown("draining".to_string())
        );
    }

    #[test]
    fn list_response_parses_instances_and_tolerates_new_fields() {
        let line = r#"{"ok":true,"instances":[
            {"name":"proj-a","state":"running","observed":"ready",
             "base_url":"http://127.0.0.1:8721","port":8721,"pid":4242,
             "last_error":null,"workspace":"/home/u/proj-a","profile":null,
             "enabled":true,"declared":true,"managed_by":null,"brand_new_field":1}
        ]}"#;
        let items = parse_list_response(line).expect("应能解析");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "proj-a");
        assert_eq!(items[0].base_url.as_deref(), Some("http://127.0.0.1:8721"));
        assert_eq!(items[0].port, Some(8721));
        assert!(items[0].can_connect());
    }

    #[test]
    fn list_response_without_instances_is_empty() {
        assert!(parse_list_response(r#"{"ok":true}"#).unwrap().is_empty());
    }

    #[test]
    fn error_response_is_classified_by_code_not_by_text() {
        let err = parse_list_response(
            r#"{"ok":false,"code":"socket_in_use","error":"控制面已在使用：/run/u/c.sock"}"#,
        )
        .unwrap_err();
        assert_eq!(err.kind, CtlErrorKind::SocketInUse);
        assert_eq!(err.code.as_deref(), Some("socket_in_use"));
        // 文案原样透传，但分类绝不依赖它。
        assert!(err.message.contains("控制面已在使用"));
    }

    #[test]
    fn unknown_code_does_not_fall_into_a_known_bucket() {
        let err =
            parse_list_response(r#"{"ok":false,"code":"brand_new","error":"x"}"#).unwrap_err();
        assert_eq!(err.kind, CtlErrorKind::Other);
        assert_eq!(err.code.as_deref(), Some("brand_new"));
    }

    /// 连 `code` 都没有的错误体：分类为 `Other`，不猜成某个已知原因。
    #[test]
    fn error_without_code_is_other() {
        let err = parse_list_response(r#"{"ok":false,"error":"boom"}"#).unwrap_err();
        assert_eq!(err.kind, CtlErrorKind::Other);
        assert_eq!(err.code, None);
        assert_eq!(err.message, "boom");
    }

    #[test]
    fn non_json_response_is_malformed() {
        let err = parse_list_response("not json at all").unwrap_err();
        assert_eq!(err.kind, CtlErrorKind::Malformed);
        assert_eq!(err.code, None);
    }
}
