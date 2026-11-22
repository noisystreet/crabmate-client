//! App 壳层接线的**单一入口**：[`bootstrap_app_shell`] 构造 [`AppSignals`] 后调用
//! [`run_shell_wiring_in_order`](super::app_shell_wire_phases::run_shell_wiring_in_order)，
//! 再组装 [`AppShellCtx`]。阶段语义见 [`super::app_shell_wire_phases`]；聊天主列内部顺序见 [`super::chat::wire_chat_domain`]。

use super::app_shell_ctx::AppShellCtx;
use super::app_shell_wire_phases::run_shell_wiring_in_order;
use super::app_signals::AppSignals;

/// 执行所有 `wire_*` 注册、闭包构建与 [`AppShellCtx`] 组装（推荐新代码直接调用本函数）。
pub fn bootstrap_app_shell() -> AppShellCtx {
    // #558：先把「当前 lane 的 scope 持有者」立起来 —— 值住在这个持有者里，不再直接
    // 躺在 `thread_local` 中。启动从「未连接」开始，随后由水合 / handoff 填值。
    crate::api::connection_scope::install_new_holder();
    // 移动薄客户端：先消费 URL hash 中的 Web API Bearer，再挂接会打 `/status` 的 Effect。
    crate::api::consume_mobile_connect_handoff();
    // Android 壳：尽早写入 --cm-safe-top，避免顶栏贴系统状态栏难点选。
    crate::mobile_remote::apply_mobile_remote_safe_top();
    // 仅移动远程壳需要在此安装外链桥；桌面 Tauri 仍由 frameless chrome 安装。
    if crate::mobile_remote::mobile_remote_client() {
        crate::tauri_shell::ensure_external_link_handler();
    }

    // 应用级 signal 单例（连接名单 / 已登记标记 / 可连实例）必须**在这里**建：此刻 owner 是
    // `App` 组件（稳定、与页面同寿），而它们的首次读取发生在 `connection_report` 的 memo 里 ——
    // memo / effect 的 computation 作用域 run 完就 dispose 自己建出来的节点，那份「全局」signal
    // 会建完即死，之后每次读都 panic（wasm 上 panic 是 trap，连带 `js-sys` 任务永久卡住借用，
    // 整屏「点了没反应」）。详见 `connection_feed::init_app_signal_singletons`。
    //
    // 位置放在 handoff 之后：注册连接要用这里已经落到 scope 的基址。
    super::connection_feed::init_app_signal_singletons();

    // #561 阶段 3：第一刀把「当前 lane 的运行时」收进容器 —— 先建信号，跑完接线，
    // 再把信号 + 接线产物 + 聊天列壳**一起**交给容器（每条 lane 一份）。
    let window = super::app_signals::WindowSignals::new();
    let app_signals: AppSignals =
        AppSignals::new_lane(&window, crate::api::connection_scope::active_holder());
    // 左栏那一行要在接线之前建好（phase 5 会往里写读数，#569）：
    // 名字留空（行上回退基址），基址取这条 lane 的 scope（与请求实际用的同一份）。
    let first_row = super::app_signals::LaneRailRow::new(None, app_signals.scope.api_base());
    // 官方壳：尽早从钥匙串 / Keystore 水合（hash 已写入内存时仅清明文 LS）。
    //
    // #558 项 1b：水合显式走「**这条 lane 自己的** holder」，不再靠零参口回落到 `ACTIVE`。
    // 单 lane 时两者是同一份（`AppSignals::new()` 刚取的就是 `install_new_holder()` 装的那份），
    // 行为与改动前一致；多 lane 时每条 lane 各自水合自己那份，不会把凭据写进别人的 scope。
    let scope = app_signals.scope.clone();
    leptos::task::spawn_local(async move {
        crate::api::web_api_bearer_local::hydrate_web_api_bearer_into(&scope).await;
    });
    crate::confirm_dialog::register_shell_confirm(app_signals.modal.confirm_signals());
    let wiring = run_shell_wiring_in_order(&app_signals, first_row);

    // 接线产物与聊天列壳按 lane 存进容器（组装表达式见 `LaneRuntime::new`）。
    let lanes = super::app_signals::LaneSet::new_single(window, first_row, app_signals, wiring);

    AppShellCtx { lanes }
}
