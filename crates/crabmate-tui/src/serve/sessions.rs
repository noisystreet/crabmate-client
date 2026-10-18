//! `GET /user-data/workspaces/current/sessions`：与 WASM 同源的 Web 会话列表，
//! 以及把 TUI 新建/绑定的会话写回同一份索引（`GET` → 改 → 全量 `PUT`）。
//!
//! 服务端对这一数组做**透传存储**（行是 Web 的全量 `ChatSession`，含 `messages` 等
//! 瘦行未列出的字段），因此写入必须保留未改动行的原样 JSON，不能按瘦行重建。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

pub use crabmate_client_api::SessionListRow;
/// 标题是否仍是「还没起名」的默认值。判定已下沉到共享层（Web `i18n::is_default_session_title`
/// 用同一份）；空串**不算**默认标题。这里 re-export 保住 `serve::is_default_session_title`。
pub use crabmate_client_api::session_model::is_default_session_title;

use crabmate_client_api::paths;
use crabmate_client_api::session_model::{
    ChatSession, session_is_blank_chat, session_is_disposable_blank,
    title_from_user_prompt as shared_title_from_user_prompt,
};
use crabmate_client_api::session_row_conversation_id_for_resume;

use crate::serve::client::ServeClient;
use crate::serve::error::TermError;

#[derive(Debug, Clone)]
pub struct WebSessionsList {
    pub sessions: Vec<SessionListRow>,
    pub active_session_id: Option<String>,
    /// 服务端原样的 `sessions` 数组（缺失 / 非数组时为 `Value::Null`）。
    /// 写入路径必须基于它做局部修改，否则会把 Web 端的 `messages` 等字段清空。
    pub raw_sessions: Value,
}

#[derive(Debug, Deserialize)]
struct WebSessionsDto {
    #[serde(default)]
    sessions: Value,
    #[serde(default)]
    active_session_id: Option<String>,
}

/// 拉取当前工作区绑定的 Web 会话快照。
pub async fn fetch_web_sessions(client: &ServeClient) -> Result<WebSessionsList, TermError> {
    let dto: WebSessionsDto = client.get_json(paths::USER_DATA_WORKSPACE_SESSIONS).await?;
    Ok(WebSessionsList {
        sessions: SessionListRow::parse_rows(&dto.sessions),
        active_session_id: dto.active_session_id,
        raw_sessions: dto.sessions,
    })
}

/// 续聊用的服务端 `conversation_id`；仅本地 Web id 时返回 `None`（不可拿去 `/conv use`）。
#[must_use]
pub fn conversation_id_for_resume(row: &SessionListRow) -> Option<&str> {
    session_row_conversation_id_for_resume(row)
}

// ── 写入：会话索引的本地变更（TUI 新建 / 首轮绑定）────────────────────────

/// 会话索引的一次变更；执行方式是 `GET` 最新索引 → 应用 → 全量 `PUT`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIndexOp {
    /// 新建一条空白会话并设为 active；列表里已有空白会话时**复用**它
    /// （对齐 Web `prefer_or_create_empty_session`）。
    NewBlank {
        /// 侧栏标题：Web 的默认标题（`i18n::default_session_title`）。
        /// 首轮发送后会按 Web 规则换成消息摘要（见 [`SessionIndexOp::SetTitle`]）。
        title: String,
    },
    /// 把服务端 `conversation_id` 写回指定本地会话行（新建的会话首轮结束后调用）。
    BindServer {
        /// 本地会话行 id（`sessions[].id`）。
        local_id: String,
        conversation_id: String,
    },
    /// 切走时回收空白会话行（对齐 Web `reclaim_disposable_blank_session`）。
    RemoveBlank {
        /// 本地会话行 id（`sessions[].id`）。
        local_id: String,
        /// 切过去的那一行的 id：被回收的行若正是 active，active 要改指向它；
        /// 为 `None` 时不回收 active 行，否则服务端 active 会指向已删除的行。
        next_active: Option<String>,
    },
    /// 把默认标题换成首条用户消息摘要（对齐 Web：仅当标题仍是默认值时才改）。
    SetTitle {
        /// 本地会话行 id（`sessions[].id`）。
        local_id: String,
        /// 新标题（已由 [`title_from_user_prompt`] 规整）。
        title: String,
    },
    /// 退出收尾：一次 `GET → 应用 → PUT` 同时补绑定与草稿。
    ///
    /// **为什么合并成一次写入**：每次 op 都是「读全量索引 → 改 → 写全量」，两次独立
    /// 的 PUT 之间若被另一端的写入插进来，就会互相覆盖——典型是 worker 那条 `BindServer`
    /// 的 body 取自更早的 GET，落地时会把刚写好的草稿拍回去。两个字段都可选；都为
    /// `None` 时调用方应直接跳过（不发请求）。
    FlushExit {
        /// 本地会话行 id（`sessions[].id`）。
        local_id: String,
        /// 待补的 `server_conversation_id`（`None` = 已绑定或无需绑定）。
        conversation_id: Option<String>,
        /// 待写回的草稿全文（`None` = 没草稿；空白草稿由调用方判掉）。
        draft: Option<String>,
    },
}

