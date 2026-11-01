//! 状态栈「连接」chip：这一屏的数据来自哪台 `serve`。
//!
//! 对齐 TUI 的 lane 行（`docs/design/tui_multi_server.md` §4.6）：TUI 在**单连接**时
//! 也照画 lane 区，理由是「不显示的话，用户既看不出自己连的是哪台 serve，也看不出
//! 这客户端支持多个连接」。Desktop / Android / Web 共用同一份 WASM，所以放在状态栈里
//! 一次三端受益——这也是本仓唯一能「对齐 TUI 显示」而不先做多连接的部分。
//!
//! 三个刻意的取舍：
//!
//! 1. **只显示 `host:port`**：状态栈每个 chip 只占**一行**（`.status-chips` 纵向
//!    堆叠、各自撑满右列宽度），一个长 FQDN 会在这行内被省略号截掉，读不出后两段
//!    IP 的差别。完整基址进 `title`，正文本只留能区分两台 serve 的那一段——端口必须留，
//!    同机两个 serve 只靠它区分。
//! 2. **空基址整段不渲染**：空 = 同 Origin（WASM 的默认态），此时没有「哪台 serve」
//!    可讲；画一个 `—` 只会让人以为连接坏了。与 TUI 的 `ctx` 段占位取舍相反，因为那里
//!    的 `—` 表示「还没测到」，而这里表示「本来就没有这个值」。
//! 3. **不放窄屏**：`mobile.css` 的 `html[data-narrow-viewport]` 已把非 role / context
//!    的 chip 全部隐藏，本 chip 自动让位。这与 TUI 的 `lanes` 段同一条纪律——连接身份
//!    是「想知道时能查到」的信息，不是每回合都要读的东西，不该和 `ctx` 抢宽度。
//!    窄屏下它仍可在「设置 → 连接」看到。
//!
//! 点击跳「设置 → 连接」：这是唯一能改基址的地方，且用户此刻想问的往往正是
//! 「我想换一台」——把它做成可点而不是纯标签。
//!
//! **与 TUI 的 `serve` 段是同一件事**（`crates/crabmate-tui/src/tui_mode/status.rs`
//! 的 `serve_value`）：两端都回答「这一屏的数据来自哪台 serve」，都经
//! `api_base_authority()` 剥掉 scheme / 路径 / userinfo。差别只有一处，且是**刻意的**：
//! 终端没有 hover，`title` 那套用不了，所以 TUI 在列宽够时显示**完整** base（那是它
//! 唯一能看到全量 URL 的地方），真放不下才退 `host:port`；这里恒为 `host:port`，
//! 完整 URL 交给 `title`。**别把其中一边「统一」成另一边** —— 拉平会丢掉对应平台
//! 唯一的那项能力：TUI 失去全量 URL，本 chip 失去悬停可读的完整地址。

use leptos::prelude::*;

use crabmate_client_api::url::api_base_authority;

use crate::api::api_base_url;
use crate::i18n::{self, Locale};

use super::settings_page::{SettingsSection, navigate_to_settings};
use super::shell_runtime_context::try_chat_shell_ctx;

/// chip 上显示什么：`None` = 整段不渲染。
///
/// 抽成纯函数而不是在闭包里现算，是为了能在非 WASM 目标下测「基址 → 短文本」
/// 这一步（它不碰任何信号）。
#[must_use]
fn connection_chip_text(base: &str) -> Option<String> {
    let authority = api_base_authority(base);
    if authority.is_empty() {
        None
    } else {
        Some(authority)
    }
}

#[component]
pub fn StatusConnectionChip(
    locale: RwSignal<Locale>,
    settings_page: RwSignal<bool>,
) -> impl IntoView {
    // 缺 ctx（单测 / 独立子树）时退回「读一次 thread_local」：没有上下文不该让整条
    // 状态栈 panic，但那种情形下也不会有别的写入点，读一次就是终值。
    let api_base = try_chat_shell_ctx()
        .map(|c| c.api_base)
        .unwrap_or_else(|| RwSignal::new(api_base_url()));

    let text = Memo::new(move |_| connection_chip_text(&api_base.get()));
    let title = Memo::new(move |_| {
        i18n::status_chip_connection_tooltip(locale.get(), api_base.get().as_str())
    });

    view! {
        <Show when=move || text.get().is_some()>
            <button
                type="button"
                class="status-chip status-chip-connection"
                data-testid="status-connection-chip"
                prop:title=move || title.get()
                on:click=move |_| {
                    navigate_to_settings(settings_page, SettingsSection::Connection);
                }
            >
                <span class="status-chip-label">
                    {move || i18n::status_chip_connection(locale.get())}
                </span>
                <span class="status-chip-value">{move || text.get().unwrap_or_default()}</span>
            </button>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::connection_chip_text;

    #[test]
    fn host_and_port_survive_but_scheme_and_path_do_not() {
        // 端口必须留：同机两个 serve 只靠它区分。
        assert_eq!(
            connection_chip_text("http://127.0.0.1:8081"),
            Some("127.0.0.1:8081".to_string())
        );
        assert_eq!(
            connection_chip_text("https://serve.example.com:8443/api"),
            Some("serve.example.com:8443".to_string())
        );
    }

    #[test]
    fn same_origin_renders_nothing_rather_than_a_placeholder() {
        // 空基址 = 同 Origin，没有「哪台 serve」可讲。
        assert_eq!(connection_chip_text(""), None);
        assert_eq!(connection_chip_text("   "), None);
        // 认不出的基址同样不渲染，而不是把原件当地址画出来。
        assert_eq!(connection_chip_text("not-a-url"), None);
    }
}
