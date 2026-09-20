//! 跨端共享的 HTTP API 端点路径常量与构造器（纯逻辑，无 IO）。
//!
//! 只收「跨端实际共用」的端点：单端专属端点（如 frontend 的
//! `/workspace/file*`、`/user-data/mcp-servers*`）仍留在端内，避免
//! 名义共享、实际单消费。`/uploads/*` 为静态资产路径，不属于 API 端点。

/// 健康检查（tui / connect）。
pub const HEALTH: &str = "/health";
/// Shell 视图状态（frontend / tui）。
pub const STATUS_SHELL: &str = "/status?view=shell";
/// 文件上传（frontend）。
pub const UPLOAD: &str = "/upload";
/// 发起 chat 流（tui / frontend）。
pub const CHAT_STREAM: &str = "/chat/stream";
/// 审批决议（tui / frontend）。
pub const CHAT_APPROVAL: &str = "/chat/approval";
/// 会话分叉（frontend）。
pub const CHAT_BRANCH: &str = "/chat/branch";
/// 工作区根（tui / frontend）。
pub const WORKSPACE: &str = "/workspace";
/// 工作区项目列表（tui / frontend）。
pub const WORKSPACE_PROJECTS: &str = "/workspace/projects";
/// 工作区克隆流（frontend）。
pub const WORKSPACE_CLONE_STREAM: &str = "/workspace/clone/stream";
/// 当前工作区会话列表（tui / frontend）。
pub const USER_DATA_WORKSPACE_SESSIONS: &str = "/user-data/workspaces/current/sessions";
/// 用户偏好（tui / connect / frontend）。
pub const USER_DATA_PREFS: &str = "/user-data/prefs";
/// LLM 覆盖配置（tui / frontend）。
pub const USER_DATA_LLM_OVERRIDES: &str = "/user-data/llm-overrides";
/// 会话存储配置（frontend）。
pub const CONFIG_SESSION_CONVERSATION_STORE: &str = "/config/session/conversation-store";

/// 取消指定 job 的 chat 流（tui / frontend）。
#[must_use]
pub fn chat_stream_cancel(job_id: &str) -> String {
    format!("/chat/stream/{job_id}/cancel")
}

#[cfg(test)]
mod tests {
    use super::{CHAT_STREAM, HEALTH, WORKSPACE, chat_stream_cancel};

    #[test]
    fn constants_are_stable() {
        assert_eq!(HEALTH, "/health");
        assert_eq!(CHAT_STREAM, "/chat/stream");
        assert_eq!(WORKSPACE, "/workspace");
    }

    #[test]
    fn cancel_path_embeds_job_id() {
        assert_eq!(chat_stream_cancel("j1"), "/chat/stream/j1/cancel");
    }
}