/// 应用变更所需的外部输入（时间戳与新行 id 由调用方注入，便于单测固定）。
#[derive(Debug, Clone)]
pub struct IndexStamp {
    pub now_ms: i64,
    pub new_local_id: String,
}

/// 变更结果：回传要 `PUT` 的全量数组、新的 active id，以及本次命中的本地行 id。
#[derive(Debug, Clone)]
pub struct AppliedIndex {
    pub sessions: Value,
    pub active_session_id: Option<String>,
    pub local_id: String,
    /// `NewBlank` 是复用了已有空白行（`true`）还是新插入一行（`false`）。
    pub reused_blank: bool,
}

/// 写入成功后回传 UI 的最小信息（不带整份索引）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIndexApplied {
    /// 本次命中的本地会话行 id。
    pub local_id: String,
    /// `NewBlank` 是否复用了已有空白行。
    pub reused_blank: bool,
}

impl IndexStamp {
    /// 当前时刻 + 一个新行 id。
    #[must_use]
    pub fn now() -> Self {
        Self {
            now_ms: now_ms(),
            new_local_id: new_local_session_id(SystemTime::now()),
        }
    }
}

/// 新会话行 id：`s_<毫秒>_<纳秒>`（Web 端 `s_<毫秒>_<随机数>` 同族）。
///
/// 格式由共享层 `new_session_id` 统一，熵源在这里注入（亚秒纳秒，同一毫秒内不撞）。
#[must_use]
pub fn new_local_session_id(now: SystemTime) -> String {
    let d = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    crabmate_client_api::new_session_id(d.as_millis() as i64, d.subsec_nanos())
}

/// 把变更应用到（服务端原样返回的）索引数组上。
///
/// 除命中的行外，其余行逐字段原样保留——服务端透传存储，重建会丢 Web 端会话正文。
pub fn apply_index_op(
    raw: &Value,
    active: Option<&str>,
    op: &SessionIndexOp,
    stamp: &IndexStamp,
) -> Result<AppliedIndex, TermError> {
    let mut rows: Vec<Value> = raw.as_array().cloned().unwrap_or_default();
    match op {
        SessionIndexOp::NewBlank { title } => {
            // 已有空白会话就复用：与 Web 一致，别在侧栏堆「0 消息」的空会话。
            let reuse = rows
                .iter()
                .find(|r| row_is_blank_chat(r))
                .and_then(|r| row_id(r))
                .map(str::to_string);
            if let Some(id) = reuse {
                return Ok(AppliedIndex {
                    sessions: Value::Array(rows),
                    active_session_id: Some(id.clone()),
                    local_id: id,
                    reused_blank: true,
                });
            }
            rows.insert(0, blank_row(&stamp.new_local_id, title, stamp.now_ms));
            Ok(AppliedIndex {
                sessions: Value::Array(rows),
                active_session_id: Some(stamp.new_local_id.clone()),
                local_id: stamp.new_local_id.clone(),
                reused_blank: false,
            })
        }
        SessionIndexOp::BindServer {
            local_id,
            conversation_id,
        } => {
            let row = rows
                .iter_mut()
                .find(|r| row_id(r) == Some(local_id.as_str()))
                .ok_or_else(|| {
                    TermError::Message(format!("会话行已不存在：{local_id}（可能被其他端删除）"))
                })?;
            bind_row(row, conversation_id, stamp.now_ms);
            Ok(AppliedIndex {
                sessions: Value::Array(rows),
                active_session_id: active.map(str::to_string),
                local_id: local_id.clone(),
                reused_blank: false,
            })
        }
        SessionIndexOp::RemoveBlank {
            local_id,
            next_active,
        } => Ok(remove_blank_row(
            &mut rows,
            active,
            local_id,
            next_active.as_deref(),
        )),
        SessionIndexOp::SetTitle { local_id, title } => {
            if let Some(row) = rows
                .iter_mut()
                .find(|r| row_id(r) == Some(local_id.as_str()))
            {
                set_default_title(row, title, stamp.now_ms);
            }
            Ok(AppliedIndex {
                sessions: Value::Array(rows),
                active_session_id: active.map(str::to_string),
                local_id: local_id.clone(),
                reused_blank: false,
            })
        }
        SessionIndexOp::FlushExit {
            local_id,
            conversation_id,
            draft,
        } => {
            // 行被别的端删掉时绑定无处可落，如实报错（退出路径上由调用方提示）。
            // 这比 `SetTitle` 的「静默跳过」更严，因为绑定丢失是用户可感知的后果。
            let row = rows
                .iter_mut()
                .find(|r| row_id(r) == Some(local_id.as_str()))
                .ok_or_else(|| {
                    TermError::Message(format!("会话行已不存在：{local_id}（可能被其他端删除）"))
                })?;
            if let Some(cid) = conversation_id {
                bind_row(row, cid, stamp.now_ms);
            }
            if let Some(d) = draft {
                set_draft(row, d, stamp.now_ms);
            }
            Ok(AppliedIndex {
                sessions: Value::Array(rows),
                active_session_id: active.map(str::to_string),
                local_id: local_id.clone(),
                reused_blank: false,
            })
        }
    }
}

