//! 左栏「其它窗口」分组（对齐 TUI 之后剩下的唯一窗口级分组）。
//!
//! 单独成文件是行数门禁的要求（`nav_rail_connections.rs` 贴着 920 行），与
//! `nav_rail_lanes.rs` / `nav_rail_ctl_restart.rs` 同一处置：`#[path]` 只换文件，
//! 模块路径仍是 `rail::other_windows`。
//!
//! 单窗口多 lane 的常态下这一组**恒空、整组不渲染** —— 前端已经没有开新窗口的入口
//! （#561 S2：连接 = 在当前窗口新建 lane），只有壳侧还挂着别的 Desktop 窗口（老路径 /
//! 调试）时才有内容，那时它们仍要能切、能关。
//!
//! 抽成子组件的另一半理由是 `lizard`：`NavRailConnections` 贴着 CCN>10 门禁，而
//! `For` / `Show` 里的闭包都会被算进外层函数。

use leptos::prelude::*;

use crabmate_client_api::connections::ConnectionSnapshot;

use crate::i18n::{self, Locale};

use super::{ConnectionMenuAnchor, connection_row};

/// 左栏「其它窗口」分组。
#[component]
pub(super) fn OtherWindowsGroup(
    locale: RwSignal<Locale>,
    rows: Signal<Vec<ConnectionSnapshot>>,
    own_authority: StoredValue<String>,
    settings_page: RwSignal<bool>,
    rename_menu: RwSignal<Option<ConnectionMenuAnchor>>,
) -> impl IntoView {
    view! {
        <Show when=move || !rows.get().is_empty()>
            <div class="nav-rail-connections-label">
                {move || format!(
                    "{} [{}]",
                    i18n::other_windows_label(locale.get()),
                    rows.get().len(),
                )}
            </div>
            <For
                each=move || rows.get()
                key=|snapshot| snapshot.key.clone()
                // `multi = true`：能走到这里就说明有别的窗口，点击一律切过去
                // （「单连接时点击进设置页」那条分支属于本窗口行，已经不再渲染）。
                children=move |snapshot| connection_row(
                    snapshot,
                    own_authority.get_value(),
                    true,
                    settings_page,
                    locale,
                    rename_menu,
                )
            />
        </Show>
    }
}
