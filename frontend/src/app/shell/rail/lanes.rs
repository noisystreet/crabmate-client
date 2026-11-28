//! 左栏「本窗口 lane」分组（#561 S3b / #569 对齐 TUI 的 lane 行）。
//!
//! 单独成文件是行数门禁的要求（`nav_rail_connections.rs` 贴着 920 行），与
//! `nav_rail_ctl_restart.rs` 同一处置：`#[path]` 只换文件，模块路径仍是
//! `rail::lanes`。

use leptos::prelude::*;

use crabmate_client_api::connections::ConnectionStatus;
use crabmate_client_api::url::api_base_authority;

use crate::i18n::{self, Locale};

use super::status_glyph;

/// 左栏「本窗口 lane」分组（#561 S3b，§6.5 方案 B；#569 对齐 TUI 的 lane 行）。
///
/// 显示与操作口径对齐 TUI 的 `lane_list`（`docs/design/tui_multi_server.md` §4.6）：
///
/// - **单 lane 也显示** —— 不显示就看不出这条 lane 连的是哪台 serve；
/// - 标题带 `[n/8]` 与 `▲k`（有 lane 在等审批时），数字口径来自共享层
///   `lane_row::count_suffix`，与 TUI 同一份；
/// - 每行 = 状态点（`·●▲×`）+ **名字**（空则回退基址 /「同源」）+ 行尾读数；
/// - 行尾读数（上下文占比 / 在途耗时 / 未读点）用共享层 `lane_row::tail_parts`，顺序与降级
///   规则同 TUI。耗时读**本行自己的起点**、由窗口级心跳推进（#577）—— 所以切到别的 lane
///   之后，原来那条的秒数继续走（它仍在跑，切走不等于停）；
///
/// 地址不进行内、只进 `title`（TUI 同样只写名字：窄条里地址会把名字挤掉）。
/// 仍只列**本窗口**的 lane —— 别的窗口有几条看不到（方案 B 的既定降级，见能力矩阵注 3）。
#[component]
pub(super) fn LaneRailGroup(locale: RwSignal<Locale>) -> impl IntoView {
    // 数据源与切换动作都走窗口级表：行读数是 `RwSignal`（`Copy`/`Send + Sync`），
    // 能进响应式闭包；点击走 thread_local 函数，闭包里不捕获 `Rc`（§5.2 的那道边界）。
    let rail = crate::app::lane_actions::rail();
    view! {
        {move || {
            let Some((rows, active, now)) = rail else {
                return ().into_any();
            };
            let all = rows.get();
            if all.is_empty() {
                return ().into_any();
            }
            let waiting = all
                .iter()
                .filter(|row| row.status.get() == ConnectionStatus::WaitingApproval)
                .count();
            view! {
                <div class="nav-rail-ctl-group" data-testid="nav-rail-lane-group">
                    <div class="nav-rail-ctl-head">
                        <span class="nav-rail-ctl-title">
                            {format!(
                                "{} {}",
                                i18n::lane_rail_title(locale.get()),
                                crabmate_client_api::lane_row::count_suffix(
                                    all.len(),
                                    crabmate_client_api::lane_attach::MAX_LANES,
                                    0,
                                    waiting,
                                ),
                            )}
                        </span>
                    </div>
                    {(0..all.len())
                        .map(|index| lane_rail_row(index, all[index], active, now, locale))
                        .collect_view()}
                </div>
            }
            .into_any()
        }}
    }
}

/// 一行 lane：状态点 + 名字 + 行尾读数。点击切过去，前台那条高亮。
fn lane_rail_row(
    index: usize,
    row: crate::app::app_signals::LaneRailRow,
    active: RwSignal<usize>,
    now: RwSignal<f64>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    // 名字优先：空名回退 `host:port`，基址也空（同 Origin）才说「同源」——
    // 与左栏连接行 `connection_display_name` 同一口径。
    let name = move || {
        let name = row.name.get();
        if !name.trim().is_empty() {
            return name;
        }
        let authority = api_base_authority(&row.api_base.get());
        if authority.is_empty() {
            i18n::status_chip_same_origin(locale.get()).to_string()
        } else {
            authority
        }
    };
    // 行尾读数：顺序与降级规则在共享层（TUI 同一份）。浏览器不做宽度裁剪，CSS 自己截。
    // 耗时读本行的起点（`now` 由窗口级心跳推进）：切走不等于停，后台那条也在走。
    let tail = move || {
        let context = row.context.get();
        crabmate_client_api::lane_row::tail_parts(crabmate_client_api::lane_row::LaneReadout {
            context: context.as_deref(),
            busy_secs: row.busy_secs(now.get()),
            unread: row.unread.get(),
        })
        .join(" ")
    };
    view! {
        <button
            type="button"
            class="nav-rail-connection-row"
            class:nav-rail-connection-row--active=move || active.get() == index
            prop:title=move || row.api_base.get()
            data-testid="nav-rail-lane-row"
            on:click=move |_| crate::app::lane_actions::switch_to(index)
        >
            <span
                class="nav-rail-connection-glyph"
                class:nav-rail-connection-glyph--waiting=move || {
                    row.status.get() == ConnectionStatus::WaitingApproval
                }
                aria-hidden="true"
            >
                {move || status_glyph(row.status.get())}
            </span>
            <span class="nav-rail-connection-name">{name}</span>
            <span class="nav-rail-connection-meta">{tail}</span>
        </button>
    }
}
