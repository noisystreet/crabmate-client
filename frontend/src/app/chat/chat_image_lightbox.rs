//! 聊天 / composer 光栅图放大：只接受已水合的 `blob:` src。

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Document, Element, HtmlElement, HtmlImageElement, KeyboardEvent, MouseEvent};

use super::chat_image_blob_io::{copy_lightbox_blob, save_lightbox_blob};
use super::chat_image_filename::lightbox_download_filename;
use crate::i18n::{self, Locale};

type MouseCb = Closure<dyn FnMut(MouseEvent)>;

struct LightboxBind {
    overlay: Element,
    on_key: Closure<dyn FnMut(KeyboardEvent)>,
    on_click: MouseCb,
    on_close: MouseCb,
    on_ctx: MouseCb,
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
    let _ = bind
        .overlay
        .remove_event_listener_with_callback("contextmenu", bind.on_ctx.as_ref().unchecked_ref());
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

/// 从已水合 `<img>` 打开全屏预览（`data-cm-ws-raw` 用于另存文件名）。
/// `img.src` 必须是本页 `createObjectURL` 的 `blob:`。
pub fn open_chat_image_lightbox_from_img(img: &HtmlImageElement) {
    let raw = img.get_attribute("data-cm-ws-raw");
    open_chat_image_lightbox_named(&img.src(), &img.alt(), raw.as_deref());
}

fn open_chat_image_lightbox_named(blob_url: &str, alt: &str, raw_src: Option<&str>) {
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
    let filename = lightbox_download_filename(raw_src, alt);
    let Some((overlay, on_close)) = build_overlay(&doc, blob_url, alt, &filename, loc) else {
        return;
    };
    let overlay_for_click = overlay.clone();
    let on_click = Closure::wrap(Box::new(move |ev: MouseEvent| {
        on_overlay_click(&overlay_for_click, &ev);
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ = overlay.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    let overlay_ctx = overlay.clone();
    let on_ctx = Closure::wrap(Box::new(move |ev: MouseEvent| {
        on_overlay_contextmenu(&overlay_ctx, &ev);
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ =
        overlay.add_event_listener_with_callback("contextmenu", on_ctx.as_ref().unchecked_ref());
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
            on_ctx,
        });
    });
}

fn on_overlay_click(overlay: &Element, ev: &MouseEvent) {
    let Some(t) = ev.target() else {
        return;
    };
    let Ok(el) = t.dyn_into::<Element>() else {
        return;
    };
    if let Some(act) = action_from_click_target(&el) {
        ev.stop_propagation();
        dispatch_lb_act(overlay, &act);
        remove_lb_menu(overlay);
        return;
    }
    remove_lb_menu(overlay);
    if overlay.is_same_node(Some(&el)) {
        close_lightbox();
    }
}

fn action_from_click_target(el: &Element) -> Option<String> {
    el.closest("[data-cm-lb-act]")
        .ok()
        .flatten()?
        .get_attribute("data-cm-lb-act")
}

fn dispatch_lb_act(overlay: &Element, act: &str) {
    let Some(node) = overlay
        .query_selector(".chat-image-lightbox-img")
        .ok()
        .flatten()
    else {
        return;
    };
    let Ok(img) = node.dyn_into::<HtmlImageElement>() else {
        return;
    };
    let loc = locale_from_document();
    let name = img
        .get_attribute("data-cm-download-name")
        .unwrap_or_else(|| "image.png".into());
    match act {
        "copy" => copy_lightbox_blob(&img.src(), loc),
        "save" => save_lightbox_blob(&img.src(), &name, loc),
        _ => {}
    }
}

fn on_overlay_contextmenu(overlay: &Element, ev: &MouseEvent) {
    let Some(t) = ev.target() else {
        return;
    };
    let Ok(el) = t.dyn_into::<Element>() else {
        return;
    };
    if !el.class_list().contains("chat-image-lightbox-img") {
        return;
    }
    ev.prevent_default();
    ev.stop_propagation();
    show_lb_menu(overlay, ev.client_x(), ev.client_y());
}

fn remove_lb_menu(overlay: &Element) {
    if let Some(m) = overlay
        .query_selector(".chat-image-lightbox-menu")
        .ok()
        .flatten()
    {
        m.remove();
    }
}

fn show_lb_menu(overlay: &Element, x: i32, y: i32) {
    remove_lb_menu(overlay);
    let Some(doc) = overlay.owner_document() else {
        return;
    };
    let loc = locale_from_document();
    let Ok(menu) = doc.create_element("div") else {
        return;
    };
    menu.set_class_name("chat-image-lightbox-menu");
    let _ = menu.set_attribute("role", "menu");
    let (left, top) = clamp_menu_pos(x, y);
    let _ = menu.set_attribute("style", &format!("left:{left}px;top:{top}px"));
    append_menu_item(&menu, &doc, "copy", i18n::chat_image_lightbox_copy(loc));
    append_menu_item(&menu, &doc, "save", i18n::chat_image_lightbox_save(loc));
    let _ = overlay.append_child(&menu);
}

fn clamp_menu_pos(x: i32, y: i32) -> (i32, i32) {
    let w = window_axis(web_sys::Window::inner_width);
    let h = window_axis(web_sys::Window::inner_height);
    (clamp_menu_axis(x, w, 168), clamp_menu_axis(y, h, 96))
}

fn window_axis(
    f: fn(&web_sys::Window) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>,
) -> i32 {
    web_sys::window()
        .and_then(|w| f(&w).ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(400.0) as i32
}

fn clamp_menu_axis(v: i32, span: i32, box_size: i32) -> i32 {
    let max = (span - box_size).max(8);
    v.max(8).min(max)
}

fn append_menu_item(menu: &Element, doc: &Document, act: &str, label: &str) {
    let Ok(b) = doc.create_element("button") else {
        return;
    };
    b.set_class_name("chat-image-lightbox-menu-item");
    let _ = b.set_attribute("type", "button");
    let _ = b.set_attribute("role", "menuitem");
    let _ = b.set_attribute("data-cm-lb-act", act);
    b.set_text_content(Some(label));
    let _ = menu.append_child(&b);
}

fn build_overlay(
    doc: &Document,
    blob_url: &str,
    alt: &str,
    filename: &str,
    loc: Locale,
) -> Option<(Element, MouseCb)> {
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
    let _ = img.set_attribute("data-cm-download-name", filename);
    let actions = build_action_row(doc, loc)?;
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
    let _ = overlay.append_child(&actions);
    let _ = overlay.append_child(&btn);
    Some((overlay, on_close))
}

fn build_action_row(doc: &Document, loc: Locale) -> Option<Element> {
    let row = doc.create_element("div").ok()?;
    row.set_class_name("chat-image-lightbox-actions");
    append_action_btn(&row, doc, "copy", i18n::chat_image_lightbox_copy(loc))?;
    append_action_btn(&row, doc, "save", i18n::chat_image_lightbox_save(loc))?;
    Some(row)
}

fn append_action_btn(row: &Element, doc: &Document, act: &str, label: &str) -> Option<()> {
    let b = doc.create_element("button").ok()?;
    b.set_class_name("chat-image-lightbox-action");
    let _ = b.set_attribute("type", "button");
    let _ = b.set_attribute("data-cm-lb-act", act);
    let _ = b.set_attribute("aria-label", label);
    b.set_text_content(Some(label));
    let _ = row.append_child(&b);
    Some(())
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

#[cfg(test)]
mod tests {
    use super::clamp_menu_axis;

    #[test]
    fn clamp_menu_axis_stays_on_screen() {
        assert_eq!(clamp_menu_axis(0, 400, 168), 8);
        assert_eq!(clamp_menu_axis(390, 400, 168), 232);
        assert_eq!(clamp_menu_axis(100, 400, 168), 100);
    }
}
