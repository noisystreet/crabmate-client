//! 主界面：单根 `App`（导航、对话、侧栏、状态栏、模态框与偏好接线）。
//!
//! 首启会话加载、`/user-data` / DOM 偏好同步、全局 `Escape` 等壳级副作用见 `app_shell_effects`。聊天主路径（滚动、查找、输入/流式）见 `chat` 子模块；Workspace / 变更集、`/status` 与任务等接线已部分迁至 **`wire_workspace_domain`**、**`status_tasks_wiring`**、**`chat::wire_chat_session_lifecycle`**（阶段 B）。

mod app_body;
mod app_bootstrap_phase;
mod app_shell_bootstrap;
mod app_shell_ctx;
pub(crate) mod app_shell_effects;
mod app_shell_init;
mod app_shell_wire_phases;
pub(crate) mod app_signals;
mod approval_modal;
mod changelist_modal;
mod chat;
pub(crate) use chat::turn_lifecycle;
mod connection_feed;
mod connectivity_recovery;
mod context_usage_page;
pub(crate) mod focusable_menu;
mod git_badge;
mod github_embed_page;
mod github_wiring;
mod ide_confirm_dialog;
mod ide_editor_body;
mod ide_editor_pane;
mod ide_find_bar;
mod ide_layout_switch;
mod ide_menu_bar;
mod ide_new_file_modal;
mod ide_session;
mod ide_settings_page;
mod ide_tabs_bar;
pub(crate) mod lane_actions;
mod lane_hotkeys;
mod lane_view;
mod layout_toggle_hotkey;
mod mobile_shell_header;
mod nav_rail_connections;
mod session_list_modal;
pub(crate) mod session_mode_defaults;
mod settings;
mod shell_confirm_dialog;
pub(crate) mod shell_prefs_storage;
mod shell_runtime_context;
mod side_column;
mod side_column_toolbar;
mod side_column_workspace_scroll;
mod sidebar_nav;
mod status_agent_role_menu;
mod status_bar;
mod status_context;
mod status_fetch_state;
mod status_permission_chip;
mod status_session_mode_seg;
mod status_tasks_state;
mod status_tasks_wiring;
mod status_view_chip;
mod tauri_window_controls;
mod web_api_bearer_recovery;
mod wire_workspace_domain;
mod workspace_browser_pick_modal;
mod workspace_clone_modal;
mod workspace_panel;
pub(crate) mod workspace_panel_state;
mod workspace_project_modal;
mod workspace_project_modal_body;
mod workspace_project_modal_parts;
mod workspace_root_actions;

use std::rc::Rc;

use crate::i18n::{self, Locale};
use crate::icon::icon_chevron_right;
use app_body::app_body;
use app_shell_ctx::AppShellCtx;
use app_shell_init::init_app_shell;
use ide_layout_switch::IdeLayoutToggleSignals;
use lane_hotkeys::wire_lane_hotkeys;
use lane_view::LaneSlots;
use layout_toggle_hotkey::wire_layout_toggle_hotkey;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// 根容器类名：随侧栏折叠 / 贴边变化（集中计算，减少 App 内 reactive 闭包数）。
///
/// 编辑器不再挂在根容器上：它只替换聊天列主区（与 TUI `/editor` 同一位置口径），
/// 侧栏与右列照旧，所以没有 `app-root--ide-layout` 这一档，也没有「切换视图时抑制
/// 侧栏宽度过渡」的 `sidebar-rail-snap` 了。
fn app_root_class_name(collapsed: bool) -> String {
    let mut c = String::from("app-root app-shell-ds");
    if collapsed {
        c.push_str(" sidebar-rail-collapsed");
    }
    c
}

/// 主区是否被全屏页遮挡：设置 / IDE 设置 / 上下文用量三页任一打开即隐藏（集中计算，减少 App 内 reactive 闭包数）。
fn shell_main_hidden(
    settings_page: RwSignal<bool>,
    ide_settings_page: RwSignal<bool>,
    context_usage_page: RwSignal<bool>,
) -> bool {
    settings_page.get() || ide_settings_page.get() || context_usage_page.get()
}

