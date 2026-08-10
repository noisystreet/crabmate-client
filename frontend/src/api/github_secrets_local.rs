//! GitHub OAuth Client ID（本机 LS）与 user access token（壳钥匙串 / 浏览器 Cookie）。
//!
//! - Client ID：各端 `localStorage`；仅发起 Device Flow 时需要
//! - User token：壳写入钥匙串槽 `github`，请求头 `X-CrabMate-GitHub-Token`；
//!   纯浏览器依赖服务端 `Set-Cookie`（HttpOnly），**禁止** LS/内存持久化 token

use std::cell::RefCell;

use super::browser::{clear_request_github_token, set_request_github_token, window};
use super::llm_secrets_local::{
    PersistKind, bridge_load_secure_slot, bridge_persist_secure_slot,
    secure_llm_secret_backend_available,
};
use crate::i18n::Locale;

const LS_CLIENT_ID: &str = "crabmate-github-oauth-client-id";
/// 浏览器连接态提示（非机密）：上次成功授权的 login；壳以钥匙串为准。
const LS_SESSION_LOGIN: &str = "crabmate-github-session-login";
const SLOT_GITHUB: &str = "github";

thread_local! {
    static TOKEN: RefCell<String> = const { RefCell::new(String::new()) };
}

fn read_ls(key: &str) -> Option<String> {
    let w = window()?;
    let storage = w.local_storage().ok().flatten()?;
    let v = storage.get_item(key).ok().flatten()?;
    let t = v.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
}

fn write_ls(key: &str, value: &str) {
    let Some(w) = window() else {
        return;
    };
    let Ok(Some(storage)) = w.local_storage() else {
        return;
    };
    let t = value.trim();
    if t.is_empty() {
        let _ = storage.remove_item(key);
    } else {
        let _ = storage.set_item(key, t);
    }
}

/// 是否存在壳安全存储（桌面钥匙串 / Android Keystore）。
#[must_use]
pub fn github_token_secure_backend_available() -> bool {
    secure_llm_secret_backend_available()
}

#[must_use]
pub fn github_oauth_client_id() -> String {
    read_ls(LS_CLIENT_ID).unwrap_or_default()
}

#[must_use]
pub fn github_oauth_client_id_is_set() -> bool {
    !github_oauth_client_id().trim().is_empty()
}

pub fn persist_github_oauth_client_id(client_id: &str) {
    write_ls(LS_CLIENT_ID, client_id);
}

pub fn clear_github_oauth_client_id() {
    write_ls(LS_CLIENT_ID, "");
}

fn sync_request_header_from_memory() {
    TOKEN.with(|c| {
        let t = c.borrow();
        if t.is_empty() {
            clear_request_github_token();
        } else {
            set_request_github_token(&t);
        }
    });
}

fn wipe_local_github_session_state() {
    TOKEN.with(|c| *c.borrow_mut() = String::new());
    clear_request_github_token();
    write_ls(LS_SESSION_LOGIN, "");
}

/// 从钥匙串 / Keystore 水合壳内 token（浏览器无操作）。
pub async fn hydrate_github_secrets_from_secure_store() {
    if !github_token_secure_backend_available() {
        clear_request_github_token();
        return;
    }
    let loaded = bridge_load_secure_slot(SLOT_GITHUB)
        .await
        .unwrap_or_default();
    TOKEN.with(|c| *c.borrow_mut() = loaded);
    sync_request_header_from_memory();
}

/// Device Flow 成功后：壳写入钥匙串；浏览器只记 login（token 在 Cookie）。
pub async fn on_device_flow_success(
    access_token: Option<&str>,
    login: Option<&str>,
) -> Result<(), String> {
    if github_token_secure_backend_available() {
        let t = access_token
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "壳端未收到 access_token（请确认 X-CrabMate-GitHub-Token-Delivery）".to_string()
            })?;
        let kind = bridge_persist_secure_slot(SLOT_GITHUB, t).await?;
        if kind != PersistKind::Durable {
            return Err("GitHub token 未能写入本机安全存储".into());
        }
        TOKEN.with(|c| *c.borrow_mut() = t.to_string());
        sync_request_header_from_memory();
        if let Some(login_t) = login.map(str::trim).filter(|s| !s.is_empty()) {
            write_ls(LS_SESSION_LOGIN, login_t);
        }
        return Ok(());
    }
    // 浏览器：禁止持久化 token；仅保存连接态提示。
    clear_request_github_token();
    TOKEN.with(|c| *c.borrow_mut() = String::new());
    let login_t = login.map(str::trim).unwrap_or("");
    if login_t.is_empty() {
        write_ls(LS_SESSION_LOGIN, "1");
    } else {
        write_ls(LS_SESSION_LOGIN, login_t);
    }
    Ok(())
}

/// 断开本机态：内存与请求头、浏览器 session 标记**无条件**清除；壳再尽力清钥匙串。
///
/// 钥匙串清除失败时仍返回 `Err`（本地请求态已断开），调用方应提示用户可能残留。
pub async fn clear_github_connection_local() -> Result<(), String> {
    wipe_local_github_session_state();
    if !github_token_secure_backend_available() {
        return Ok(());
    }
    bridge_persist_secure_slot(SLOT_GITHUB, "")
        .await
        .map(|_| ())
        .map_err(|e| format!("本机钥匙串未能清除 GitHub token: {e}"))
}

#[must_use]
pub fn github_token_is_set() -> bool {
    if github_token_secure_backend_available() {
        return TOKEN.with(|c| !c.borrow().trim().is_empty());
    }
    read_ls(LS_SESSION_LOGIN).is_some()
}

fn looks_like_github_auth_failure(msg: &str) -> bool {
    let low = msg.to_ascii_lowercase();
    low.contains("401")
        || low.contains("unauthorized")
        || low.contains("bad credentials")
        || low.contains("requires authentication")
        || low.contains("authentication failed")
        || low.contains("gh auth login")
        || (low.contains("认证") && (low.contains("失败") || low.contains("无效")))
}

/// 刷新连接态：壳看钥匙串内存；浏览器在有 session 标记时用 `repo-context` 探活，鉴权失败则清标记。
pub async fn reconcile_github_connection_status(loc: Locale) -> bool {
    if github_token_secure_backend_available() {
        return github_token_is_set();
    }
    if read_ls(LS_SESSION_LOGIN).is_none() {
        return false;
    }
    match super::http::fetch_github_repo_context(loc).await {
        Ok(ctx) if ctx.connected => true,
        Ok(ctx) => {
            if let Some(err) = ctx.error.as_deref()
                && looks_like_github_auth_failure(err)
            {
                wipe_local_github_session_state();
                return false;
            }
            // 非 git 仓 / gh 不可用等：保留「曾成功授权」乐观态，避免误踢。
            true
        }
        Err(e) if looks_like_github_auth_failure(&e) => {
            wipe_local_github_session_state();
            false
        }
        Err(_) => true,
    }
}
