//! `/workspace <路径>` 的客户端前置处理：`~` 展开，以及**端内能确定判断的拦截**。
//!
//! 路径语义属于 **serve 主机**（Web 文案写的就是「服务器上的工作区绝对路径」）：远端
//! `repl` / `tui` 连别的机器时，本机 `$HOME` 与 serve 用户的家目录不是一回事。所以：
//!
//! 1. 默认展开，但留 [`EXPAND_TILDE_ENV`] 把 `~` 原样交给 serve（远端部署的正解在 serve）；
//! 2. 只认 `~` 与 `~/…`；`~user` 要查 passwd / NSS，猜错会静默落进别人的目录，直接拦下；
//! 3. 其余输入一律原样透传 —— 池外路径能不能落地是 serve 的白名单策略，本端不做取舍。

use std::path::{Path, PathBuf};

use crate::tui_mode::controls::clear_word;
use crate::tui_mode::tool_summary::strip_control_chars;

/// 关掉客户端 `~` 展开的开关（值为 `off` / `0` / `false` / `no` / `disable`）。
pub(crate) const EXPAND_TILDE_ENV: &str = "CM_TUI_EXPAND_TILDE";

/// 是否在客户端展开 `~`：默认开，[`EXPAND_TILDE_ENV`] 显式关闭时为假。
pub(crate) fn expand_tilde_enabled() -> bool {
    match std::env::var(EXPAND_TILDE_ENV) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no" | "disable"
        ),
        Err(_) => true,
    }
}

/// `/workspace` 的清除词：载荷发空串＝恢复 serve 默认根。
///
/// 与 `/model` `/mode` `/role` 的清除词复用同一组（`controls::clear_word`），另加直观的
/// `default`。空参无法表达它 —— 空参已被「查询当前根」占用，所以这两个入口（全屏 `tui`
/// 与行模式 `repl`）都靠这个词才能回到默认根。
pub(crate) fn is_default_word(arg: &str) -> bool {
    let t = arg.trim();
    clear_word(t) || t.eq_ignore_ascii_case("default")
}

/// 按环境配置展开 `~`；展不开时返回**给用户看**的失败文案，调用方据此拦截、不发请求。
pub(crate) fn resolve(raw: &str) -> Result<String, String> {
    resolve_with(raw, expand_tilde_enabled(), home_dir().as_deref())
}

