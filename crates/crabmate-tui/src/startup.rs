//! `crabmate-tui` 的启动装配：主连接从哪来、额外 lane 怎么拼、没连接时进哪块屏。
//!
//! 从 `main.rs` 拆出：那边顶着 `fn-nloc-ratchet` 的 920 行/文件上限，而这一段
//! （`--from-ctl` 解析、Bearer 传递、`--connect` 装配）与子命令分发无关 ——
//! 它只回答「这次启动连谁」，与 chat / repl 的回合逻辑没有交集。
//!
//! [`resolve_startup_api_base`] 刻意不接收 `Cli`：子命令类型留在 `main.rs`，这里只要
//! 四个事实（有没有地址、是不是 `tui`、要不要问 ctl、控制面路径），函数因此可以单测。

use anyhow::{Context, Result, bail};

use crate::SessionPrefs;
use crate::connect_spec::parse_connect_spec;
use crate::serve::{BearerSource, ConnectionConfig, ServeClient};
use crate::tui_mode;
use crate::tui_wiring::TuiWiring;
use crate::{ensure_tty, maybe_probe};

/// `tui` 启动时要带的两件事：新 lane 复用的 Bearer **值**与它的**来源**。
///
/// 收成一个结构而不是两个参数：`run_tui_cmd` 的实参已经贴着 clippy 的 `too_many_arguments`
/// 上限，而这两项本就是一件事（「连这台 serve 带什么凭据」）。
#[derive(Clone, Copy)]
pub(crate) struct TuiBearer<'a> {
    /// Web API Bearer（`None` / 空串 = 不带）。
    pub(crate) token: &'a str,
    /// 上面那份 Bearer 是哪来的（设置面板「连接」栏只报来源）。
    pub(crate) source: BearerSource,
}

pub(crate) async fn run_tui_cmd(
    client: Option<&ServeClient>,
    bearer: TuiBearer<'_>,
    no_probe: bool,
    no_keyring: bool,
    yes: bool,
    overrides: &mut SessionPrefs,
    wiring: TuiWiring<'_>,
) -> Result<()> {
    ensure_tty()?;
    // 没有主连接时无 `/health` 可探（未连接界面里的 attach 各自探自己的）。
    if let Some(client) = client {
        maybe_probe(client, no_probe).await?;
    }
    let TuiWiring {
        connect,
        ctl_socket,
    } = wiring;
    let extras = build_extra_lanes(connect, bearer)?;
    // 控制面 socket：显式 `--ctl-socket` 优先、否则默认路径存在才启用。
    let ctl_arg = ctl_socket;
    let ctl_socket = crate::serve::resolve_socket_arg(ctl_arg);
    // 无主连接 + 无 `--connect` + 无控制面：照样进未连接界面 —— 那块屏里还剩
    // `/lane attach-url`（不经 ctl 就够用）。「没地址就别启动」的判据只在
    // `resolve_startup_api_base` 里有一处（非 `tui` 子命令），这里不再重复。
    tui_mode::check_lane_capacity(extras.len() + client.iter().count())?;
    let lanes = assemble_lanes(client, &extras, ctl_arg);
    tui_mode::run_tui(
        &lanes,
        (bearer.token, bearer.source),
        overrides,
        yes,
        no_keyring,
        ctl_socket.as_deref(),
    )
    .await
}

/// `--connect name=url` 逐条构造额外 lane：与主连接共用启动时那份 Bearer。
///
/// 抽成函数只为 `run_tui_cmd` 的 CCN（`lizard` 对 >10 零容忍）；它本身没有任何分支
/// 之外的耦合 —— 地址解析、client 构造、错误直上都在这里收口。
pub(crate) fn build_extra_lanes(
    connect: &[String],
    bearer: TuiBearer<'_>,
) -> Result<Vec<(String, ServeClient)>> {
    let mut extras: Vec<(String, ServeClient)> = Vec::with_capacity(connect.len());
    for spec in connect {
        let (name, url) = parse_connect_spec(spec)?;
        let cfg =
            ConnectionConfig::new(url, bearer.token.to_string()).with_bearer_source(bearer.source);
        extras.push((name, ServeClient::new(cfg)?));
    }
    Ok(extras)
}

