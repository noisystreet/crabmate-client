//! `localStorage` 的**连接分域** —— #457 W0-2 / W0-3 / W0-4 的落点。
//!
//! `localStorage` 属于 **origin**，不属于 WebView 实例：Desktop 的多个连接窗口共享同一个
//! `tauri.localhost` origin，固定键名于是让两个窗口互相覆盖同一份配置
//! （`docs/design/tui_multi_server.md` §9.4.1 把它列为「单窗口多 lane 的唯一前置硬障碍」，
//! 而它其实是**路线 A 下就存在的 bug**）。
//!
//! 本模块只回答一个问题：**「基础键」在本页属于哪个连接的命名空间**。
//!
//! - 有连接标识（Desktop 连接窗口）→ `crabmate-api-base-url#<key>`；
//! - 没有（纯浏览器 / Android / Desktop 主窗口 / `web-host`）→ 原样用旧键名：
//!   单连接语义与用户已有数据都保持原样，**零迁移**。
//!
//! # 标识从哪来（为什么不是一条 `invoke`）
//!
//! 壳在建窗口时用 `WebviewWindowBuilder::initialization_script` 注入
//! `globalThis.__CM_CONN_KEY__`，它**先于**页面脚本执行，因此 [`connection_key`] 是同步可用的。
//! 这一点是被读路径逼出来的：`hydrate_api_base_once` 是同步的，如果标识只能异步拿到
//! （`invoke` 回壳查注册表），启动水合就会**先读不分域键、后读写分域键** —— 读写不对称，
//! 重启后配置读不回来。异步方案要补「纠正重放」，比同步注入更容易出隐蔽 bug。
//!
//! 当前的注入值恰好等于 window label（见 `ConnectionKey` 的 doc）。W1 若要解耦「连接身份」
//! 与「窗口身份」，只改壳侧注入的值，本模块与全部调用点都不用动 —— 这正是 W0-3 想要的形状。
//!
//! # 键名单一来源
//!
//! 四个「与连接绑定」的键只在本模块出现（`scripts/check-storage-scope.sh` 守这条）：
//! 其它模块必须经 [`API_BASE_URL_KEY`] / [`WEB_API_BEARER_TOKEN_KEY`] /
//! [`LLM_THINKING_MODE_KEY`] 取名字，再交给 `api::browser` 的分域读写。
//! **应用级偏好**（语言、主题、列宽、会话草稿）刻意**不**分域：它们描述这台机器 /
//! 这个用户，不描述某台 `serve`。

/// 与历史文档 / `serve` 启动提示一致：纯浏览器侧引导缓存键（官方壳不得写入）。
pub(crate) const WEB_API_BEARER_TOKEN_KEY: &str = "crabmate-api-bearer-token";

/// 跨 Origin 时指向远程 `serve` 根（无尾斜杠）；空 = 同 Origin。
pub(crate) const API_BASE_URL_KEY: &str = "crabmate-api-base-url";

/// 思考模式本机两态（`on` / `off`）：非机密偏好，但它描述**这次连接**怎么发请求
/// （换成另一台 serve 时两台的模型配置本来就不同），因此按连接分域。
pub(crate) const LLM_THINKING_MODE_KEY: &str = "crabmate-llm-thinking-mode";

/// 壳注入的连接标识所在的 JS 全局名（`desktop-tauri` 的 `desktop_main_window` 写入）。
pub(crate) const INJECTED_GLOBAL: &str = "__CM_CONN_KEY__";

/// 分域分隔符。
///
/// 用 `#` 而不是 `:` / `/`：Tauri 的 window label 字符集是 `[A-Za-z0-9-/:]`，后两个会和
/// 分隔符撞。`#` 不在 label 里，也不影响 `localStorage` 的键合法性；本模块只拼不拆，
/// 所以不需要转义。
const SCOPE_SEP: char = '#';

