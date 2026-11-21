//! 一条连接的作用域（#457 W0-1）：**这屏连的是哪台 `serve`、用哪个 Bearer**。
//!
//! 纯数据、无 IO：持久化（`localStorage` / 钥匙串）、HTTP、窗口都在调用方。放在
//! `crabmate-client-api` 是为了让「规范化规则 + 不变量」只有一份 —— WASM / TUI /
//! 将来的多 lane 共用同一套，而不是各写一遍「哪个算空基址」「Bearer 要不要 trim」。
//!
//! 与 [`crate::connections::ConnectionSnapshot`] 的分工：那个是**注册表里的一行**
//! （给 UI 画连接列表：状态、读数、在途耗时）；这个只回答「请求打到哪、带什么凭证」。
//! 一个窗口一条连接时两者一一对应，多 lane 之后 scope 会成为每条 lane 的随行物，
//! 而快照只属于「列表那一屏」。
//!
//! **Debug 不打印 Bearer**：类型一旦 `derive(Debug)`，`tracing` / `println!` /
//! 测试失败输出都会把凭证带进日志。这里手写，只报「有没有」。

use std::fmt;

use crate::connections::ConnectionKey;
use crate::url::{api_base_authority, normalize_api_base};

/// 一条连接的作用域。
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionScope {
    key: ConnectionKey,
    /// 规范化后的基址；空 = 同 Origin（相对路径请求）。
    api_base: String,
    /// trim 后的 Web API Bearer；空 = 没有（`serve` 未要求鉴权）。
    bearer: String,
}

impl ConnectionScope {
    /// 建立作用域。`api_base` 走与全仓同一套规范化（[`normalize_api_base`]），
    /// 所以「带尾斜杠」「首尾空白」「非法 URL」在这里就被抹平，下游不必再判断。
    #[must_use]
    pub fn new(key: ConnectionKey, api_base: &str, bearer: &str) -> Self {
        Self {
            key,
            api_base: normalize_api_base(api_base),
            bearer: bearer.trim().to_string(),
        }
    }

    /// 连接标识（localStorage 分域、窗口注册表用的都是它）。
    #[must_use]
    pub fn key(&self) -> &ConnectionKey {
        &self.key
    }

    /// 基址；空串表示「同 Origin」。
    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// Web API Bearer；空串表示没有。**不要把它写进日志**（见类型级文档）。
    #[must_use]
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    /// 有没有非空 Bearer。
    #[must_use]
    pub fn has_bearer(&self) -> bool {
        !self.bearer.is_empty()
    }

    /// 基址为空 = 相对路径直连本页 Origin（纯浏览器 / `web-host` 的常态）。
    #[must_use]
    pub fn is_same_origin(&self) -> bool {
        self.api_base.is_empty()
    }

    /// 基址的 `host:port`（给状态栏 / 连接行做短标签）；空基址给空串。
    #[must_use]
    pub fn authority(&self) -> String {
        api_base_authority(&self.api_base)
    }

    /// 换基址，`key` 与 Bearer 不动。
    ///
    /// 这是「同一个窗口改连另一台 `serve`」的形状：标识仍是那条连接的身份
    /// （W0-3 的等式暂不变），只是它指向的地址变了。
    #[must_use]
    pub fn with_api_base(&self, api_base: &str) -> Self {
        Self {
            api_base: normalize_api_base(api_base),
            ..self.clone()
        }
    }

    /// 换 Bearer，其余不动。
    #[must_use]
    pub fn with_bearer(&self, bearer: &str) -> Self {
        Self {
            bearer: bearer.trim().to_string(),
            ..self.clone()
        }
    }
}

/// 空基址（= 同 Origin）的固定指纹。
///
/// 它表示「连的就是本页自己这一份」，不是「没有连接」—— 所以给常量而不是空串，
/// 否则分域键会退化成 `<基础键>#<label>#`（尾部分隔符悬空）。
pub const SAME_ORIGIN_FINGERPRINT: &str = "same-origin";