#[component]
fn SidebarRailRevealBtn(
    sidebar_rail_collapsed: RwSignal<bool>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    view! {
        <Show when=move || sidebar_rail_collapsed.get()>
            <button
                type="button"
                class="btn btn-secondary sidebar-rail-reveal-btn"
                prop:aria-label=move || i18n::nav_sidebar_expand_aria(locale.get())
                on:click=move |_| sidebar_rail_collapsed.set(false)
            >
                {icon_chevron_right("")}
            </button>
        </Show>
    }
}

#[component]
pub fn App() -> impl IntoView {
    let app_ctx = init_app_shell();
    // #561 S1b-3：`App` 只做「取当前前台那条 lane 的运行时 → 交给视图主体」。视图与
    // `provide_context` 都在 `app_body` 里，所以 `mount_to` 挂第二条 lane 时 owner 就是新子树
    // 的，ctx 不会串到第一条（§5.2「已知风险」）。
    //
    // #561 S2/S3：`App` **长期持有** `app_ctx`（事件回调靠它建新 lane），并持有 lane 的挂载
    // 容器。容器里一条 lane 一个 `.lane-slot`；切换只换 class，不重建子树（`lane_view.rs`）。
    let lanes_host = NodeRef::<leptos::html::Div>::new();
    let lane0_slot = NodeRef::<leptos::html::Div>::new();
    let slots = LaneSlots::new();
    {
        let slots = slots.clone();
        let active = app_ctx.lanes.active_signal();
        // 前台下标一变就重挂 class。`lane0_slot.get()` 是响应式读：lane 0 的 slot 挂上之后
        // 这条 Effect 会再跑一次，把它登记进 `slots`。
        Effect::new(move |_| {
            if let Some(el) = lane0_slot.get() {
                slots.set_slot(0, el.unchecked_into());
            }
            slots.apply_active(active.get());
        });
    }
    let rt = app_ctx.current_runtime();
    // #561 S3b：左栏「本窗口 lane」列表的数据源 —— 每条 lane 的地址（空 = 同 Origin）与当前
    // 前台下标。两个都是 `RwSignal`（`Copy` + `Send + Sync`），能进左栏的响应式闭包；
    // 点击切换走 `lane_actions::switch_to`（thread_local，零捕获）。
    {
        let lanes = app_ctx.lanes.clone();
        lane_actions::register_rail(
            app_ctx.lanes.rail_rows(),
            app_ctx.lanes.active_signal(),
            app_ctx.lanes.busy_now(),
            Rc::new(move |index: usize| {
                lanes.set_active(index);
            }),
        );
    }
    // 左栏行尾「在途耗时」的窗口级心跳（#577）：**只在这里起一次**。放进 lane 接线里会变成
    // 每条 lane 一个定时器（8 条 = 每秒醒 8 次），而它们读的是同一个刻度。
    app_shell_effects::wire_lane_busy_clock(app_ctx.lanes.rail_rows(), app_ctx.lanes.busy_now());
    register_new_lane_action(&app_ctx, lanes_host, slots.clone());
    wire_lane_hotkeys(app_ctx.lanes.clone());
    // 热键是**窗口级**的：只在这里注册一次。放进 `app_body` 会让每条 lane 各注册一份全局
    // 监听 —— 第二条 lane 建出来后，一次按键就 toggle 两次。
    wire_layout_toggle_hotkey(IdeLayoutToggleSignals::from_app_signals(&rt.signals));

    view! {
        <div class="lane-layer" node_ref=lanes_host>
            <div class="lane-slot lane-slot--active" data-lane="0" node_ref=lane0_slot>
                {app_body(rt)}
            </div>
        </div>
    }
    .into_any()
}

