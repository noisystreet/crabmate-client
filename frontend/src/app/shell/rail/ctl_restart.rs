//! 左栏「重启」按钮的状态与提示行（`crabmatectl` 的 `restart`）。
//!
//! 单独成文件只有一个理由：`nav_rail_connections.rs` 贴着 920 行门禁（`fn-nloc`）。
//! 这块与「怎么画连接行」无关 —— 它只负责**一个同步动作**的在途标记与回执文案。
//!
//! `restart` 是**同步** op（down → up → 等就绪才回一行，最长约 50 s，见
//! `crabmate_client_api::ctl::RESTART_BUDGET_SECS`），所以这里三件事都是必需的：
//! 按下即置在途（否则 50 s 里看起来就是「点了没反应」）、按钮在途中禁用（再点一次等于
//! 把一条正在起的实例再打一次）、回执把 Server 的 `notes` 摆出来（`env` 丢了 / profile
//! 漂移只有它知道）。

use leptos::prelude::*;

use crabmate_client_api::ctl::RestartOutcome;

use crate::i18n::{self, Locale};
use crate::tauri_shell::tauri_ctl_restart_instance_json;

/// 「重启」按钮的界面状态（一条信号带两件事）。
///
/// `busy` 存**名字**而不是 `bool`：ctl 只有一个线程，同一时刻只可能有一条重启在跑，
/// 而用户需要看见「正在重启哪一条」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CtlRestartUi {
    /// 在途那条实例的名字（`None` = 没有在跑的重启）。
    pub busy: Option<String>,
    /// 最近一次重启的结果；`None` = 还没试过。
    pub notice: Option<CtlRestartNotice>,
}

/// 一次重启的结果提示（画在左栏分组标题下面那一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CtlRestartNotice {
    /// 被重启的实例名（提示文本里也带，这里留着是为了后续定位）。
    pub name: String,
    /// 面向用户的一行：结论 + 新 pid + Server 的 `notes`。
    pub text: String,
    /// `true` = 失败（用错误色）；成功但带 `notes` 仍是正常色。
    pub error: bool,
}

/// 重启回执 → 提示（一句结论 + 新 pid + Server 的 `notes`，逐条接在后面）。
///
/// `notes` 是「env 丢了 / profile 漂移 / 声明其实 `enabled = false`」的**唯一**出处：
/// 不摆出来，重启看起来成功、行为却和预期不一样。`restarted = false` 也不写成「已重启」
/// —— 那是 Server 的说法，别替它加戏。
#[must_use]
pub(super) fn restart_notice(outcome: &RestartOutcome, locale: Locale) -> CtlRestartNotice {
    let name = outcome.instance.name.clone();
    let mut text = if outcome.restarted {
        i18n::ctl_restart_done(locale, &name)
    } else {
        i18n::ctl_restart_unchanged(locale, &name)
    };
    if let Some(pid) = outcome.instance.pid {
        text.push_str(&format!("（pid {pid}）"));
    }
    for note in &outcome.notes {
        text.push_str(" · ");
        text.push_str(note);
    }
    CtlRestartNotice {
        name,
        text,
        error: false,
    }
}

/// 调壳重启一条实例，把回执折成一行提示（失败也走这里，带 Server 的 `code`）。
///
/// 非桌面壳里壳命令不存在，`tauri_ctl_restart_instance_json` 直接回 `Err` —— 那种壳
/// 也渲染不到这个按钮（`tauri_shell_available()` 为假时整块不画）。
pub(super) async fn run_ctl_restart(name: &str, locale: Locale) -> CtlRestartNotice {
    let failed = |reason: String| CtlRestartNotice {
        name: name.to_string(),
        text: i18n::ctl_restart_failed(locale, name, &reason),
        error: true,
    };
    let json = match tauri_ctl_restart_instance_json(name).await {
        Ok(json) => json,
        Err(reason) => return failed(reason),
    };
    match serde_json::from_str::<RestartOutcome>(&json) {
        Ok(outcome) => restart_notice(&outcome, locale),
        Err(_) => CtlRestartNotice {
            name: name.to_string(),
            text: i18n::ctl_restart_bad_reply(locale).to_string(),
            error: true,
        },
    }
}

/// 重启结果的提示行：一句结论 + 新 pid + Server 的 `notes`，失败用错误色。
///
/// 单独成组件只为压 `lizard` 的 CCN：`view!` 里的分支全算在承载它的函数头上，而
/// `NavRailConnections` 本来就在 10 上下。没有结果时整行不画（不占位）。
#[component]
pub(super) fn CtlRestartNoticeLine(restart: RwSignal<CtlRestartUi>) -> impl IntoView {
    let text = Memo::new(move |_| restart.get().notice.map(|n| n.text).unwrap_or_default());
    let error = Memo::new(move |_| restart.get().notice.is_some_and(|n| n.error));
    view! {
        <Show when=move || !text.get().is_empty()>
            <p
                class="nav-rail-ctl-notice"
                class:nav-rail-ctl-notice--error=move || error.get()
                data-testid="nav-rail-ctl-notice"
            >
                {move || text.get()}
            </p>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::restart_notice;
    use crate::i18n::Locale;
    use crabmate_client_api::ctl::{CtlInstance, RestartOutcome};

    fn outcome(restarted: bool, pid: Option<u32>, notes: &[&str]) -> RestartOutcome {
        RestartOutcome {
            instance: CtlInstance {
                name: "prod".to_string(),
                pid,
                ..CtlInstance::default()
            },
            restarted,
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

    /// 重启提示必须带上 `notes`：那是「env 丢了 / profile 漂移」唯一能露面的地方。
    #[test]
    fn a_restart_notice_carries_the_server_notes() {
        let notice = restart_notice(
            &outcome(true, Some(4243), &["声明里的 env 已丢失（3 项）"]),
            Locale::ZhHans,
        );
        assert_eq!(notice.name, "prod");
        assert!(!notice.error, "成功且带 notes 仍是正常色");
        assert!(notice.text.contains("prod"), "{}", notice.text);
        assert!(notice.text.contains("4243"), "{}", notice.text);
        assert!(
            notice.text.contains("env 已丢失"),
            "notes 不能吞掉：{}",
            notice.text
        );
    }

    /// `restarted = false` 不写成「已重启」：那是 Server 的说法，别替它加戏。
    #[test]
    fn a_non_restart_reads_as_unchanged() {
        let notice = restart_notice(&outcome(false, None, &[]), Locale::En);
        assert!(notice.text.contains("not restarted"), "{}", notice.text);
        assert!(!notice.error, "「没重启」不是失败");
    }
}
