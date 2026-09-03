//! 命令审批弹窗（阻塞式，替代 ApprovalBar）。

use gloo_timers::future::TimeoutFuture;
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::a11y::{focus_first_in_modal_container, handle_modal_layer_keydown};
use crate::api::submit_chat_approval;
use crate::i18n::{self, Locale};

/// 审批弹窗入参聚合（阶段 B：`App` 单行传入）。
#[derive(Clone, Copy)]
pub struct ApprovalModalSignals {
    pub pending_approval: RwSignal<Option<(String, String, String)>>,
    /// 决定提交中：按钮禁用，避免 Escape / 连击重复请求。
    pub busy: RwSignal<bool>,
    /// 提交失败（弹窗保留并展示，允许重试）。
    pub error: RwSignal<Option<String>>,
    pub locale: RwSignal<Locale>,
}

/// 提交审批决定。提交中不重复提交；**失败时保留弹窗并回显错误**，由用户重试，
/// 避免「先清信号再 POST、失败被忽略」导致命令在服务端永久挂起。
pub(crate) fn submit_pending_approval_decision(
    signals: ApprovalModalSignals,
    decision: &'static str,
) {
    if signals.busy.get_untracked() {
        return;
    }
    let Some((sid, _, _)) = signals.pending_approval.get_untracked() else {
        return;
    };
    signals.busy.set(true);
    signals.error.set(None);
    let loc = signals.locale.get_untracked();
    spawn_local(async move {
        match submit_chat_approval(&sid, decision, loc).await {
            Ok(()) => {
                signals.pending_approval.set(None);
                signals.busy.set(false);
                crate::mobile_stream_keepalive::on_approval_resolved();
            }
            Err(e) => {
                signals.error.set(Some(e));
                signals.busy.set(false);
            }
        }
    });
}

/// Escape / 拒绝：向服务端提交 `deny`。
pub(crate) fn deny_pending_approval(signals: ApprovalModalSignals) {
    submit_pending_approval_decision(signals, "deny");
}

/// 弹窗出现后聚焦首元素（异步等待首帧）。
fn schedule_approval_modal_focus(
    pending_approval: RwSignal<Option<(String, String, String)>>,
    dialog_ref: NodeRef<Div>,
) {
    Effect::new(move |_| {
        if pending_approval.get().is_none() {
            return;
        }
        let r = dialog_ref;
        spawn_local(async move {
            TimeoutFuture::new(0).await;
            if let Some(el) = r.get() {
                focus_first_in_modal_container(el.as_ref());
            }
        });
    });
}

/// `pending_approval`: `(approval_session_id, command, args)`
#[component]
pub fn ApprovalModal(signals: ApprovalModalSignals) -> impl IntoView {
    let ApprovalModalSignals {
        pending_approval,
        busy,
        error,
        locale,
    } = signals;
    let dialog_ref = NodeRef::<Div>::new();

    schedule_approval_modal_focus(pending_approval, dialog_ref);

    let deny = move |_| {
        deny_pending_approval(signals);
    };
    let allow_once = move |_| {
        submit_pending_approval_decision(signals, "allow_once");
    };
    let allow_always = move |_| {
        submit_pending_approval_decision(signals, "allow_always");
    };

    view! {
        <Show when=move || pending_approval.get().is_some()>
            <div class="modal-backdrop approval-modal-backdrop">
                <div
                    class="modal approval-modal"
                    node_ref=dialog_ref
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="approval-modal-title"
                    data-testid="approval-modal"
                    tabindex="-1"
                    on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if let Some(el) = dialog_ref.get() {
                            handle_modal_layer_keydown(&ev, el.as_ref(), || {
                                deny_pending_approval(signals);
                            });
                        }
                    }
                >
                    <div class="modal-head">
                        <span id="approval-modal-title" class="modal-title">
                            {"⚠️ "}
                            {move || i18n::approval_modal_title(locale.get())}
                        </span>
                    </div>
                    <div class="modal-body">
                        <p class="approval-modal-intro">
                            {move || i18n::approval_modal_intro(locale.get())}
                        </p>
                        {move || {
                            pending_approval.get().map(|(_sid, cmd, args)| {
                                let full = format!("{} {}", cmd, args);
                                view! {
                                    <pre class="approval-modal-command">{full}</pre>
                                }
                            })
                        }}
                        <Show when=move || error.get().is_some()>
                            <p class="approval-modal-error" role="alert">
                                {move || error.get().unwrap_or_default()}
                            </p>
                        </Show>
                    </div>
                    <ApprovalModalActions
                        locale=locale
                        busy=busy
                        deny=deny
                        allow_once=allow_once
                        allow_always=allow_always
                    />
                </div>
            </div>
        </Show>
    }
}

#[component]
fn ApprovalModalActions<Fd, FOnce, FAlways>(
    locale: RwSignal<Locale>,
    busy: RwSignal<bool>,
    deny: Fd,
    allow_once: FOnce,
    allow_always: FAlways,
) -> impl IntoView
where
    Fd: Fn(leptos::ev::MouseEvent) + Copy + 'static,
    FOnce: Fn(leptos::ev::MouseEvent) + Copy + 'static,
    FAlways: Fn(leptos::ev::MouseEvent) + Copy + 'static,
{
    view! {
        <div class="modal-footer actions approval-modal-actions">
            <button
                type="button"
                class="btn btn-danger"
                data-testid="approval-deny"
                prop:disabled=move || busy.get()
                on:click=deny
            >
                {move || i18n::approval_deny(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-secondary"
                data-testid="approval-allow-once"
                prop:disabled=move || busy.get()
                on:click=allow_once
            >
                {move || i18n::approval_allow_once(locale.get())}
            </button>
            <button
                type="button"
                class="btn btn-primary"
                data-testid="approval-allow-always"
                prop:disabled=move || busy.get()
                on:click=allow_always
            >
                {move || i18n::approval_allow_always(locale.get())}
            </button>
        </div>
    }
}

#[cfg(test)]
mod approval_modal_source_tests {
    #[test]
    fn approval_modal_traps_tab_and_esc_denies() {
        let src = include_str!("approval_modal.rs");
        assert!(src.contains("handle_modal_layer_keydown"));
        assert!(src.contains("deny_pending_approval"));
        assert!(src.contains("focus_first_in_modal_container"));
        assert!(src.contains("tabindex=\"-1\""));
    }
}