/// 基址短指纹（8 个十六进制字符）：把「同一窗口连过的不同 `serve`」分成不同的
/// 存储域 / 钥匙串槽（#457）。
///
/// 输入先按全仓同一套规则规范化（[`normalize_api_base`]），所以 `http://h:1` 与
/// ` http://h:1/ ` 得到同一个指纹 —— 否则同一台 serve 会因一个尾斜杠分裂成两份配置，
/// 而症状只是「设置回到了默认值」，查不到分域头上。
///
/// 用 FNV-1a 而不是 SHA-256：这里要的是「撞不上就好」的去重，不是抗篡改；共享层不为
/// 一个哈希拉加密依赖。指纹会出现在 `localStorage` 键名与钥匙串账户名里，所以只输出
/// `[0-9a-f]` —— 不含分域分隔符 `#` 与 `@`，拼接方不必转义。
#[must_use]
pub fn api_base_fingerprint(api_base: &str) -> String {
    let normalized = normalize_api_base(api_base);
    if normalized.is_empty() {
        return SAME_ORIGIN_FINGERPRINT.to_string();
    }
    let mut hash: u32 = 0x811c_9dc5;
    for byte in normalized.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

impl fmt::Debug for ConnectionScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionScope")
            .field("key", &self.key)
            .field("api_base", &self.api_base)
            // 只报有没有：值本身可能进崩溃报告 / 测试失败输出。
            .field(
                "bearer",
                &if self.has_bearer() {
                    "<set>"
                } else {
                    "<empty>"
                },
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ConnectionKey {
        ConnectionKey::from_raw("lane-a")
    }

    #[test]
    fn api_base_is_normalised_on_the_way_in() {
        let s = ConnectionScope::new(key(), "  http://127.0.0.1:8080/ ", "");
        assert_eq!(s.api_base(), "http://127.0.0.1:8080");
        assert!(!s.is_same_origin());
        assert_eq!(s.authority(), "127.0.0.1:8080");
    }

    #[test]
    fn empty_or_invalid_api_base_means_same_origin() {
        // 非法（相对路径 / 非 http）与空白都归一成空串 = 同 Origin，而不是留着半截地址。
        for raw in ["", "   ", "/only-path", "ftp://x"] {
            let s = ConnectionScope::new(key(), raw, "tok");
            assert!(s.is_same_origin(), "raw={raw:?}");
            assert_eq!(s.authority(), "", "raw={raw:?}");
        }
    }

    #[test]
    fn bearer_is_trimmed_and_emptiness_is_explicit() {
        let s = ConnectionScope::new(key(), "http://h:1", "  secret  ");
        assert_eq!(s.bearer(), "secret");
        assert!(s.has_bearer());

        let cleared = s.with_bearer("   ");
        assert!(!cleared.has_bearer(), "清空是「没有」，不是「一个空格」");
        assert_eq!(cleared.api_base(), "http://h:1", "换 Bearer 不该动基址");
        assert_eq!(cleared.key(), s.key(), "换 Bearer 不该动标识");
    }

    #[test]
    fn swapping_the_base_keeps_the_identity_and_the_bearer() {
        let s = ConnectionScope::new(key(), "http://a:1", "tok");
        let moved = s.with_api_base("https://b:2/");
        assert_eq!(moved.api_base(), "https://b:2");
        assert_eq!(moved.bearer(), "tok");
        assert_eq!(moved.key(), s.key());
    }

    #[test]
    fn debug_never_prints_the_bearer() {
        let s = ConnectionScope::new(key(), "http://a:1", "super-secret");
        let dump = format!("{s:?}");
        assert!(!dump.contains("super-secret"), "{dump}");
        assert!(dump.contains("<set>"), "{dump}");
        assert!(dump.contains("lane-a"), "{dump}");

        let empty = ConnectionScope::new(key(), "http://a:1", "");
        assert!(format!("{empty:?}").contains("<empty>"));
    }

    #[test]
    fn fingerprint_ignores_the_normalization_noise() {
        // 同一台 serve 的四种写法必须落到同一个域，否则「设置回到了默认值」。
        let a = api_base_fingerprint("http://127.0.0.1:8080");
        assert_eq!(a, api_base_fingerprint(" http://127.0.0.1:8080/ "));
        assert_eq!(a, api_base_fingerprint("http://127.0.0.1:8080///"));
    }

    #[test]
    fn different_serves_get_different_fingerprints() {
        assert_ne!(
            api_base_fingerprint("http://127.0.0.1:8080"),
            api_base_fingerprint("http://127.0.0.1:8081")
        );
        // 协议差一个字符也算不同连接：这正是两级键要分开记的那种情况。
        assert_ne!(
            api_base_fingerprint("http://a:1"),
            api_base_fingerprint("https://a:1")
        );
    }

    #[test]
    fn same_origin_has_its_own_constant_fingerprint() {
        // 空 = 同 Origin，是「一份真实配置」，不是「没有连接」。
        assert_eq!(api_base_fingerprint(""), SAME_ORIGIN_FINGERPRINT);
        assert_eq!(api_base_fingerprint("   "), SAME_ORIGIN_FINGERPRINT);
        assert_ne!(api_base_fingerprint(""), api_base_fingerprint("http://a:1"));
    }

    #[test]
    fn fingerprint_never_carries_a_separator() {
        // 它要拼进 `localStorage` 键名与钥匙串账户名（`#` / `@` 是分隔符）。
        for base in ["http://127.0.0.1:8080", "https://serve.example/sub/path"] {
            let fp = api_base_fingerprint(base);
            assert_eq!(fp.len(), 8, "{fp}");
            assert!(
                fp.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "{fp}"
            );
        }
    }
}
