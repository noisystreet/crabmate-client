//! 会话 / 消息的**客户端本地**数据模型与派生判定（多端共用）。
//!
//! 这里的类型描述的是 Web / Desktop 与 TUI 各自的本地会话列表（`ChatSession`）；
//! 服务端把它当 JSON 透传存储，因此**不是**契约类型，不受 `contract_pin.md` 约束。
//!
//! 从 `frontend/src/storage.rs` 与 `frontend/src/session_workspace_partition.rs` 下沉而来：
//! 两端此前各写一份「空白会话 / 可丢弃空白行 / 复用或新建」判定，已经漂移过一次
//! （TUI 曾把缺失标题当作默认标题，而 Web 的 `is_default_session_title("")` 返回 false）。
//!
//! 时间与随机源一律由调用方注入（Web 用 `js_sys`，TUI 用 `SystemTime`），
//! 这样本模块保持纯函数、可单测，也不会把 `web-sys` 拖进共享层。

use serde::{Deserialize, Serialize};

/// 新建会话默认标题（**存储用**，与语言无关）；界面展示各自本地化
/// （Web 侧见 `frontend/src/i18n::session_title_for_display`）。
pub const DEFAULT_CHAT_SESSION_TITLE: &str = "New chat";
/// 旧会话未记录布局版本时使用的兼容版本。
pub const LEGACY_LAYOUT_SCHEMA_VERSION: u32 = 1;
/// 当前 Web 流式投影布局版本：closed commentary 按稳定行分别持久化。
pub const CURRENT_LAYOUT_SCHEMA_VERSION: u32 = 2;
/// v2 closed commentary 的持久化消息 ID 前缀。
pub const V2_COMMENTARY_ROW_ID_PREFIX: &str = "turn-commentary-";
/// v2 工具前思维链（reasoning）的持久化消息 ID 前缀。
pub const V2_THINK_ROW_ID_PREFIX: &str = "turn-think-";
/// v2 终答的持久化消息 ID。
pub const V2_FINAL_ANSWER_ROW_ID: &str = "turn-final-answer";
/// `StoredMessageState::TimelineUiJson` 内嵌 JSON 的判别键 `k`。
pub const TIMELINE_UI_STATE_KEY: &str = "cm_tl";

const fn default_layout_schema_version() -> u32 {
    LEGACY_LAYOUT_SCHEMA_VERSION
}

/// 本地会话消息 UI / 流式协议状态（原 `Option<String>`，现枚举化；JSON 仍存为同一字符串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredMessageState {
    Loading,
    Error,
    /// 同一模型轮次在 `tool_calls` 之前流出的正文旁注（不进主气泡/导出）。
    CommentaryBeforeTools,
    /// 侧栏时间线：`k` 为 [`TIMELINE_UI_STATE_KEY`] 的 JSON。
    TimelineUiJson(String),
    /// 未能归入已知变体的字符串（兼容往返）。
    Opaque(String),
}

