//! Composer 待发附图：鉴权拉 blob，失败占位，点击放大。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use web_sys::Url;

use super::chat_image_lightbox::{img_opens_lightbox, open_chat_image_lightbox};
use crate::api::fetch_auth_raster_image_blob_url;
use crate::chat_upload_src::chat_upload_filename;
use crate::i18n::{self, Locale};

#[component]
pub fn ComposerPendingImagesRow(
    locale: RwSignal<Locale>,
    pending_images: RwSignal<Vec<String>>,
) -> impl IntoView {
    view! {
        <div class="composer-pending-images" data-testid="composer-pending-images">
            <For
                each=move || pending_images.get()
                key=|url| url.clone()
                children=move |url| {
                    let u_rm = url.clone();
                    view! {
                        <div class="composer-pending-img-wrap">
                            <ComposerPendingThumb locale=locale path=url />
                            <button
                                type="button"
                                class="composer-pending-img-remove"
                                prop:aria-label=move || i18n::composer_remove_image_aria(locale.get())
                                on:click=move |_| pending_images.update(|v| v.retain(|x| x != &u_rm))
                            >"×"</button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
fn ComposerPendingThumb(locale: RwSignal<Locale>, path: String) -> impl IntoView {
    let blob = RwSignal::new(Option::<String>::None);
    let missing = RwSignal::new(false);
    let alive = Arc::new(AtomicBool::new(true));
    spawn_pending_thumb_blob(path.clone(), Arc::clone(&alive), blob, missing);
    on_cleanup(move || {
        alive.store(false, Ordering::Relaxed);
        if let Some(u) = blob.get_untracked() {
            let _ = Url::revoke_object_url(&u);
        }
    });
    let path_alt = path.clone();
    view! {
        <img
            class="composer-pending-img"
            class:chat-tui-img-missing=move || missing.get()
            prop:src=move || blob.get().unwrap_or_default()
            prop:alt=move || pending_thumb_alt(locale.get(), missing.get(), &path_alt)
            on:click=on_pending_thumb_click
        />
    }
}

fn spawn_pending_thumb_blob(
    path: String,
    alive: Arc<AtomicBool>,
    blob: RwSignal<Option<String>>,
    missing: RwSignal<bool>,
) {
    spawn_local(async move {
        match fetch_auth_raster_image_blob_url(&path).await {
            Some(u) => apply_pending_thumb_blob(&alive, blob, u),
            None => {
                if alive.load(Ordering::Relaxed) {
                    missing.set(true);
                }
            }
        }
    });
}

fn apply_pending_thumb_blob(alive: &AtomicBool, blob: RwSignal<Option<String>>, url: String) {
    if !alive.load(Ordering::Relaxed) {
        let _ = Url::revoke_object_url(&url);
        return;
    }
    blob.set(Some(url));
}

fn pending_thumb_alt(locale: Locale, missing: bool, path: &str) -> String {
    if missing {
        i18n::chat_image_unavailable(locale).to_string()
    } else {
        i18n::chat_image_attachment_alt(locale, chat_upload_filename(path)).to_string()
    }
}

fn on_pending_thumb_click(ev: web_sys::MouseEvent) {
    let Some(img) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlImageElement>().ok())
    else {
        return;
    };
    if !img_opens_lightbox(&img) {
        return;
    }
    open_chat_image_lightbox(&img.src(), &img.alt());
}