/// 本页所属连接的标识；`None` = 不按连接分域（单连接语义）。
///
/// 值取壳注入的字符串，本模块**不**再做任何规范化：壳侧（`desktop_main_window`）已经过
/// `ConnectionKey::from_raw`，与注册表 key 是同一个函数算出来的，两边因此不会分叉。
#[cfg(target_arch = "wasm32")]
pub(crate) fn connection_key() -> Option<String> {
    let raw = js_sys::Reflect::get(
        &js_sys::global(),
        &wasm_bindgen::JsValue::from_str(INJECTED_GLOBAL),
    )
    .ok()?
    .as_string()?;
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 非 wasm（测试 / SSR）：没有壳，也就没有连接标识。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn connection_key() -> Option<String> {
    None
}

/// 分域键（纯函数，便于单测）：`base` + `#` + `key`；`key` 为 `None` 时原样返回 `base`。
#[must_use]
pub(crate) fn scoped_key_with(base: &str, key: Option<&str>) -> String {
    match key {
        Some(k) if !k.is_empty() => format!("{base}{SCOPE_SEP}{k}"),
        _ => base.to_string(),
    }
}

/// [`scoped_key_with`] 的当前环境版本。
#[must_use]
pub(crate) fn scoped_key(base: &str) -> String {
    scoped_key_with(base, connection_key().as_deref())
}

/// 读侧的迁移判定（纯函数）：分域键存在就用它；缺失才回落到旧键，并提示调用方回填。
///
/// 返回 `(采用的值, 是否需要把该值回填到分域键)`。
///
/// **`Some("")` 与 `None` 不等价**：`crabmate-api-base-url` 的空串表示「显式同 Origin」，
/// 缺失表示「用构建期默认」。所以这里搬的是 `Option<String>` 原值，不做 trim / 折空 ——
/// 折了就会让「用户清空过基址」重新回落到 `CRABMATE_API_BASE`。
///
/// 幂等性由「分域键存在即不看旧键」保证：迁移只发生一次，之后永远读分域键。
#[must_use]
pub(crate) fn resolve_scoped_read(
    scoped: Option<String>,
    legacy: Option<String>,
) -> (Option<String>, bool) {
    if scoped.is_some() {
        return (scoped, false);
    }
    match legacy {
        Some(v) => (Some(v), true),
        None => (None, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_connection_key_keeps_the_historical_key_name() {
        assert_eq!(
            scoped_key_with("crabmate-api-base-url", None),
            "crabmate-api-base-url"
        );
        assert_eq!(
            scoped_key_with("crabmate-api-base-url", Some("")),
            "crabmate-api-base-url"
        );
    }

    #[test]
    fn connection_key_appends_a_namespaced_suffix() {
        assert_eq!(
            scoped_key_with("crabmate-api-base-url", Some("127-0-0-1-8080")),
            "crabmate-api-base-url#127-0-0-1-8080"
        );
        // label 允许 `/` 与 `:`，分隔符必须是它们之外的字符。
        assert_eq!(scoped_key_with("k", Some("lane/2:prod")), "k#lane/2:prod");
    }

    #[test]
    fn scoped_value_wins_over_the_legacy_one() {
        assert_eq!(
            resolve_scoped_read(Some("http://b".into()), Some("http://a".into())),
            (Some("http://b".into()), false)
        );
    }

    #[test]
    fn missing_scoped_key_migrates_the_legacy_value_once() {
        assert_eq!(
            resolve_scoped_read(None, Some("http://a".into())),
            (Some("http://a".into()), true)
        );
        // 第二次：分域键已被回填，不再看旧键。
        assert_eq!(
            resolve_scoped_read(Some("http://a".into()), Some("http://a".into())),
            (Some("http://a".into()), false)
        );
    }

    #[test]
    fn explicit_empty_string_is_migrated_as_a_value_not_as_absence() {
        // 空串是「显式同 Origin」，迁移后仍必须是「键存在」。
        assert_eq!(
            resolve_scoped_read(None, Some(String::new())),
            (Some(String::new()), true)
        );
    }

    #[test]
    fn nothing_to_migrate_when_both_sides_are_absent() {
        assert_eq!(resolve_scoped_read(None, None), (None, false));
    }
}