impl StoredMessageState {
    /// 从落盘字符串还原；未知值收进 [`Self::Opaque`]，保证往返不丢信息。
    #[must_use]
    pub fn from_wire(s: String) -> Self {
        match s.as_str() {
            "loading" => return Self::Loading,
            "error" => return Self::Error,
            "commentary_before_tools" => return Self::CommentaryBeforeTools,
            _ => {}
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
            && v.get("k").and_then(|x| x.as_str()) == Some(TIMELINE_UI_STATE_KEY)
        {
            return Self::TimelineUiJson(s);
        }
        Self::Opaque(s)
    }

    /// 落盘字符串；[`Self::Opaque`] 原样回写，避免把别人的状态值改掉。
    #[must_use]
    pub fn to_wire(&self) -> String {
        match self {
            Self::Loading => "loading".to_string(),
            Self::Error => "error".to_string(),
            Self::CommentaryBeforeTools => "commentary_before_tools".to_string(),
            Self::TimelineUiJson(s) | Self::Opaque(s) => s.clone(),
        }
    }

    #[must_use]
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// 若非空则交给调用方做时间线 JSON 解析（本模块只负责取串，不解析）。
    #[must_use]
    pub fn as_timeline_parse_candidate(&self) -> Option<&str> {
        match self {
            Self::TimelineUiJson(s) | Self::Opaque(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// 服务端快照合并时：本地时间线旁注是否应保留。
    #[must_use]
    pub fn is_local_timeline_snapshot_row(&self) -> bool {
        match self {
            Self::TimelineUiJson(_) => true,
            Self::Opaque(s) => s.contains(TIMELINE_UI_STATE_KEY),
            _ => false,
        }
    }
}

/// `Option<StoredMessageState>` 的 serde 桥：落盘始终是字符串，缺失即 `None`。
mod serde_opt_stored_message_state {
    use super::StoredMessageState;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        value: &Option<StoredMessageState>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            None => Option::<String>::None.serialize(serializer),
            Some(st) => Some(st.to_wire()).serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<StoredMessageState>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        Ok(opt.map(StoredMessageState::from_wire))
    }
}

/// 一条本地会话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub role: String,
    #[serde(default)]
    pub text: String,
    /// 助手思维链（与 `text` 终答分隔）；旧数据缺省为空。
    #[serde(default)]
    pub reasoning_text: String,
    /// 用户消息附带的图片（`/uploads/...`）；旧数据缺省为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_opt_stored_message_state"
    )]
    pub state: Option<StoredMessageState>,
    #[serde(default)]
    pub is_tool: bool,
    /// 与 SSE `tool_call` / `tool_result` 的 `tool_call_id` 对齐；旧数据缺省为无（按 FIFO 配对）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// OpenAI function `name`（蛇形）；供工具气泡图标等 UI，**不**拼进可复制正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// 消息创建时间（毫秒）；旧数据缺省为 0，UI 不显示时钟点。
    #[serde(default)]
    pub created_at: i64,
}

/// 一条本地会话（侧栏一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    /// Web 消息行投影版本。旧缓存缺省为 v1；新建会话写 v2。
    #[serde(default = "default_layout_schema_version")]
    pub layout_schema_version: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub draft: String,
    #[serde(default)]
    pub messages: Vec<StoredMessage>,
    /// 旧版前端未存该字段时默认为 0。
    #[serde(default)]
    pub updated_at: i64,
    /// 置顶：侧栏排序优先于收藏与普通会话。
    #[serde(default)]
    pub pinned: bool,
    /// 收藏：侧栏排序次于置顶、优于仅按时间。
    #[serde(default)]
    pub starred: bool,
    /// 与服务端 `conversation_id` 对齐；无则纯本地会话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_conversation_id: Option<String>,
    /// 最近一次已知的 `conversation_saved.revision` 或水合时的 revision。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_revision: Option<u64>,
    /// 本会话绑定的 Web 工作区根；切换到此会话时自动应用。旧数据缺省为不绑定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// 服务端过滤后消息总数（分页水合时写入）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_total: Option<u32>,
    /// 当前 `messages[0]` 在服务端过滤数组中的下标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_window_start: Option<u32>,
    /// 是否还有更早消息（`GET /conversation/messages?before_index=`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_has_older: Option<bool>,
}

impl ChatSession {
    /// 是否已有可直接复用的 v2 投影。稳定 key 优先于旧缓存中可能缺失的版本字段。
    #[must_use]
    pub fn has_v2_layout_projection(&self) -> bool {
        self.layout_schema_version >= CURRENT_LAYOUT_SCHEMA_VERSION || self.has_v2_finalized_rows()
    }

    /// 是否已经持久化至少一条可安全复用的 v2 finalized 行。
    #[must_use]
    pub fn has_v2_finalized_rows(&self) -> bool {
        self.messages.iter().any(|m| {
            m.id.starts_with(V2_COMMENTARY_ROW_ID_PREFIX)
                || m.id.starts_with(V2_THINK_ROW_ID_PREFIX)
                || m.id == V2_FINAL_ANSWER_ROW_ID
        })
    }