/// 把「新建一条 lane」注册进窗口级动作表（#561 S2）。
///
/// 闭包由**事件回调**触发，那时没有「这条 lane 的 owner」；所以真正的构造（含
/// `run_shell_wiring_in_order` 里那约 29 个 `Effect::new`）放进 `mount_to` 的闭包里跑 ——
/// 这是唯一会给新子树建 owner 的地方，也是 `provide_context` 能按 lane 生效的原因。
///
/// # 新 lane 的 owner 必须挂在 **`App` 的 owner** 下（实测踩过）
///
/// `mount_to` 内部用 `Owner::new()`，而它的文档写明「新 owner **注册为当前 owner 的子节点**」。
/// 事件回调执行时的当前 owner 是**触发这次点击的那条 DOM 分支**（`For` / `Show` 的一支），
/// 它在本帧稍后完全可能被 cleanup —— 一旦被 cleanup，新 lane 的整棵子树（信号 + 全部接线
/// `Effect`）会作为它的子孙被连带 dispose。症状是：建出第二条 lane 的那一刻起成串
/// `Tried to access a reactive value that has already been disposed`，wasm32 上 panic 是 trap
/// （不是 unwind，`Drop` 全跳过），`js-sys` 任务的 `RefCell` 借用不还 → `spawn_local` 那条链
/// 一起死 → 之后**点什么都**没反应（切 lane、点「项目」菜单、点会话都不动）。
///
/// 所以这里先把 `App` 的 owner 抓下来，挂载时在它之下跑：既与页面同寿，又能照原样继承
/// `App` 那一层提供的 context（换成 `Owner::new_root` 会断掉这条链）。
///
/// 新 lane 先挂进一个**自己**的 `.lane-slot`（默认隐藏）。它建出来时不是前台，所以不抢
/// `lane-slot--active`；切过去由 `Effect` 把 class 挪过去。
fn register_new_lane_action(
    app_ctx: &AppShellCtx,
    layer: NodeRef<leptos::html::Div>,
    slots: LaneSlots,
) {
    let ctx = app_ctx.clone();
    // 此刻的当前 owner 就是 `App` 组件自己的 owner（由 `App` 的 body 调用过来）。
    let root_owner = Owner::current();
    lane_actions::register_new_lane(Rc::new(
        move |name: Option<String>, base: Option<String>| {
            // #561 决策「一台 serve 只占一条 lane」：目标基址已经有一条 lane 就**切过去**，
            // 不再建第二条。少了这道判重，设置页那颗「新建 lane」按钮连点两次会得到两条同
            // Origin 的 lane —— 两条打同一台 serve、会话桶与凭据槽位都一样，用户看到的是
            // 「切了没反应」，排查时很容易误判成作用域又串了（#569 那个症状）。
            //
            // 基址缺省（`None`）= 沿用当前基址，与 `add_lane` 侧的兜底口径一致。
            let target_base = base
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map_or_else(crate::api::api_base_url, str::to_string);
            if let Some(index) = ctx.lanes.index_of_base(&target_base) {
                // index 来自同一份 `rail_rows` 快照，越界不可能。真越界说明容器与行不一致，
                // 那时新建一条只会让状态更歪 —— 所以两种情况都直接返回，但要在 debug 下叫出来。
                let switched = ctx.lanes.set_active(index);
                debug_assert!(switched, "命中 lane 下标应当可切换");
                return;
            }
            // 上限与 TUI 同源（`MAX_LANES = 8`，`Alt+1..8` 也只到 8）：满了就不再建 —— 每条 lane
            // 都带一整套接线与后台轮询，「点几次就堆几条」的代价没人想要。
            if ctx.lanes.is_full() {
                return;
            }
            let Some(layer_el) = layer.get() else {
                // 视图还没挂上。动作只可能由页面里的按钮触发，理论上到不了这里。
                return;
            };
            // 下标先取：`create_slot` 与 `add_lane` 必须落在同一个 index 上。
            let index = ctx.lanes.len();
            let Some(slot) = slots.create_slot(&layer_el.unchecked_into(), index) else {
                return;
            };
            let ctx = ctx.clone();
            let mount = move || {
                leptos::mount::mount_to(slot, move || {
                    // 行数据由 `add_lane` 一并推进容器（名字 + 基址），左栏立刻能看到这一条。
                    let rt = ctx.add_lane(name.as_deref(), base.as_deref());
                    app_body(rt)
                })
            };
            // 在 `App` 的 owner 下挂载 —— 不是在「当前 owner（点击所在的 DOM 分支）」下。
            let handle = match root_owner.clone() {
                Some(owner) => owner.with(mount),
                None => mount(),
            };
            // 还没有「关 lane」入口，先放弃句柄（与 `mount_to_body` 同一取舍）。关 lane 落地时
            // 改成把 handle 存进 `LaneSet`，关闭时 drop 以清理那条 lane 的反应式系统。
            handle.forget();
        },
    ));
}
