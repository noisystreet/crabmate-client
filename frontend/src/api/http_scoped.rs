//! JSON over `fetch` 的**按作用域**版本（#569）。
//!
//! 从 `http.rs` 拆出来两个理由：那个文件本来就贴着 920 行门禁（`fn-nloc-ratchet`），
//! 加上这四个函数就超了；而语义上它们也自成一类 —— 「这次请求用哪一份作用域」。
//!
//! # 为什么要显式传 `scope`
//!
//! `browser::api_url` / `prepare_api_auth` / `auth_headers` 的零参口径读的是 `ACTIVE`
//! 指针（「当前前台那条 lane」）。单窗口多 lane 之后这个口径只能回答「现在谁在前台」，
//! 回答不了「**这个请求属于哪条 lane**」—— 后台那条的轮询借它发出去，就打到前台的
//! serve 上了（#569 报告的现象：切 lane 内容不变，来回切几轮两边的状态互相污染）。
//!
//! 所以凡是由**某条 lane 自己的接线**驱动的请求，都要把那条 lane 的 `ScopeHolder` 带下来。
//! 只在前台发生的路径（点审批、斜杠命令、弹窗）继续用零参版本即可 —— 那时前台就是它。

use serde::Deserialize;
use web_sys::{Request, RequestInit};

use crate::i18n::Locale;

use super::browser::{api_url_on, auth_headers_for, prepare_api_auth_on};
use super::connection_scope::{self, ScopeHolder};
use super::http::do_fetch_json;

/// 带**显式作用域**的 JSON 请求（#569）：每条 lane 自己的请求都走这条。
pub(crate) async fn fetch_json_on<T: for<'de> Deserialize<'de>>(
    scope: &ScopeHolder,
    method: &str,
    url: &str,
    _body: Option<&str>,
    loc: Locale,
) -> Result<T, String> {
    let init = RequestInit::new();
    init.set_method(method);
    prepare_api_auth_on(scope, &init).await;
    let url = api_url_on(scope, url);
    let req =
        Request::new_with_str_and_init(&url, &init).map_err(|e| format!("request: {:?}", e))?;
    do_fetch_json(req, loc).await
}

/// 前台口径的 JSON 请求（用户操作触发的路径）；跨 lane 的请用 [`fetch_json_on`]。
pub(crate) async fn fetch_json<T: for<'de> Deserialize<'de>>(
    method: &str,
    url: &str,
    body: Option<&str>,
    loc: Locale,
) -> Result<T, String> {
    fetch_json_on(&connection_scope::active_holder(), method, url, body, loc).await
}

/// 带**显式作用域**的 JSON POST/PUT（#569）；口径同 [`fetch_json_on`]。
pub(crate) async fn fetch_json_with_body_on<T: for<'de> Deserialize<'de>>(
    scope: &ScopeHolder,
    method: &str,
    url: &str,
    body: &str,
    loc: Locale,
) -> Result<T, String> {
    let init = RequestInit::new();
    init.set_method(method);
    prepare_api_auth_on(scope, &init).await;
    let h = auth_headers_for(scope);
    let _ = h.set("Content-Type", "application/json");
    init.set_headers(&h);
    init.set_body(&wasm_bindgen::JsValue::from_str(body));
    let url = api_url_on(scope, url);
    let req =
        Request::new_with_str_and_init(&url, &init).map_err(|e| format!("request: {:?}", e))?;
    do_fetch_json(req, loc).await
}

/// 前台口径的 JSON POST/PUT；跨 lane 的请用 [`fetch_json_with_body_on`]。
pub(crate) async fn fetch_json_with_body<T: for<'de> Deserialize<'de>>(
    method: &str,
    url: &str,
    body: &str,
    loc: Locale,
) -> Result<T, String> {
    fetch_json_with_body_on(&connection_scope::active_holder(), method, url, body, loc).await
}