/// [`resolve`] 的可测内核：开关与家目录都由参数给，测试不读环境变量（并发下不可靠）。
///
/// 只做 `~` 这一件事：结果是否存在、是不是目录、在不在 `allowed_roots` / 项目池里，
/// 全部归 serve 判（客户端没有 whiteList，也不该替它猜）。
pub(crate) fn resolve_with(raw: &str, expand: bool, home: Option<&Path>) -> Result<String, String> {
    let t = raw.trim();
    // 空串是「恢复 serve 默认根」的载荷；非 `~` 开头的东西也一概原样交给 serve。
    if t.is_empty() || !t.starts_with('~') || !expand {
        return Ok(t.to_string());
    }
    if t == "~" {
        return home.map(display).ok_or_else(|| no_home(t));
    }
    let Some(rest) = t.strip_prefix("~/") else {
        return Err(tilde_user(t));
    };
    // 多余的前导 `/` 必须削掉：`Path::join` 遇到绝对路径会**丢掉**基址（`~//etc` → `/etc`）。
    let rest = rest.trim_start_matches('/');
    home.map(|h| display(&h.join(rest)))
        .ok_or_else(|| no_home(t))
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// 错误文案会拼上原始输入（方便用户对照），所以先剥控制字符：`repl` 用 `eprintln!` 直写
/// 终端，夹带的 ANSI 会被当成控制序列执行。
fn no_home(raw: &str) -> String {
    let raw = strip_control_chars(raw);
    format!(
        "无法展开 `{raw}`：本机取不到家目录（HOME / USERPROFILE 未设置）；\
         请改用绝对路径，或设 `{EXPAND_TILDE_ENV}=off` 交给 serve 展开"
    )
}

fn tilde_user(raw: &str) -> String {
    format!(
        "不支持 `~user` 形式（只有 `~` 与 `~/…` 可展开）：{}；请写绝对路径",
        strip_control_chars(raw)
    )
}

/// 本机家目录：类 Unix 看 `HOME`，Windows 看 `USERPROFILE`（不引入新依赖）。
///
/// 非 UTF-8 的家目录（Linux 上合法）也当作「取不到」：展开结果只能经 `to_string_lossy`
/// 交给 serve，替换字符会让路径**静默变形**，不如报错。
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty() && p.to_str().is_some())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{is_default_word, resolve_with};

    fn home() -> &'static Path {
        Path::new("/home/tester")
    }

    /// 清除词：`/workspace off` 之类的载荷语义靠它，两个入口（`tui` / `repl`）共用。
    #[test]
    fn clear_words_are_recognised() {
        for w in ["off", "none", "clear", "default", "Default", "  off  "] {
            assert!(is_default_word(w), "{w} 应视为恢复默认根");
        }
        // 近似词不能被误判：`defaults` 是路径，不是清除词。
        // `off` / `none` / `clear` 与 `controls::clear_word` 一样**大小写敏感**（`/model OFF`
        // 用的是同一套词），所以 `OFF` 按路径对待，`default` 才不分大小写。
        for w in ["defaults", "/data/proj", "~/proj", "", "o", "OFF"] {
            assert!(!is_default_word(w), "{w} 不是清除词");
        }
    }

    #[test]
    fn bare_tilde_is_home() {
        assert_eq!(
            resolve_with("~", true, Some(home())),
            Ok("/home/tester".into())
        );
        assert_eq!(
            resolve_with("  ~  ", true, Some(home())),
            Ok("/home/tester".into())
        );
    }

    #[test]
    fn tilde_slash_prefix_joins_home() {
        assert_eq!(
            resolve_with("~/proj", true, Some(home())),
            Ok("/home/tester/proj".into())
        );
        assert_eq!(
            resolve_with("~/a/b/c", true, Some(home())),
            Ok("/home/tester/a/b/c".into())
        );
        // 末尾斜杠保留（`~/` = 家目录本身）。
        assert_eq!(
            resolve_with("~/", true, Some(home())),
            Ok("/home/tester/".into())
        );
    }

    /// `Path::join` 见到绝对路径会丢基址：`~//etc` 必须仍是家目录下的 `etc`。
    #[test]
    fn extra_leading_slashes_do_not_escape_home() {
        assert_eq!(
            resolve_with("~//etc/passwd", true, Some(home())),
            Ok("/home/tester/etc/passwd".into())
        );
    }

    #[test]
    fn tilde_user_forms_are_rejected() {
        for raw in ["~root", "~root/proj", "~no-such-user"] {
            let err = resolve_with(raw, true, Some(home())).unwrap_err();
            assert!(err.contains("~user"), "{raw} 应报不支持 ~user：{err}");
        }
    }

    /// 取不到家目录时必须报错，不能静默展开成相对路径。
    #[test]
    fn missing_home_is_an_error_not_a_relative_path() {
        for raw in ["~", "~/proj"] {
            let err = resolve_with(raw, true, None).unwrap_err();
            assert!(err.contains("家目录"), "{raw} 应说明取不到家目录：{err}");
            assert!(
                err.contains("CM_TUI_EXPAND_TILDE=off"),
                "应给出逃生开关：{err}"
            );
        }
    }

    #[test]
    fn non_tilde_input_passes_through_unchanged() {
        assert_eq!(
            resolve_with("/data/proj", true, None),
            Ok("/data/proj".into())
        );
        assert_eq!(
            resolve_with("data/proj", true, None),
            Ok("data/proj".into())
        );
        assert_eq!(
            resolve_with("  /data/p  ", true, None),
            Ok("/data/p".into())
        );
        assert_eq!(resolve_with("", true, None), Ok(String::new()));
    }

    /// 关掉开关时 `~` 原样透传，由 serve 展开（远端部署的正确归属）。
    #[test]
    fn switch_off_keeps_tilde_for_serve() {
        assert_eq!(
            resolve_with("~/proj", false, Some(home())),
            Ok("~/proj".into())
        );
        assert_eq!(
            resolve_with("~root", false, Some(home())),
            Ok("~root".into())
        );
    }
}
