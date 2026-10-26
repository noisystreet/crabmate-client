//! `POST /conversation/state`：会话级运行态意图的**空闲态**写入口。
//!
//! 与 `GET /permission`（[`super::http_permission`]）同面：受保护，需 Bearer。
//! 「这次响应到底算哪种结局」的规则**不在本文件**，而在共享层
//! `crabmate_client_api::conversation_state` —— 前端与 TUI 必须对「404 的两种含义」
//! 给出同一个答案，各写一份就是新的漂移点。
//!
//! 单独成文件而不是塞进 `http.rs`：那一支已在单文件 920 行棘轮边缘
//! （`do_fetch` / `read_response_text` 就是为这类拆出而 `pub(crate)` 的），
//! 与 `http_permission.rs` 同一做法。

use web_sys::{Request, RequestInit};

use crabmate_client_api::conversation_state::{
    ConversationStateOutcome, ConversationStatePatch, classify_conversation_state_response,
};
use crabmate_client_api::paths;

use super::browser::{api_url, auth_headers, prepare_api_auth};
use super::http::{do_fetch, read_response_text};
use crate::i18n::Locale;

/// 写会话级意图，拿回写后的权限投影。
///
/// 调用方只该把这个 `patch` 填上**本次真正要改的字段**：服务端是部分更新，
/// 顺带回传 `session_mode` / `agent_role` 会把过期值写回去，或送一个未确认的角色吃 400。
///
/// 返回分两层，不能合并：
/// - `Err` = 请求**根本没拿到响应**（建请求 / 网络 / 读 body 失败），本地错误，已本地化；
/// - `Ok(outcome)` = 拿到了 HTTP 响应，由共享层按 `status` + `error.code` 分类。
///
/// 合并会凭空造出一个不存在的状态码，调用方也就分不清「服务端拒绝了」与「压根没连上」。
pub async fn post_conversation_state(
    patch: &ConversationStatePatch,
    loc: Locale,
) -> Result<ConversationStateOutcome, String> {
    let body = serde_json::to_string(patch).map_err(|e| e.to_string())?;
    let init = RequestInit::new();
    init.set_method("POST");
    prepare_api_auth(&init).await;
    let headers = auth_headers();
    let _ = headers.set("Content-Type", "application/json");
    init.set_headers(&headers);
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let url = api_url(paths::CONVERSATION_STATE);
    let req = Request::new_with_str_and_init(&url, &init).map_err(|e| format!("request: {e:?}"))?;
    let resp = do_fetch(&req, loc).await?;
    let status = resp.status();
    // 404 **也要读 body**：这条路由的 404 有两种含义（老 serve 没有路由 / 会话不存在），
    // 判别依据是 body 里有没有 `error.code`。与 `/permission` 那个「裸 404 即能力缺失」
    // 刚好相反，那边可以提前返回，这里不行。
    let text = read_response_text(&resp, loc).await?;
    Ok(classify_conversation_state_response(status, &text))
}
