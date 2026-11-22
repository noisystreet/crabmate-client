//! `GET /workspace/git`：当前工作区的只读 git 状态投影（分支 + 未提交脏摘要）。
//!
//! 响应**一律 200**，可用性在 body 的 `state` 里（四值互不相同），所以这里不做
//! 「按状态码分流」——只有真正拿不到响应（老 serve 的 404、离线、Bearer 失效）才是 `Err`。
//! 调用方对 `Err` 的处理是**不显示徽标**：宁可没有，也不要渲染一个看起来干净的分支名。

use crabmate::cm_api_contract::workspace::WorkspaceGitStatusResponse;
use crabmate_client_api::paths;

use crate::i18n::Locale;

use super::http::fetch_json;

/// 拉取工作区 git 状态；`refresh = true` 跳过服务端缓存读（手动刷新手势）。
pub async fn fetch_workspace_git(
    refresh: bool,
    loc: Locale,
) -> Result<WorkspaceGitStatusResponse, String> {
    fetch_json("GET", &paths::workspace_git(refresh), None, loc).await
}
