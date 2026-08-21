//! 聊天 / composer 光栅图放大：只接受已水合的 `blob:` src。

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlElement, HtmlImageElement, KeyboardEvent, MouseEvent};

use crate::i18n::{self, Locale};

type LightboxClose = Closure<dyn FnMut(MouseEvent)>;

struct LightboxBind {
    overlay: Element,
    on_key: Closure<dyn FnMut(KeyboardEvent)>,
    on_click: Closure<dyn FnMut(MouseEvent)>,
    on_close: LightboxClose,
}

thread_local! {
    static OPEN: RefCell<Option<LightboxBind>> = const { RefCell::new(None) };
}

fn close_lightbox() {
    OPEN.with(|cell| {
        let Some(bind) = cell.borrow_mut().take() else {
            return;
        };
        unbind_lightbox(bind);
    });
}

fn unbind_lightbox(bind: LightboxBind) {
    if let Some(w) = web_sys::window() {
        let _ =
            w.remove_event_listener_with_callback("keydown", bind.on_key.as_ref().unchecked_ref());
    }
    let _ = bind
        .overlay
        .remove_event_listener_with_callback("click", bind.on_click.as_ref().unchecked_ref());
    if let Some(btn) = bind
        .overlay
        .query_selector(".chat-image-lightbox-close")
        .ok()
        .flatten()
    {
        let _ = btn
            .remove_event_listener_with_callback("click", bind.on_close.as_ref().unchecked_ref());
    }
    bind.overlay.remove();
}

/// 打开全屏预览；`blob_url` 必须是本页 `createObjectURL` 的结果。
pub fn open_chat_image_lightbox(blob_url: &str, alt: &str) {
    if !blob_url.starts_with("blob:") {
        return;
    }
    close_lightbox();
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(body) = doc.body() else {
        return;
    };
    let loc = locale_from_document();
    let Some((overlay, on_close)) = build_overlay(&doc, blob_url, alt, loc) else {
        return;
    };
    let overlay_for_click = overlay.clone();
    let on_click = Closure::wrap(Box::new(move |ev: MouseEvent| {
        let Some(t) = ev.target() else {
            return;
        };
        let Ok(el) = t.dyn_into::<Element>() else {
            return;
        };
        if overlay_for_click.is_same_node(Some(&el)) {
            close_lightbox();
        }
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ = overlay.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    let on_key = Closure::wrap(Box::new(move |ev: KeyboardEvent| {
        if ev.key() == "Escape" {
            ev.prevent_default();
            close_lightbox();
        }
    }) as Box<dyn FnMut(KeyboardEvent)>);
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref());
    }
    let _ = body.append_child(&overlay);
    if let Ok(focus_el) = overlay.clone().dyn_into::<HtmlElement>() {
        let _ = focus_el.focus();
    }
    OPEN.with(|cell| {
        *cell.borrow_mut() = Some(LightboxBind {
            overlay,
            on_key,
            on_click,
            on_close,
        });
    });
}

fn build_overlay(
    doc: &Document,
    blob_url: &str,
    alt: &str,
    loc: Locale,
) -> Option<(Element, LightboxClose)> {
    let overlay = doc.create_element("div").ok()?;
    overlay.set_class_name("chat-image-lightbox");
    let _ = overlay.set_attribute("role", "dialog");
    let _ = overlay.set_attribute("aria-modal", "true");
    let _ = overlay.set_attribute("aria-label", i18n::chat_image_lightbox_aria(loc));
    let _ = overlay.set_attribute("tabindex", "0");
    let img = doc.create_element("img").ok()?;
    img.set_class_name("chat-image-lightbox-img");
    let _ = img.set_attribute("src", blob_url);
    let _ = img.set_attribute("alt", alt);
    let btn = doc.create_element("button").ok()?;
    btn.set_class_name("chat-image-lightbox-close");
    let _ = btn.set_attribute("type", "button");
    let _ = btn.set_attribute("aria-label", i18n::chat_image_lightbox_close(loc));
    btn.set_text_content(Some("×"));
    let on_close = Closure::wrap(Box::new(move |ev: MouseEvent| {
        ev.stop_propagation();
        close_lightbox();
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ = btn.add_event_listener_with_callback("click", on_close.as_ref().unchecked_ref());
    let _ = overlay.append_child(&img);
    let _ = overlay.append_child(&btn);
    Some((overlay, on_close))
}

fn locale_from_document() -> Locale {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .and_then(|e| e.get_attribute("lang"))
        .map(|l| Locale::from_html_lang(&l))
        .unwrap_or(Locale::ZhHans)
}

#[must_use]
pub fn img_opens_lightbox(img: &HtmlImageElement) -> bool {
    if img.class_list().contains("chat-tui-img-missing") {
        return false;
    }
    img.src().starts_with("blob:")
}
