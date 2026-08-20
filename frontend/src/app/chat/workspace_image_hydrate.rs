//! 聊天 DOM 中工作区图片：鉴权 fetch 后换成 `blob:`（`<img>` 不会带 Bearer）。

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlImageElement, Url};

use crate::api::fetch_workspace_image_blob_url;
use crate::markdown::workspace_image::is_workspace_raw_img_src;

/// 替换 innerHTML 前收回已创建的 blob URL，避免泄漏。
pub fn revoke_workspace_image_blobs(root: &web_sys::Element) {
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

/// 将 `/workspace/file/raw` 的 `<img>` 换成带鉴权的 blob src。
pub fn schedule_workspace_image_hydrate(root: &web_sys::Element) {
    let root = root.clone();
    spawn_local(async move {
        hydrate_workspace_images(&root).await;
    });
}

async fn hydrate_workspace_images(root: &web_sys::Element) {
    let Ok(list) = root.query_selector_all("img") else {
        return;
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
        if !is_workspace_raw_img_src(&src) {
            continue;
        }
        if img.get_attribute("data-cm-ws-hydrated").is_some() {
            continue;
        }
        let _ = img.set_attribute("data-cm-ws-hydrated", "1");
        let _ = img.set_attribute("data-cm-ws-raw", &src);
        img.set_src("");
        jobs.push((img, src));
    }
    for (img, src) in jobs {
        if let Some(blob_url) = fetch_workspace_image_blob_url(&src).await {
            img.set_src(&blob_url);
        }
    }
}
