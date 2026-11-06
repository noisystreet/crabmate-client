//! 状态栏 / 状态栈的**显示快照**：`LaneApp::status_info` 组装好、渲染层只读的一份值。
//!
//! 从 `render.rs` 拆出来：它是纯数据 + 口径文档，与「怎么画」无关；而 `render.rs` 正贴着
//! 920 行的文件门禁 —— `ws` 段（#425）加一个字段就撞线，抬门禁是最差的选项。
//! `render.rs` 用 `pub use` 转出，`render::StatusInfo` 这条既有路径不变。

/// 状态行展示信息（mod 层组装好的显示值，含 override 标记）。
///
/// 派生 `Default` 是给测试 fixture 用的：它们的任务是"其余字段取中性值"，
/// 每加一个字段就让十来处 `StatusInfo { .. }` 各补一行，pure noise（而且 fn-nloc
/// 把整个 `mod tests` 当一个函数，补一行就可能越界）。
#[derive(Clone, Default)]
pub struct StatusInfo {
    pub api_base: String,
    /// 生效模型；本地 override 时带 `*` 后缀。
    pub model: Option<String>,
    pub role: Option<String>,
    pub mode: Option<String>,
    /// 思考模式，**已按 Desktop 状态栏开关的口径归一化**（`开` / `关`）；线格式
    /// （`on` / `off` / `server`）不漏到渲染层，两端对 `server` 同判（Desktop 只认 `on`）。
    /// `None` = 未拉到 user-data（**不是**「关」）——画成「关」等于把「不知道」说成「关掉了」。
    pub thinking: Option<String>,
    pub running: bool,
    pub cancel_sent: bool,
    /// 生成中 spinner 帧号（事件循环按时间推进；空闲 / 取消中不显示）。
    pub spinner: usize,
    /// transcript 回看行数（>0 时显示 ↑N）。
    pub view_offset: usize,
    /// 活跃搜索词（/find）。
    pub search_term: Option<String>,
    /// 搜索命中逻辑行数。
    pub search_total: usize,
    pub context: String,
    /// 当前生效权限等级（线格式：`chat` / `read` / `workspace` / `approved` / `unrestricted`）。
    ///
    /// `None` = 不知道（老 serve 没有 `/permission`，或还没拉到）。**不要**回落到
    /// `permission_level_default` 之类的配置值来填这个空：那是"没会话意图时会落到哪"，不是
    /// "此刻是哪"，两者在角色上限收窄时并不相同。
    pub permission: Option<String>,
    /// 权限等级**意图**（本地切换选的那一档，尚未被服务端确认时与上面那个不同）。
    ///
    /// 与 [`Self::permission`] 分开存是为了照 Desktop chip 的口径显示「意图 → 生效」：
    /// 服务端允许意图高于生效（min 链钳制），只显示生效档会让用户以为切换没生效；
    /// 而把意图改写成生效值又会让下一条 `/chat` 把被钳制的低档写回会话。
    pub permission_intent: Option<String>,
    /// 主区是不是只读文件视图（`st.editor.open`）：`bool` 无「不知道」态，`Default` = 聊天。
    pub editor_open: bool,
    /// 窄屏工作区指示（issue #425）：左右栏一藏，右栏标题里的工作区名就没有第二处可看。
    ///
    /// `None` = 不显示 —— 宽屏走这一支（标题已经交代了，且状态栈 9 段行预算正好用满）。
    /// `Some(text)` = 要显示的文本，未设置时是 [`WS_UNSET_TEXT`](super::status::WS_UNSET_TEXT)。
    /// 「什么时候才有值」由 `status::narrow_workspace_text` 一处决定，`Default` = `None`
    /// 因此既有 fixture 全部保持「宽屏」形态。
    pub workspace: Option<String>,
}