    /// 加载旧缓存后补齐可由稳定 projection key 明确判定的 v2 版本。
    pub fn normalize_layout_schema_version(&mut self) {
        if self.has_v2_finalized_rows() {
            self.layout_schema_version = CURRENT_LAYOUT_SCHEMA_VERSION;
        } else if self.layout_schema_version == 0 {
            self.layout_schema_version = LEGACY_LAYOUT_SCHEMA_VERSION;
        }
    }

    #[must_use]
    pub fn history_has_older_flag(&self) -> bool {
        self.history_has_older.unwrap_or(false)
    }

    /// 非空且 trim 后的 `server_conversation_id`；与消息读取端点的路径参数对齐。
    #[must_use]
    pub fn trimmed_server_conversation_id(&self) -> Option<&str> {
        crate::sessions::conversation_id_for_resume(self.server_conversation_id.as_deref())
    }
}

/// 新会话行 id：`s_<毫秒>_<熵>`。
///
/// 时间与熵由调用方注入：Web 传 `js_sys::Date::now()` 与 `Math::random()` 放大后的整数，
/// TUI 传毫秒时间戳与亚秒纳秒。两端格式一致，但**熵源不同**（Web 是随机、TUI 是单调时间），
/// 都能保证「同一毫秒内不撞」。
#[must_use]
pub fn new_session_id(now_ms: i64, entropy: u32) -> String {
    format!("s_{now_ms}_{entropy}")
}

/// 新建一条空会话行（时间与 id 由调用方注入）。
///
/// ⚠️ `layout_schema_version` 固定写 [`CURRENT_LAYOUT_SCHEMA_VERSION`]（v2），与 Web 建会话
/// 一致。**TUI 若要登记自己的行，不要直接采用本函数给出的版本字段**：Web 的
/// [`ChatSession::has_v2_layout_projection`] 对 `version >= CURRENT` 直接返回 true，
/// 据此可能跳过会话水合——而没有 v2 投影正文的空行会让 Web 误判「已有可复用投影」。
/// TUI 建行时请保持字段缺失（legacy）。
#[must_use]
pub fn new_empty_session(
    local_id: String,
    title: String,
    now_ms: i64,
    workspace_root: Option<String>,
) -> ChatSession {
    ChatSession {
        id: local_id,
        layout_schema_version: CURRENT_LAYOUT_SCHEMA_VERSION,
        title,
        draft: String::new(),
        messages: Vec::new(),
        updated_at: now_ms,
        pinned: false,
        starred: false,
        server_conversation_id: None,
        server_revision: None,
        workspace_root,
        history_total: None,
        history_window_start: None,
        history_has_older: None,
    }
}

/// 空白会话：无正文且未绑定服务端会话（Web `session_is_blank_chat`）。
#[must_use]
pub fn session_is_blank_chat(s: &ChatSession) -> bool {
    s.messages.is_empty()
        && s.server_conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .is_none()
}

/// 无正文、无草稿、无工作区绑定：冷启动可复用，且不必改写已有行
/// （Web `session_is_unbound_empty_chat`）。
#[must_use]
pub fn session_is_unbound_empty_chat(s: &ChatSession) -> bool {
    session_is_blank_chat(s)
        && s.draft.trim().is_empty()
        && s.workspace_root
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
}

/// 可自动清理的空白新会话：空白 + 无草稿 + 非置顶/收藏
/// （Web `is_disposable_blank_session`）。
///
/// 仅用于「离开该会话」的切换 / 新建路径，**勿**据此批量清理列表中其它空会话。
#[must_use]
pub fn session_is_disposable_blank(s: &ChatSession) -> bool {
    session_is_blank_chat(s) && s.draft.trim().is_empty() && !s.pinned && !s.starred
}

