//! 打开后聚焦第一项、方向键在 menuitem 间移动的 `role=menu` 容器，以及模态焦点陷阱。

use leptos::html::Div;
use leptos::prelude::*;

use crate::a11y::{
    capture_focus, handle_menu_keyboard, handle_modal_layer_keydown, restore_focus_to,
    schedule_focus_first_in_modal, schedule_focus_first_menu_item,
};

/// 自定义上下文 / 下拉菜单面板（须放在 `<Show>` 内，挂载时聚焦；关闭后焦点归还触发元素）。
#[component]
pub(crate) fn FocusableRoleMenu(
    #[prop(optional)] class: &'static str,
    #[prop(optional)] menu_style: Option<Memo<String>>,
    #[prop(optional)] aria_label: Option<Memo<String>>,
    on_escape: Callback<()>,
    children: Children,
) -> impl IntoView {
    let menu_ref = NodeRef::<Div>::new();
    let restore_target = StoredValue::new(capture_focus());
    on_cleanup(move || {
        restore_focus_to(restore_target.get_value().as_ref());
    });
    Effect::new(move |_| {
        if let Some(el) = menu_ref.get() {
            schedule_focus_first_menu_item(el.as_ref());
        }
    });
    view! {
        <div
            class=class
            node_ref=menu_ref
            role="menu"
            tabindex="-1"
            prop:style=move || menu_style.map(|m| m.get()).unwrap_or_default()
            attr:aria-label=move || {
                aria_label
                    .map(|m| m.get())
                    .filter(|s| !s.is_empty())
            }
            on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if let Some(el) = menu_ref.get() {
                    handle_menu_keyboard(&ev, el.as_ref(), move || on_escape.run(()));
                }
            }
        >
            {children()}
        </div>
    }
}

/// 确认框 / 新建文件等模态面板：挂载聚焦 + Tab 陷阱 + Escape；关闭后焦点归还触发元素（须放 `<Show>` 内）。
///
/// `labelledby` 用 `#[prop(into)] String`：标题 id 既可为静态字面量，也可为按会话 / 模式拼出的动态 id。
/// `stop_pointerdown` 默认关：仅个别弹窗需要吞掉 `pointerdown`，避免文档级手势（窄屏边缘滑动切抽屉）
/// 或顶栏菜单的 window 级 `pointerdown` 在点弹窗内部时被触发。
#[component]
pub(crate) fn FocusableModalPanel(
    #[prop(optional)] class: &'static str,
    #[prop(default = "alertdialog")] dialog_role: &'static str,
    #[prop(into)] labelledby: String,
    #[prop(optional)] testid: Option<&'static str>,
    #[prop(optional)] stop_pointerdown: bool,
    on_escape: Callback<()>,
    children: Children,
) -> impl IntoView {
    let dialog_ref = NodeRef::<Div>::new();
    let restore_target = StoredValue::new(capture_focus());
    on_cleanup(move || restore_focus_to(restore_target.get_value().as_ref()));
    Effect::new(move |_| {
        if let Some(el) = dialog_ref.get() {
            schedule_focus_first_in_modal(el.as_ref());
        }
    });
    view! {
        <div
            class=class
            node_ref=dialog_ref
            role=dialog_role
            aria-modal="true"
            aria-labelledby=labelledby
            data-testid=testid
            tabindex="-1"
            on:pointerdown=move |ev: leptos::ev::PointerEvent| {
                if stop_pointerdown {
                    ev.stop_propagation();
                }
            }
            on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if let Some(el) = dialog_ref.get() {
                    handle_modal_layer_keydown(&ev, el.as_ref(), move || on_escape.run(()));
                }
            }
        >
            {children()}
        </div>
    }
}
