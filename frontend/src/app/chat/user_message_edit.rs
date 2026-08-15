//! 用户消息就地编辑：状态 + 挂到 TUI wrap 内的 textarea（DOM 由 transcript 同步拥有）。

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, HtmlTextAreaElement};

use crate::i18n::{self, Locale};
use crate::markdown::plaintext_to_safe_html;
use crate::session_search::is_safe_dom_token;

/// 正在编辑的用户气泡。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserMessageEdit {
    pub message_id: String,
    pub draft: String,
}

/// 在目标 wrap 内挂编辑器；正文隐藏。已存在且焦点在 textarea 内则只同步 id。
pub(crate) fn mount_user_message_editor(
    transcript: &HtmlElement,
    editing: Option<&UserMessageEdit>,
    locale: Locale,
) {
    unmount_stale_editors(transcript, editing.map(|e| e.message_id.as_str()));
    let Some(edit) = editing else {
        return;
    };
    if !is_safe_dom_token(&edit.message_id) {
        return;
    }
    let Ok(Some(wrap)) = transcript.query_selector(&format!(
        ".chat-tui-turn-wrap[data-tui-wrap-id=\"{}\"]",
        edit.message_id
    )) else {
        return;
    };
    let _ = wrap.class_list().add_1("chat-tui-turn-wrap--editing");
    if wrap
        .query_selector("textarea.chat-user-edit-ta")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<HtmlTextAreaElement>().ok())
        .is_some_and(|ta| is_textarea_focused(&ta))
    {
        return;
    }
    if let Some(old) = wrap.query_selector(".chat-user-edit").ok().flatten() {
        old.remove();
    }
    if let Some(body) = wrap.query_selector(".chat-tui-body").ok().flatten() {
        let _ = body.class_list().add_1("chat-tui-body--editing-hidden");
    }
    let loc = locale;
    let html = user_edit_form_html(&edit.draft, loc);
    let _ = wrap.insert_adjacent_html("beforeend", &html);
    if let Some(ta) = wrap
        .query_selector("textarea.chat-user-edit-ta")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<HtmlTextAreaElement>().ok())
    {
        ta.set_value(&edit.draft);
        let _ = ta.focus();
    }
}

fn is_textarea_focused(ta: &HtmlTextAreaElement) -> bool {
    let Some(doc) = ta.owner_document() else {
        return false;
    };
    doc.active_element()
        .is_some_and(|el| ta.is_same_node(Some(el.as_ref())))
}

fn unmount_stale_editors(transcript: &HtmlElement, keep_id: Option<&str>) {
    let Ok(nodes) = transcript.query_selector_all(".chat-tui-turn-wrap--editing") else {
        return;
    };
    for i in 0..nodes.length() {
        let Some(n) = nodes.item(i) else {
            continue;
        };
        let Ok(wrap) = n.dyn_into::<web_sys::Element>() else {
            continue;
        };
        let id = wrap.get_attribute("data-tui-wrap-id").unwrap_or_default();
        if keep_id == Some(id.as_str()) {
            continue;
        }
        let _ = wrap.class_list().remove_1("chat-tui-turn-wrap--editing");
        if let Some(body) = wrap.query_selector(".chat-tui-body").ok().flatten() {
            let _ = body.class_list().remove_1("chat-tui-body--editing-hidden");
        }
        if let Some(ed) = wrap.query_selector(".chat-user-edit").ok().flatten() {
            ed.remove();
        }
    }
}

fn user_edit_form_html(draft: &str, locale: Locale) -> String {
    let _ = draft;
    format!(
        "<div class=\"chat-user-edit\" data-testid=\"chat-user-edit\">\
           <textarea class=\"chat-user-edit-ta\" data-testid=\"chat-user-edit-ta\" rows=\"4\"></textarea>\
           <div class=\"chat-user-edit-actions\">\
             <button type=\"button\" class=\"btn btn-primary btn-sm\" data-user-edit-save=\"1\" data-testid=\"chat-user-edit-save\">{}</button>\
             <button type=\"button\" class=\"btn btn-muted btn-sm\" data-user-edit-cancel=\"1\" data-testid=\"chat-user-edit-cancel\">{}</button>\
           </div>\
         </div>",
        plaintext_to_safe_html(i18n::msg_edit_save(locale)),
        plaintext_to_safe_html(i18n::msg_edit_cancel(locale)),
    )
}

/// transcript 点击：保存 / 取消。返回是否已消费。
pub(crate) fn try_handle_user_edit_click(
    ev: &web_sys::MouseEvent,
    editing: RwSignal<Option<UserMessageEdit>>,
) -> UserEditClick {
    let Some(target) = ev.target() else {
        return UserEditClick::None;
    };
    let Ok(node) = target.dyn_into::<web_sys::Node>() else {
        return UserEditClick::None;
    };
    let Some(el) = node
        .dyn_ref::<web_sys::Element>()
        .cloned()
        .or_else(|| node.parent_element())
    else {
        return UserEditClick::None;
    };
    if el
        .closest("[data-user-edit-cancel]")
        .ok()
        .flatten()
        .is_some()
    {
        editing.set(None);
        return UserEditClick::Cancel;
    }
    if el.closest("[data-user-edit-save]").ok().flatten().is_none() {
        return UserEditClick::None;
    }
    let Some(form) = el.closest(".chat-user-edit").ok().flatten() else {
        return UserEditClick::None;
    };
    let Some(wrap) = form.closest(".chat-tui-turn-wrap").ok().flatten() else {
        return UserEditClick::None;
    };
    let Some(id) = wrap.get_attribute("data-tui-wrap-id") else {
        return UserEditClick::None;
    };
    let text = form
        .query_selector("textarea.chat-user-edit-ta")
        .ok()
        .flatten()
        .and_then(|n| n.dyn_into::<HtmlTextAreaElement>().ok())
        .map(|ta| ta.value())
        .unwrap_or_default();
    editing.set(None);
    UserEditClick::Save {
        message_id: id,
        text,
    }
}

pub(crate) fn try_sync_user_edit_draft(
    ev: &web_sys::Event,
    editing: RwSignal<Option<UserMessageEdit>>,
) {
    let Some(target) = ev.target() else {
        return;
    };
    let Ok(ta) = target.dyn_into::<HtmlTextAreaElement>() else {
        return;
    };
    if !ta.class_list().contains("chat-user-edit-ta") {
        return;
    }
    let value = ta.value();
    editing.update(|ed| {
        if let Some(e) = ed.as_mut() {
            e.draft = value;
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserEditClick {
    None,
    Cancel,
    Save { message_id: String, text: String },
}