/// 优先复用已有空白会话，否则在列表前插入一条空会话
/// （Web `prefer_or_create_empty_session`）。
///
/// 复用时会用 `workspace_root` 覆盖该行的绑定（`None` 表示保持原样）。
///
/// `new_id` / `now_ms` 收**闭包而非值**：复用分支根本不生成新 id，提前求值等于白跑。
/// Web 侧那两个来源是 `js_sys::Date::now()` / `Math::random()`，在 host target 的单测里
/// 一调用就 panic（wasm-bindgen 导入函数只能在 wasm 上调），所以惰性是硬要求。
#[must_use]
pub fn prefer_or_create_empty_session(
    mut list: Vec<ChatSession>,
    workspace_root: Option<String>,
    title: String,
    new_id: impl FnOnce() -> String,
    now_ms: impl FnOnce() -> i64,
) -> (Vec<ChatSession>, String) {
    if let Some(pos) = list.iter().position(session_is_blank_chat) {
        let id = list[pos].id.clone();
        if workspace_root.is_some() {
            list[pos].workspace_root = workspace_root;
        }
        return (list, id);
    }
    let row = new_empty_session(new_id(), title, now_ms(), workspace_root);
    let id = row.id.clone();
    list.insert(0, row);
    (list, id)
}

/// 是否仍是「还没起名」的默认标题。
///
/// 任一语言的默认值都算（`新会话` / `New chat`，后者大小写不敏感）。
/// **空串与全空白不算**：`ChatSession.title` 缺省就是空串，若把空串当默认值，
/// 别的端从没起过名的会话会被这里改写。
#[must_use]
pub fn is_default_session_title(title: &str) -> bool {
    let t = title.trim();
    t == "新会话" || t.eq_ignore_ascii_case("new chat")
}

/// 首条用户消息 → 侧栏标题：压平换行、折叠连续空白，超过 48 字符取前 47 加 `…`。
///
/// 全空白退回 `default_title`——那是调用方的**存储用**默认值（Web 传
/// [`DEFAULT_CHAT_SESSION_TITLE`]，TUI 传自己的中文默认标题），所以由调用方给，
/// 而不是写死在这里。注意它不是界面语言下的展示标题（那是 `i18n::default_session_title`）。
#[must_use]
pub fn title_from_user_prompt(text: &str, default_title: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return default_title.to_string();
    }
    let single_line: String = t
        .chars()
        .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
        .collect();
    let collapsed = single_line.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 48;
    let n = collapsed.chars().count();
    if n <= MAX_CHARS {
        return collapsed;
    }
    let kept: String = collapsed
        .chars()
        .take(MAX_CHARS.saturating_sub(1))
        .collect();
    format!("{kept}…")
}

