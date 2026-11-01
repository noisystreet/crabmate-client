//! 严格绝对 API 基址规范化与 path join。
//!
//! 连接页「可补 `http://`、拒 `0.0.0.0`」等输入规范化仍属 `crabmate-connect`；
//! 本模块只接受已是 `http(s)://…` 的基址。

/// API 基址或 path 不合法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiUrlError {
    InvalidBase(String),
    InvalidPath(String),
}

impl core::fmt::Display for ApiUrlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidBase(m) | Self::InvalidPath(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ApiUrlError {}

/// 去掉首尾空白与尾 `/`；非绝对 `http(s)://` 返回空串。
#[must_use]
pub fn normalize_api_base(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    let without_slash = t.trim_end_matches('/');
    if !(without_slash.starts_with("http://") || without_slash.starts_with("https://")) {
        return String::new();
    }
    without_slash.to_string()
}

/// `base` + `path`（`path` 空视为 `/`，否则须以 `/` 开头；`base` 必须是绝对 http(s)）。
pub fn join_api_path(base: &str, path: &str) -> Result<String, ApiUrlError> {
    let base = normalize_api_base(base);
    if base.is_empty() {
        return Err(ApiUrlError::InvalidBase(
            "API base must be an absolute http(s) URL".into(),
        ));
    }
    let path = if path.is_empty() {
        "/"
    } else if path.starts_with('/') {
        path
    } else {
        return Err(ApiUrlError::InvalidPath(format!(
            "path must start with /: {path}"
        )));
    };
    Ok(format!("{base}{path}"))
}

/// API 基址的 `host[:port]` 短文本（RFC 3986 的 authority，去 userinfo）；不是绝对
/// `http(s)` 基址时返回空串。
///
/// 用途是**窄**位置，目前两个消费者：WASM 状态栈的「连接」chip 恒用它；TUI 状态栈的
/// `serve` 段在列宽不够时才退到它（够宽时显示完整 base——终端没有 hover，那是 TUI 唯一
/// 能看到全量 URL 的地方）。这些位置要回答的只有「我连的是哪台 serve」，scheme 每行都
/// 一样，path 也不该出现在基址里，真正能区分两台 serve 的只有 host 与**端口**；
/// 完整 URL 留给 `title`。
///
/// 与 `frontend` 的 `api::browser::api_base_host` **不是**同一件事，刻意不同名：
/// 那个返回 host **不含端口**，喂给 loopback 判定（`127.0.0.1:8080` 与
/// `127.0.0.1:8081` 对「是否本机」没有区别）；这一个必须留端口，否则同一台机器上
/// 的两个 serve 在 UI 上长得一模一样。
///
/// 三条刻意的规则：
/// 1. **丢掉 userinfo**（`http://user:secret@host:8080` → `host:8080`）：这是给人看的
///    位置，一个拼错进基址的凭据不该被顺手画到屏幕上。
/// 2. `?` / `#` 与 `/` 同权截断 —— 基址不该有它们，真出现了也不该把查询串带进 UI。
/// 3. 认不出基址时返回空串而不是原件：调用方据此整段不渲染，比画一个看起来像
///    地址的残片好。
#[must_use]
pub fn api_base_authority(base: &str) -> String {
    let base = normalize_api_base(base);
    if base.is_empty() {
        return String::new();
    }
    let after_scheme = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .unwrap_or(base.as_str());
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    // 取**最后**一个 `@` 之后：userinfo 本身也可能含 `@`，按第一个切会把 `secret@host`
    // 里的 host 一并当成凭据丢掉。
    authority.rsplit('@').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_slash_and_rejects_relative() {
        assert_eq!(
            normalize_api_base("http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_api_base("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert_eq!(normalize_api_base("  "), "");
        assert_eq!(normalize_api_base("/relative"), "");
        assert_eq!(normalize_api_base("ftp://x"), "");
        // 去尾 `/` 后不再是 http(s):// → 空（与 frontend 一致；严于旧 tui `http://`→`http:`）
        assert_eq!(normalize_api_base("http://"), "");
    }

    #[test]
    fn join_requires_absolute_base_and_slash_path() {
        let u = join_api_path("http://127.0.0.1:8080/", "/chat/stream").unwrap();
        assert_eq!(u, "http://127.0.0.1:8080/chat/stream");
        assert!(join_api_path("", "/health").is_err());
        assert!(join_api_path("http://127.0.0.1:8080", "health").is_err());
        assert_eq!(
            join_api_path("http://127.0.0.1:8080", "").unwrap(),
            "http://127.0.0.1:8080/"
        );
    }

    #[test]
    fn api_base_authority_keeps_only_the_authority() {
        assert_eq!(
            api_base_authority("http://127.0.0.1:8080"),
            "127.0.0.1:8080"
        );
        // 尾 `/` 由 normalize 吃掉，与上面 `join_api_path` 同一口径。
        assert_eq!(
            api_base_authority("http://127.0.0.1:8080/"),
            "127.0.0.1:8080"
        );
        // 基址不该带 path，带了也只取 authority（否则 chip 会显示半截路径）。
        assert_eq!(
            api_base_authority("https://serve.example.com:8443/prefix"),
            "serve.example.com:8443"
        );
        // 默认端口不补：这一行是给人读的，不是用来拼请求的。
        assert_eq!(
            api_base_authority("https://serve.example.com"),
            "serve.example.com"
        );
        // IPv6 保留方括号，读写都认得出来。
        assert_eq!(api_base_authority("http://[::1]:8080"), "[::1]:8080");
    }

    #[test]
    fn api_base_authority_never_leaks_userinfo_or_query() {
        // 凭据不进 UI：这是刻意的取舍，不是顺手截断。
        assert_eq!(
            api_base_authority("http://user:secret@127.0.0.1:8080"),
            "127.0.0.1:8080"
        );
        // userinfo 里自带 `@` 时，host 是**最后**一个 `@` 之后那段。
        assert_eq!(
            api_base_authority("http://user@token@127.0.0.1:8080"),
            "127.0.0.1:8080"
        );
        assert_eq!(api_base_authority("http://host:8080?x=1"), "host:8080");
        assert_eq!(api_base_authority("http://host:8080#frag"), "host:8080");
    }

    #[test]
    fn api_base_authority_is_empty_when_there_is_no_usable_authority() {
        // 空基址 = 同源（WASM 的默认态），调用方据此整段不渲染。
        assert_eq!(api_base_authority(""), "");
        assert_eq!(api_base_authority("   "), "");
        assert_eq!(api_base_authority("/relative"), "");
        assert_eq!(api_base_authority("ftp://x"), "");
        // `http://` 已被 normalize 判为非法，不在这里另立规则。
        assert_eq!(api_base_authority("http://"), "");
        assert_eq!(api_base_authority("http://user:secret@"), "");
    }
}
