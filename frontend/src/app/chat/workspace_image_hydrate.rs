//! 聊天 DOM 中工作区图与 `/uploads/` 附图：鉴权 fetch 后换成 `blob:`。

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlImageElement, MouseEvent, Url};

use super::chat_image_lightbox::{img_opens_lightbox, open_chat_image_lightbox};
use crate::api::fetch_auth_raster_image_blob_url;
use crate::chat_upload_src::relative_auth_image_src;
use crate::i18n::{self, Locale};

/// 替换 innerHTML 前收回已创建的 blob URL，避免泄漏。
pub fn revoke_workspace_image_blobs(root: &Element) {
    let Ok(list) = root.query_selector_all("img") else {
        return;
    };
    for i in 0..list.length() {
        let Some(node) = list.item(i) else {
            continue;
        };
        let Ok(img) = node.dyn_into::<HtmlImageElement>() else {
            continue;
        };
        let src = img.src();
        if src.starts_with("blob:") {
            let _ = Url::revoke_object_url(&src);
        }
    }
}

/// 将可鉴权相对路径的 `<img>` 换成 blob src；失败则占位。
pub fn schedule_workspace_image_hydrate(root: &Element) {
    bind_lightbox_once(root);
    let root = root.clone();
    spawn_local(async move {
        hydrate_auth_images(&root).await;
    });
}

fn bind_lightbox_once(root: &Element) {
    if root.get_attribute("data-cm-img-lb").is_some() {
        return;
    }
    let _ = root.set_attribute("data-cm-img-lb", "1");
    let on_click = Closure::wrap(Box::new(move |ev: MouseEvent| {
        let Some(t) = ev.target() else {
            return;
        };
        let Ok(img) = t.dyn_into::<HtmlImageElement>() else {
            return;
        };
        if !img_opens_lightbox(&img) {
            return;
        }
        open_chat_image_lightbox(&img.src(), &img.alt());
    }) as Box<dyn FnMut(MouseEvent)>);
    let _ = root.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    on_click.forget();
}

async fn hydrate_auth_images(root: &Element) {
    let jobs = collect_hydrate_jobs(root);
    let loc = locale_from_document();
    for (img, src) in jobs {
        match fetch_auth_raster_image_blob_url(&src).await {
            Some(blob_url) => img.set_src(&blob_url),
            None => mark_img_unavailable(&img, loc),
        }
    }
}

fn collect_hydrate_jobs(root: &Element) -> Vec<(HtmlImageElement, String)> {
    let Ok(list) = root.query_selector_all("img") else {
        return Vec::new();
    };
    let mut jobs = Vec::new();
    for i in 0..list.length() {
        let Some(node) = list.item(i) else {
            continue;
        };
        let Ok(img) = node.dyn_into::<HtmlImageElement>() else {
            continue;
        };
        let src = img.get_attribute("src").unwrap_or_default();
        let Some(rel) = relative_auth_image_src(&src) else {
            continue;
        };
        if img.get_attribute("data-cm-ws-hydrated").is_some() {
            continue;
        }
        let _ = img.set_attribute("data-cm-ws-hydrated", "1");
        let _ = img.set_attribute("data-cm-ws-raw", rel);
        img.set_src("");
        jobs.push((img, rel.to_string()));
    }
    jobs
}

fn mark_img_unavailable(img: &HtmlImageElement, loc: Locale) {
    img.set_src("");
    let _ = img.class_list().add_1("chat-tui-img-missing");
    img.set_alt(i18n::chat_image_unavailable(loc));
}

fn locale_from_document() -> Locale {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .and_then(|e| e.get_attribute("lang"))
        .map(|l| Locale::from_html_lang(&l))
        .unwrap_or(Locale::ZhHans)
}
