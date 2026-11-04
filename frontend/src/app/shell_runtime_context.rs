//! Leptos [`provide_context`]：向子树提供**可 `Copy`** 的聊天域句柄，减轻与 [`AppSignals`](super::app_signals::AppSignals) 同源的长 props 传递。
//!
//! 全量壳层仍用 [`super::app_shell_ctx::AppShellCtx`]（含 `Rc` / `StoredValue` 等）；此处仅收敛满足
//! `Send + Sync + 'static` 的聊天切片。典型消费方：**`sidebar_nav_view`**、**`session_list_modal_view`**、
//! **`side_status_stack_view`**、**`SessionContextMenuLayer`**、**[`crate::app::chat::find_bar::ChatFindBar`]**（见各自模块）。
//!
//! 新增子树消费方优先 [`expect_chat_shell_ctx`]；单测或 Story 可在挂载前 [`provide_context`]，或改用
//! [`try_chat_shell_ctx`] 做缺失分支。

use leptos::prelude::*;

use crate::app::app_signals::{AppSignals, ChatComposerSignals};
use crate::app::ide_layout_switch::IdeLayoutToggleSignals;
use crate::app::turn_lifecycle::turn_lifecycle_stream_turn_busy;
use crate::chat_session_state::ChatSessionSignals;

/// 聊天域：会话 / 作曲器、界面语言与助手正文过滤（与 [`AppSignals`] 内对应字段同源）。
#[derive(Clone, Copy)]
pub struct ChatShellLeptosContext {
    pub chat: ChatSessionSignals,
    pub composer: ChatComposerSignals,
    pub locale: RwSignal<crate::i18n::Locale>,
    pub apply_assistant_display_filters: RwSignal<bool>,
    pub show_turn_context_inject: RwSignal<bool>,
    /// 权限等级意图（与 [`AppSignals`] 的 `llm_settings.selected_permission_level` 同源）。
    ///
    /// 放在 ctx 里是为了让「设置页选择器」与「聊天发送路径」读**同一个**信号：
    /// 两处各持一份副本，就会出现「设置页选了 workspace、发出去还是旧档」。
    pub selected_permission_level: RwSignal<Option<String>>,
    /// 该意图归属哪个本地会话（切会话时据此清空，见 `llm_settings` 的同名字段）。
    pub permission_intent_owner: RwSignal<Option<String>>,
    /// 当前这一屏连的 `serve` 基址（与 [`AppSignals::shell_ui`] 的 `api_base` 同源）。
    ///
    /// 放进 ctx 而非各组件自读 thread_local：状态栈的「连接」chip 与设置页的
    /// 「连接」区块必须看到**同一个**值，否则改完基址回到聊天仍是旧 host。
    pub api_base: RwSignal<String>,
    /// 单轮 `/chat/stream` 是否在途（与 `ChatStreamBusyMemos::stream_turn_busy_ui`
    /// 同一谓词、同一输入，见 [`Self::from_app_signals`]）。
    ///
    /// 权限等级的两个写入入口（底栏 chip 与设置页选择器）用它守卫：回合进行中不换档，
    /// 因为本轮 `/chat` 早已带着开跑时的档位发出去了，飞行中的工具不该在一轮内看到两个档位。
    /// 与 TUI 的 `permission_intent::apply_permission_level` 是同一条守卫。
    pub perm_turn_in_flight: Memo<bool>,
    /// 对话 / 编辑器主区切换（同源 `AppSignals::shell_ui.editor_layout_mode`）。
    ///
    /// 状态栈的 `view` chip 与 TUI 的 `view` 段是同一个开关：读它决定显示 `chat` / `file`，
    /// 写它切换主区。走 ctx 而不是再穿一层 props —— 状态栈挂在右列深处，
    /// 与 `side_status_stack_view` 里已有的 ctx 消费方同一口径。
    pub layout_toggle: IdeLayoutToggleSignals,
}

impl ChatShellLeptosContext {
    #[must_use]
    pub fn from_app_signals(app: &AppSignals) -> Self {
        // 「回合在途」：与 `ChatStreamBusyMemos::stream_turn_busy_ui`
        // （`chat_session_state::make_chat_stream_busy_memos`）**同一谓词、同一输入**。
        // 两处各建一个 memo 而不是共享同一个对象：busy memos 在聊天域 wire 之后才有，
        // 而本 ctx 在 `App` 根就 provide 了。共享的是**判定口径**——改一边必须同时改另一边，
        // 否则「停止」门闩与权限换档守卫会对同一个回合给出两种答案。
        let stream = app.stream.clone();
        let perm_turn_in_flight = Memo::new(move |_| {
            let _ = stream.stream_abort_epoch.get();
            turn_lifecycle_stream_turn_busy(
                stream.turn_lifecycle.get(),
                stream.abort_cell.lock().unwrap().is_some(),
            )
        });
        Self {
            chat: app.chat,
            composer: app.chat_composer,
            locale: app.shell_ui.locale,
            apply_assistant_display_filters: app.shell_ui.apply_assistant_display_filters,
            show_turn_context_inject: app.shell_ui.show_turn_context_inject,
            selected_permission_level: app.llm_settings.selected_permission_level,
            permission_intent_owner: app.llm_settings.permission_intent_owner,
            api_base: app.shell_ui.api_base,
            perm_turn_in_flight,
            layout_toggle: IdeLayoutToggleSignals::from_app_signals(app),
        }
    }
}

/// 在 `App` 根等已 [`provide_context`] 的子树内调用；缺失时 panic（开发期即暴露接线错误）。
#[must_use]
pub fn expect_chat_shell_ctx() -> ChatShellLeptosContext {
    try_chat_shell_ctx()
        .expect("ChatShellLeptosContext: provide ChatShellLeptosContext in App root")
}

/// 与 [`expect_chat_shell_ctx`] 同源；无上下文时返回 `None`（便于单测 / 可选子树）。
#[must_use]
pub fn try_chat_shell_ctx() -> Option<ChatShellLeptosContext> {
    use_context::<ChatShellLeptosContext>()
}
