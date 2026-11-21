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

    // #561 阶段 3：第一刀把「当前 lane 的运行时」收进容器 —— 先建信号，跑完接线，
    // 再把信号 + 接线产物 + 聊天列壳**一起**交给容器（每条 lane 一份）。
    let window = super::app_signals::WindowSignals::new();
    let app_signals: AppSignals =
        AppSignals::new_lane(&window, crate::api::connection_scope::active_holder());
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
    let wiring = run_shell_wiring_in_order(&app_signals);

    // 接线产物与聊天列壳按 lane 存进容器（组装表达式见 `LaneRuntime::new`）。
    let lanes = super::app_signals::LaneSet::new_single(app_signals, wiring);

    AppShellCtx { lanes }
}