/// `PUT` 体：`active_session_id` 为 `None` 时不写（与 Web 端 `skip_serializing_if` 同义）。
///
/// 注意这是**整体替换**，只带这两个顶层字段：服务端若在同一对象上另有字段（版本号之类），
/// 这次 `PUT` 会把它们抹掉。目前 `WebSessionsDto` 只认这两个，所以暂时安全——服务端在该
/// 对象上加字段时必须同步这里。
#[must_use]
pub fn put_sessions_body(applied: &AppliedIndex) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("sessions".into(), applied.sessions.clone());
    if let Some(active) = applied.active_session_id.as_deref() {
        body.insert(
            "active_session_id".into(),
            Value::String(active.to_string()),
        );
    }
    Value::Object(body)
}

/// `GET` 最新索引 → 应用变更 → 全量 `PUT`；成功返回命中的本地行 id。
pub async fn apply_web_session_op(
    client: &ServeClient,
    op: &SessionIndexOp,
) -> Result<SessionIndexApplied, TermError> {
    let list = fetch_web_sessions(client).await?;
    let applied = apply_index_op(
        &list.raw_sessions,
        list.active_session_id.as_deref(),
        op,
        &IndexStamp::now(),
    )?;
    client
        .put_json_no_content(
            paths::USER_DATA_WORKSPACE_SESSIONS,
            &put_sessions_body(&applied),
        )
        .await?;
    Ok(SessionIndexApplied {
        local_id: applied.local_id,
        reused_blank: applied.reused_blank,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_id(row: &Value) -> Option<&str> {
    row.get("id")?.as_str()
}

/// 空白会话行：只写 Web `ChatSession` 的必需 / 常用字段（其余由 Web 侧 serde 缺省补齐）。
/// 不写 `workspace_root`：工作区分桶由 Web 侧规范化路径决定，TUI 不自造绑定。
fn blank_row(id: &str, title: &str, now_ms: i64) -> Value {
    json!({
        "id": id,
        "title": title,
        "draft": "",
        "messages": [],
        "updated_at": now_ms,
        "pinned": false,
        "starred": false,
    })
}

/// 绑定服务端会话：只动 `server_conversation_id` 与 `updated_at`，其余字段原样保留。
fn bind_row(row: &mut Value, conversation_id: &str, now_ms: i64) {
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    obj.insert(
        "server_conversation_id".into(),
        Value::String(conversation_id.to_string()),
    );
    obj.insert("updated_at".into(), Value::from(now_ms));
}

/// Web 侧「默认会话标题」（`i18n::default_session_title(ZhHans)`）：TUI 界面文案固定中文，
/// 所以建行就写这个值——Web 端展示时会按它自己的界面语言再折算。
pub const DEFAULT_SESSION_TITLE: &str = "新会话";

/// 首条用户消息 → 侧栏标题（对齐 Web `session_ops::title_from_user_prompt`）：
/// 压平换行、折叠连续空白，超过 48 字符取前 47 加 `…`；全空白退回默认标题。
///
/// 规整算法已下沉到 `crabmate-client-api`（Web 用同一份），这里只绑定 TUI 的中文默认标题。
#[must_use]
pub fn title_from_user_prompt(text: &str) -> String {
    shared_title_from_user_prompt(text, DEFAULT_SESSION_TITLE)
}

/// 改默认标题：只有当前标题**明确等于中 / 英默认标题**时才写
/// （对齐 Web `is_default_session_title` 的守卫），别端起过名或用户改过的标题一律保留。
///
/// 标题缺失 / 空串按 Web 语义**不算**默认标题（Web `ChatSession.title` 缺省为空串，
/// `is_default_session_title("")` 是 `false`），所以这两种情况下不动它。
fn set_default_title(row: &mut Value, title: &str, now_ms: i64) {
    if !row_trimmed(row, "title").is_some_and(is_default_session_title) {
        return;
    }
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    obj.insert("title".into(), Value::String(title.to_string()));
    obj.insert("updated_at".into(), Value::from(now_ms));
}

/// 写草稿：只动 `draft` 与 `updated_at`，其余字段（含 Web 写的 `messages`）原样保留。
fn set_draft(row: &mut Value, draft: &str, now_ms: i64) {
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    obj.insert("draft".into(), Value::String(draft.to_string()));
    obj.insert("updated_at".into(), Value::from(now_ms));
}

/// 把索引行解析成 Web 的 `ChatSession`；解析失败返回 `None`。
///
/// 服务端存的就是 Web 侧完整 `ChatSession`，所以判定只做**读**——写回仍走原始 JSON
/// （`bind_row` / `set_default_title` 只动目标字段，不按类型重建整行，免得抹掉别的端数据）。
///
/// 解析失败（字段类型不对、`messages` 不是数组……）由调用方按「不是空白行」处理：宁可少认
/// 一条空行去复用 / 回收，也不误动别的端写过的会话。代价是**任何一条消息**的字段类型不符，
/// 该行就再也不被认作空白行——`/conv new` 会改为新建一行（不会误删，但可能堆积空会话）。
fn row_as_session(row: &Value) -> Option<ChatSession> {
    ChatSession::deserialize(row).ok()
}

/// `messages` 是否「可能为空」：只做长度快筛，**不解析**元素。
///
/// 仅用于跳过明显的非空行，真正的判定仍走共享层（这里不重复任何会话规则）。
/// 非数组时返回 `false`——那种行本来也解析不出 `ChatSession`，结论一致。
///
/// 没有这一层，`apply_index_op` 的 `NewBlank` 会逐行完整反序列化（含全部 `messages`）只为
/// 求一个 `messages.is_empty()`；而列表里绝大多数行都有正文，那等于全表解析。
fn messages_look_empty(row: &Value) -> bool {
    match row.get("messages") {
        None => true,
        Some(Value::Array(list)) => list.is_empty(),
        Some(_) => false,
    }
}

/// 空白会话行（共享层 `session_is_blank_chat`）：无正文且未绑定 `conversation_id`。
fn row_is_blank_chat(row: &Value) -> bool {
    messages_look_empty(row) && row_as_session(row).is_some_and(|s| session_is_blank_chat(&s))
}

/// 可丢弃的空白行（共享层 `session_is_disposable_blank`）：空白 + 无草稿 + 未置顶 / 收藏。
fn row_is_disposable_blank(row: &Value) -> bool {
    messages_look_empty(row) && row_as_session(row).is_some_and(|s| session_is_disposable_blank(&s))
}

/// 回收一条空白行（对齐 Web `reclaim_disposable_blank_session`）。
///
/// 只有「确实可丢弃」的行才删：别的端写过的正文 / 草稿 / 收藏一律不动。
/// 被删行正是 `active` 时必须同时把 active 指到 `next_active`，否则服务端 active 会指向
/// 一条已不存在的行；`next_active` 缺失时宁可不删。
///
/// `next_active` 是调用方按**本地快照**反查出来的 id，而这里操作的是刚 `GET` 回来的
/// 最新数组——两者可能已被别的端改动，所以落点必须按这里的数据再校验一次。
fn remove_blank_row(
    rows: &mut Vec<Value>,
    active: Option<&str>,
    local_id: &str,
    next_active: Option<&str>,
) -> AppliedIndex {
    let Some(idx) = rows.iter().position(|r| row_id(r) == Some(local_id)) else {
        // 已被其他端删掉：回收是幂等动作，不算失败。
        return applied_without_removal(rows, active, local_id);
    };
    if !row_is_disposable_blank(&rows[idx]) {
        return applied_without_removal(rows, active, local_id);
    }
    if active == Some(local_id) {
        // 落点必须真的在数组里，且不能是被删的这一行本身。
        let next =
            next_active.filter(|id| *id != local_id && rows.iter().any(|r| row_id(r) == Some(*id)));
        let Some(next) = next else {
            return applied_without_removal(rows, active, local_id);
        };
        rows.remove(idx);
        return AppliedIndex {
            sessions: Value::Array(std::mem::take(rows)),
            active_session_id: Some(next.to_string()),
            local_id: local_id.to_string(),
            reused_blank: false,
        };
    }
    rows.remove(idx);
    applied_without_removal(rows, active, local_id)
}

/// 未改动任何行时的返回（行数不变，active 照旧）。
fn applied_without_removal(
    rows: &mut Vec<Value>,
    active: Option<&str>,
    local_id: &str,
) -> AppliedIndex {
    AppliedIndex {
        sessions: Value::Array(std::mem::take(rows)),
        active_session_id: active.map(str::to_string),
        local_id: local_id.to_string(),
        reused_blank: false,
    }
}

/// 取字符串字段并 trim；缺失 / 非字符串 / 全空白都算 `None`。
fn row_trimmed<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

// 测试拆到独立文件：避免 lizard 函数体合并跨度超 fn-nloc 门禁（同 md / render 惯例）。
#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