/// 从会话列表回收 `leaving_id` 指向的空白会话
/// （Web `reclaim_disposable_blank_session`）。
///
/// `leaving_id` 为空、行不存在、或该行不可丢弃时都不动作。
pub fn reclaim_disposable_blank_session(sessions: &mut Vec<ChatSession>, leaving_id: &str) {
    if leaving_id.is_empty() {
        return;
    }
    let disposable = sessions
        .iter()
        .find(|s| s.id == leaving_id)
        .is_some_and(session_is_disposable_blank);
    if disposable {
        sessions.retain(|s| s.id != leaving_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> ChatSession {
        new_empty_session(
            id.to_string(),
            DEFAULT_CHAT_SESSION_TITLE.to_string(),
            1_000,
            None,
        )
    }

    fn msg(id: &str) -> StoredMessage {
        StoredMessage {
            id: id.to_string(),
            role: "assistant".to_string(),
            text: String::new(),
            reasoning_text: String::new(),
            image_urls: Vec::new(),
            state: None,
            is_tool: false,
            tool_call_id: None,
            tool_name: None,
            created_at: 0,
        }
    }

    #[test]
    fn known_states_roundtrip_through_wire_strings() {
        for (wire, want) in [
            ("loading", StoredMessageState::Loading),
            ("error", StoredMessageState::Error),
            (
                "commentary_before_tools",
                StoredMessageState::CommentaryBeforeTools,
            ),
        ] {
            assert_eq!(StoredMessageState::from_wire(wire.to_string()), want);
            assert_eq!(want.to_wire(), wire);
        }
        assert!(StoredMessageState::Loading.is_loading());
        assert!(StoredMessageState::Error.is_error());
        assert!(!StoredMessageState::Loading.is_error());
    }

    #[test]
    fn unknown_state_is_preserved_as_opaque() {
        // 别的端写进来的新状态值不能被抹掉：原样往返，且不算本地时间线快照行。
        let s = StoredMessageState::from_wire("something-new".to_string());
        assert_eq!(s, StoredMessageState::Opaque("something-new".into()));
        assert_eq!(s.to_wire(), "something-new");
        assert!(!s.is_local_timeline_snapshot_row());
        assert_eq!(s.as_timeline_parse_candidate(), Some("something-new"));
    }

    #[test]
    fn timeline_ui_json_needs_the_cm_tl_key() {
        let raw = "{\"k\":\"cm_tl\",\"x\":1}";
        let tl = StoredMessageState::from_wire(raw.to_string());
        assert!(matches!(tl, StoredMessageState::TimelineUiJson(_)));
        assert!(tl.is_local_timeline_snapshot_row());
        assert_eq!(tl.as_timeline_parse_candidate(), Some(raw));

        // `k` 不是 cm_tl：按不透明串保留，但不算本地快照行。
        let other = StoredMessageState::from_wire("{\"k\":\"other\"}".to_string());
        assert!(matches!(other, StoredMessageState::Opaque(_)));
        assert!(!other.is_local_timeline_snapshot_row());
    }

    #[test]
    fn absent_state_is_not_serialized() {
        let m = msg("m1");
        let v = serde_json::to_value(&m).expect("serialize");
        assert!(v.get("state").is_none(), "None 不该落盘");
        let back: StoredMessage = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back.state, None);
    }

    #[test]
    fn present_state_serializes_as_wire_string() {
        let m = StoredMessage {
            state: Some(StoredMessageState::CommentaryBeforeTools),
            ..msg("m1")
        };
        let v = serde_json::to_value(&m).expect("serialize");
        assert_eq!(v["state"], "commentary_before_tools");
        let back: StoredMessage = serde_json::from_value(v).expect("deserialize");
        assert_eq!(back.state, Some(StoredMessageState::CommentaryBeforeTools));
    }

    #[test]
    fn blank_chat_requires_no_messages_and_no_server_id() {
        let mut s = session("s1");
        assert!(session_is_blank_chat(&s));
        assert!(session_is_unbound_empty_chat(&s));
        assert!(session_is_disposable_blank(&s));

        s.messages.push(msg("m1"));
        assert!(!session_is_blank_chat(&s), "有正文就不是空白会话");

        s.messages.clear();
        s.server_conversation_id = Some("   ".to_string());
        assert!(session_is_blank_chat(&s), "全空白的 cid 等同未绑定");

        s.server_conversation_id = Some("c1".to_string());
        assert!(!session_is_blank_chat(&s), "绑定了服务端会话就不是空白会话");
    }

    #[test]
    fn disposable_blank_is_blocked_by_draft_pin_or_star() {
        let mut s = session("s1");
        s.draft = "半句".to_string();
        assert!(!session_is_disposable_blank(&s), "有草稿不能回收");

        s.draft.clear();
        s.pinned = true;
        assert!(!session_is_disposable_blank(&s), "置顶不能回收");

        s.pinned = false;
        s.starred = true;
        assert!(!session_is_disposable_blank(&s), "收藏不能回收");

        s.starred = false;
        assert!(session_is_disposable_blank(&s));
    }

    #[test]
    fn unbound_empty_chat_requires_no_workspace_root() {
        let mut s = session("s1");
        s.workspace_root = Some("  ".to_string());
        assert!(session_is_unbound_empty_chat(&s), "全空白视为未绑定");

        s.workspace_root = Some("/w".to_string());
        assert!(!session_is_unbound_empty_chat(&s));
        assert!(session_is_blank_chat(&s), "但仍是空白会话");
    }

    #[test]
    fn prefer_reuses_blank_row_and_overrides_its_root() {
        let list = vec![session("s1"), session("s2")];
        let (list, id) = prefer_or_create_empty_session(
            list,
            Some("/w".to_string()),
            "t".into(),
            || "s_new".to_string(),
            || 5,
        );
        assert_eq!(id, "s1", "复用列表里的空白行，不新建");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].workspace_root.as_deref(), Some("/w"));
    }

    /// 复用分支必须惰性：既不该生成新 id，也不该读时钟。
    ///
    /// Web 侧那两个来源是 `js_sys` 调用，提前求值会让 host target 的
    /// `cargo test --lib` 在 wasm-bindgen 导入函数上 panic。
    #[test]
    fn prefer_reuses_blank_without_touching_id_or_clock() {
        let (list, id) = prefer_or_create_empty_session(
            vec![session("s1")],
            None,
            "t".into(),
            || -> String { panic!("复用分支不该生成新 id") },
            || -> i64 { panic!("复用分支不该读时钟") },
        );
        assert_eq!(id, "s1");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].workspace_root, None, "None 不该写绑定");
    }

    #[test]
    fn prefer_keeps_existing_root_when_none_given() {
        let mut blank = session("s1");
        blank.workspace_root = Some("/keep".to_string());
        let (list, id) = prefer_or_create_empty_session(
            vec![blank],
            None,
            "t".into(),
            || "s_new".to_string(),
            || 5,
        );
        assert_eq!(id, "s1");
        assert_eq!(
            list[0].workspace_root.as_deref(),
            Some("/keep"),
            "None 不该抹掉已有绑定"
        );
    }

    #[test]
    fn prefer_inserts_when_nothing_is_blank() {
        let mut used = session("s1");
        used.messages.push(msg("m1"));
        let (list, id) = prefer_or_create_empty_session(
            vec![used],
            None,
            "t".into(),
            || "s_new".to_string(),
            || 5,
        );
        assert_eq!(id, "s_new");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "s_new", "新行插到最前");
        assert_eq!(list[0].title, "t");
        assert_eq!(list[0].layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
        assert!(session_is_blank_chat(&list[0]));
    }

    #[test]
    fn reclaim_drops_only_disposable_rows() {
        let mut list = vec![session("s1"), session("s2")];
        list[1].pinned = true;

        reclaim_disposable_blank_session(&mut list, "s1");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "s2");

        reclaim_disposable_blank_session(&mut list, "s2");
        assert_eq!(list.len(), 1, "置顶的空白行不回收");

        reclaim_disposable_blank_session(&mut list, "");
        assert_eq!(list.len(), 1, "空 id 不动作");

        reclaim_disposable_blank_session(&mut list, "gone");
        assert_eq!(list.len(), 1, "行不存在不动作");
    }

    #[test]
    fn session_id_encodes_injected_time_and_entropy() {
        assert_eq!(new_session_id(1_700_000_000_000, 42), "s_1700000000000_42");
        // 同一毫秒靠不同熵区分；熵源由调用方保证。
        assert_ne!(new_session_id(1, 1), new_session_id(1, 2));
    }

    #[test]
    fn v2_projection_detection_and_normalize() {
        let s = session("s1");
        assert_eq!(s.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
        assert!(s.has_v2_layout_projection());

        // 旧缓存：版本 0 → 归一到 legacy，且不算 v2 投影。
        let mut old = session("s2");
        old.layout_schema_version = 0;
        assert!(!old.has_v2_layout_projection());
        old.normalize_layout_schema_version();
        assert_eq!(old.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);

        // 版本字段是旧的，但含稳定 v2 行 id：稳定 key 优先。
        let mut partial = session("s3");
        partial.layout_schema_version = LEGACY_LAYOUT_SCHEMA_VERSION;
        partial.messages.push(msg(V2_FINAL_ANSWER_ROW_ID));
        assert!(partial.has_v2_finalized_rows());
        assert!(partial.has_v2_layout_projection());
        partial.normalize_layout_schema_version();
        assert_eq!(partial.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
    }

    #[test]
    fn trimmed_server_conversation_id_uses_shared_resume_rule() {
        let mut s = session("s1");
        assert_eq!(s.trimmed_server_conversation_id(), None);
        s.server_conversation_id = Some("  c1  ".to_string());
        assert_eq!(s.trimmed_server_conversation_id(), Some("c1"));
        s.server_conversation_id = Some("   ".to_string());
        assert_eq!(s.trimmed_server_conversation_id(), None);
    }

    #[test]
    fn legacy_session_json_defaults_to_legacy_layout_version() {
        // 旧缓存没有 layout_schema_version 字段：serde default 必须是 v1，且不构成 v2 投影。
        // 这条断言原先只存在于 wasm crate（本地与 CI 都不跑），下沉后必须能在共享层实跑。
        let s: ChatSession =
            serde_json::from_str(r#"{"id":"s","messages":[]}"#).expect("deserialize legacy");
        assert_eq!(s.layout_schema_version, LEGACY_LAYOUT_SCHEMA_VERSION);
        assert!(!s.has_v2_finalized_rows());
        assert!(!s.has_v2_layout_projection());
    }

    #[test]
    fn json_with_stable_v2_row_id_upgrades_on_normalize() {
        // 版本字段仍是旧的，但消息里已带稳定 v2 行 id：从 JSON 反序列化后归一为当前版本。
        let mut s: ChatSession = serde_json::from_str(
            r#"{"id":"s","layout_schema_version":1,"messages":[{"id":"turn-commentary-t1","role":"assistant"}]}"#,
        )
        .expect("deserialize pre-versioned v2");
        assert!(s.has_v2_layout_projection(), "稳定 key 优先于版本字段");
        s.normalize_layout_schema_version();
        assert_eq!(s.layout_schema_version, CURRENT_LAYOUT_SCHEMA_VERSION);
    }

    #[test]
    fn default_title_matches_any_locale_but_never_empty() {
        for s in ["新会话", "New chat", "new chat", "NEW CHAT", "  新会话  "] {
            assert!(is_default_session_title(s), "{s:?} 应算默认标题");
        }
        // 空串 / 全空白 / 别的名字都不算——空串是 `title` 的 serde 缺省值。
        for s in ["", "   ", "我的会话", "New chats", "新会话 2"] {
            assert!(!is_default_session_title(s), "{s:?} 不该算默认标题");
        }
    }

    #[test]
    fn title_from_prompt_flattens_and_collapses() {
        assert_eq!(
            title_from_user_prompt("  hello\nworld  ", DEFAULT_CHAT_SESSION_TITLE),
            "hello world"
        );
        assert_eq!(
            title_from_user_prompt("a\t\tb", DEFAULT_CHAT_SESSION_TITLE),
            "a b"
        );
        // 全空白退回调用方给的默认标题（Web 与 TUI 传的不同）。
        assert_eq!(
            title_from_user_prompt("  \n\t ", DEFAULT_CHAT_SESSION_TITLE),
            DEFAULT_CHAT_SESSION_TITLE
        );
        assert_eq!(
            title_from_user_prompt("   ", "新会话"),
            "新会话",
            "默认标题由调用方决定"
        );
    }

    #[test]
    fn title_from_prompt_truncates_at_48_chars() {
        let body = "字".repeat(60);
        let out = title_from_user_prompt(&body, DEFAULT_CHAT_SESSION_TITLE);
        assert_eq!(out.chars().count(), 48, "47 个字符 + 省略号");
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().take(47).collect::<String>(), "字".repeat(47));

        // 恰好 48 个字符不截断。
        let exact = "字".repeat(48);
        assert_eq!(
            title_from_user_prompt(&exact, DEFAULT_CHAT_SESSION_TITLE),
            exact
        );
    }
}
