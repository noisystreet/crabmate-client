//! `GET /permission`：权限等级只读投影（从 `http.rs` 拆出以守住单文件行数棘轮）。
//!
//! 该路由挂在**受保护** API 面（需 Bearer，与 `/status` 不同）。`404` 是「该 serve
//! 还没有这条路由」（老版本）的唯一可靠信号——`/status` 对新旧 serve 都会成功，
//! 拿它探测会把老 serve 当成支持的。

use web_sys::{Request, RequestInit};

use crabmate_client_api::paths;
use crabmate_client_api::permission::PermissionView;

use super::browser::{api_url, prepare_api_auth};
use super::http::{do_fetch, http_status_error, read_response_text};
use crate::i18n::Locale;

/// `GET /permission` 的结果。**404 必须与「取到了但解析不了」分开**，否则老 serve
/// 会被当成"支持但读取失败"，界面就会一直挂着一条错误。
#[derive(Debug, Clone)]
pub enum PermissionFetch {
    /// 拿到只读投影：`level` / `level_source` / `ceiling` / `available` / `clamped_by` / `caps`
    /// 全部由服务端算好，客户端不重新推导。设置页目前只渲染前五项，
    /// `caps`（六轴投影）随契约一起取回但暂不展示。
    Ok(PermissionView),
    /// 404：该 serve 不支持权限等级（不显示等级，也不报错）。
    Unsupported,
    /// 其它失败（网络 / 401 / 403 / 内容不合法），已本地化为可展示文案。
    Err(String),
}

/// `GET /permission`。
///
/// `agent_role` / `conversation_id` 都可省：前者决定角色上限，后者带来会话持久化的
/// 角色与会话模式；省略即只取静态投影。
pub async fn fetch_permission(
    agent_role: Option<&str>,
    conversation_id: Option<&str>,
    loc: Locale,
) -> PermissionFetch {
    let init = RequestInit::new();
    init.set_method("GET");
    prepare_api_auth(&init).await;
    let url = api_url(&paths::permission(agent_role, conversation_id));
    let req = match Request::new_with_str_and_init(&url, &init) {
        Ok(r) => r,
        Err(e) => return PermissionFetch::Err(format!("request: {e:?}")),
    };
    let resp = match do_fetch(&req, loc).await {
        Ok(r) => r,
        Err(e) => return PermissionFetch::Err(e),
    };
    let status = resp.status();
    // 404 先于读 body 判定：这是「该 serve 没有这条路由」的唯一信号，
    // 且它不需要 body —— 先读 body 会让一次 body 读取失败把老 serve 报成「读取失败」。
    if status == 404 {
        return PermissionFetch::Unsupported;
    }
    let body = match read_response_text(&resp, loc).await {
        Ok(b) => b,
        Err(e) => return PermissionFetch::Err(e),
    };
    if !(200..300).contains(&status) {
        return PermissionFetch::Err(http_status_error(status, &body, loc));
    }
    match serde_json::from_str::<PermissionView>(&body) {
        Ok(v) => PermissionFetch::Ok(v),
        Err(e) => PermissionFetch::Err(e.to_string()),
    }
}