/// 装配 `run_tui` 的 lane 列表：主连接（**可能有、也可能没有**）在前，`--connect` 依次追加。
///
/// 主 lane 的名字从 ctl 里按 `base_url` 反查实例名（反查不到仍叫 `local`，见
/// `serve::ctl::primary_binding`）；没有主连接时列表可能为空 —— 那就是未连接界面。
pub(crate) fn assemble_lanes<'a>(
    client: Option<&'a ServeClient>,
    extras: &'a [(String, ServeClient)],
    ctl_arg: Option<&str>,
) -> Vec<(String, &'a ServeClient)> {
    let mut lanes: Vec<(String, &'a ServeClient)> = Vec::with_capacity(extras.len() + 1);
    if let Some(client) = client {
        let primary = crate::serve::primary_binding(ctl_arg, &client.config().api_base).1;
        lanes.push((primary, client));
    }
    for (name, c) in extras {
        lanes.push((name.clone(), c));
    }
    lanes
}

/// 启动时确定主连接的 `api_base`；`None` = **没有主连接**（只有 `tui` 允许）。
///
/// 三条来源按优先级：显式 `--api-base` / `CRABMATE_API_BASE` → `--from-ctl` 从控制面取
/// 第一条可连实例 → 未连接界面。
///
/// **没有控制面不是失败**：`--from-ctl` 只是「顺手替我挑一条」，而那块屏里的
/// `/lane attach-url` 本来就不经 ctl —— 单机、没装 `crabmatectl` 的机器同样得能起来
/// （否则「不设置 `--api-base` 也能启动」只在装了控制面的场景成立）。
pub(crate) fn resolve_startup_api_base(
    api_base: Option<&str>,
    is_tui: bool,
    from_ctl: bool,
    ctl_socket: Option<&str>,
) -> Result<Option<String>> {
    if let Some(base) = api_base.map(str::trim).filter(|s| !s.is_empty()) {
        // 两个都给时以 `--api-base` 为准，但要说一句：静默忽略会让人以为 `--from-ctl`
        // 生效了（他给出它，多半是想让控制面来挑）。
        if from_ctl {
            eprintln!("[crabmate-tui] --api-base 与 --from-ctl 同时给出：以 --api-base 为准");
        }
        return Ok(Some(base.to_string()));
    }
    // 只有 `tui` 允许「先起来、后连接」；其余子命令仍必须有地址。
    if !is_tui {
        bail!("missing --api-base / CRABMATE_API_BASE (absolute http(s) URL to crabmate serve)");
    }
    if from_ctl {
        match crate::serve::resolve_socket_arg(ctl_socket) {
            Some(socket) => {
                if let Some(base) = base_url_from_ctl(&socket) {
                    return Ok(Some(base));
                }
            }
            None => eprintln!(
                "[crabmate-tui] --from-ctl 需要控制面：未启用；进入未连接界面（`/lane attach-url` 仍可直连）"
            ),
        }
    }
    Ok(None)
}

/// `--from-ctl`：问一次控制面，取第一条可连实例的地址。
///
/// 取不到**不是**启动失败：控制面在、但没有就绪实例时，进未连接界面比退出去更能说明
/// 现状（`/lane` 会把每条实例的观测态与 `last_error` 摆出来）。所以两种失败都只说一句
/// 就返回 `None`。
pub(crate) fn base_url_from_ctl(socket: &std::path::Path) -> Option<String> {
    match crate::serve::list_instances(socket) {
        Ok(items) => match crate::serve::first_ready_base_url(&items) {
            Some((name, url)) => {
                eprintln!("[crabmate-tui] --from-ctl：主连接用 ctl 实例 `{name}`（{url}）");
                Some(url)
            }
            None => {
                eprintln!(
                    "[crabmate-tui] --from-ctl：控制面里没有可连实例，进入未连接界面（`/lane` 看状态）"
                );
                None
            }
        },
        Err(e) => {
            eprintln!(
                "[crabmate-tui] --from-ctl 查询失败：{}；进入未连接界面",
                e.user_message()
            );
            None
        }
    }
}

/// 构造主连接。`bearer_source` 由调用方在解析 Bearer 的同一处定下 —— 面板要如实回答
/// 「带了没、哪来的」，值本身逆推不出来。
pub(crate) fn build_client(
    api_base: &str,
    bearer: Option<&str>,
    bearer_source: BearerSource,
) -> Result<ServeClient> {
    let bearer = bearer
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    Ok(ServeClient::new(
        ConnectionConfig::new(api_base.to_string(), bearer).with_bearer_source(bearer_source),
    )?)
}

/// 除 `tui` 外的子命令都要求真有主连接：缺 `--api-base` 时 `client` 是 `None`。
///
/// 单独成函数是为了让三个调用点报同一句话：它们各自的 `bail!` 迟早会漂成三种措辞。
pub(crate) fn require_client(client: &Option<ServeClient>) -> Result<&ServeClient> {
    client.as_ref().with_context(
        || "missing --api-base / CRABMATE_API_BASE (absolute http(s) URL to crabmate serve)",
    )
}

#[cfg(test)]
mod tests {
    use super::{assemble_lanes, resolve_startup_api_base};

    /// 两个都给了：`--api-base` 赢（并打一句提示，别让 `--from-ctl` 静默失效）。
    #[test]
    fn explicit_api_base_wins_over_from_ctl() {
        let got = resolve_startup_api_base(Some("http://127.0.0.1:8080"), true, true, None)
            .expect("显式地址必然成功");
        assert_eq!(got.as_deref(), Some("http://127.0.0.1:8080"));
        // 空白值不算「给了」。
        let got = resolve_startup_api_base(Some("   "), true, false, None).expect("空态");
        assert_eq!(got, None);
    }

    /// 非 `tui` 子命令没有「未连接界面」这一档：缺地址就是失败。
    #[test]
    fn non_tui_subcommands_still_require_the_address() {
        let err = resolve_startup_api_base(None, false, false, None)
            .expect_err("chat / repl / connect 必须有地址")
            .to_string();
        assert!(err.contains("missing --api-base"), "{err}");
        // `--from-ctl` 只属于 `tui`，救不了它们。
        assert!(resolve_startup_api_base(None, false, true, Some("/tmp/x.sock")).is_err());
    }

    /// **没有控制面也能进未连接界面**：那块屏里的 `/lane attach-url` 不经 ctl。
    ///
    /// 这条是「不设置 `--api-base` 也能启动」的回归测试 —— 早先的实现要求 ctl socket
    /// 存在才允许空态，于是没装 `crabmatectl` 的机器仍然起不来。
    ///
    /// 只测 `from_ctl = false`：`true` + `ctl_socket = None` 会去读**真实的**默认 socket
    /// （本机可能正跑着 ctl，结果就不可复现了），那个形状由下面用显式死路径的用例覆盖。
    #[test]
    fn tui_without_any_address_enters_the_unconnected_screen() {
        let got =
            resolve_startup_api_base(None, true, false, None).expect("没有 ctl 不该是启动失败");
        assert_eq!(got, None);
    }

    /// `--from-ctl` 撞上一个连不上的 socket：说一句，然后进未连接界面 —— 不退出。
    #[test]
    fn from_ctl_on_a_dead_socket_falls_into_the_unconnected_screen() {
        let got = resolve_startup_api_base(None, true, true, Some("/tmp/crabmate-no-such.sock"))
            .expect("ctl 连不上不是启动失败");
        assert_eq!(got, None);
    }

    /// 没有主连接时 lane 列表就是空的 —— 那正是未连接界面的判据。
    #[test]
    fn assemble_lanes_without_a_primary_is_empty() {
        assert!(assemble_lanes(None, &[], None).is_empty());
    }
}
