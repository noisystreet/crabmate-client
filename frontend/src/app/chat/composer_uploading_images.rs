//! Composer 上传中的附图：逐项状态（等待 / 上传中 / 失败原因）、移除与取消。
//!
//! 与 [`super::composer_pending_images::ComposerPendingImagesRow`] 分开：待发区只放**已完成**
//! 的 `/uploads/...`，所以发送路径天然看不到未完成项；这里放的是还没有 URL 的那些。

use leptos::prelude::*;

use super::composer_image_upload::{
    cancel_chat_image_uploads, remove_chat_image_upload, upload_queue,
};
use crate::chat_upload_plan::{ChatUploadItem, ChatUploadState, has_pending_uploads};
use crate::i18n::{self, Locale};
use crate::icon::icon_x;

#[component]
pub fn ComposerUploadingImagesRow(locale: RwSignal<Locale>) -> impl IntoView {
    let queue = upload_queue();
    view! {
        <Show when=move || !queue.get().is_empty()>
            <div class="composer-uploading-images" data-testid="composer-uploading-images">
                // 不用 `For` + key：状态变化时 key（id）不变，`For` 会保留旧视图不重绘。
                // 队列最多 6 项，整体重绘既简单又不必维护 key 语义。
                {move || {
                    let items = queue.get();
                    let show_cancel = has_pending_uploads(&items);
                    let rows: Vec<_> = items
                        .into_iter()
                        .map(|item| upload_item_row(locale, item))
                        .collect();
                    view! {
                        {rows}
                        {show_cancel
                            .then(|| {
                                view! {
                                    <button
                                        type="button"
                                        class="composer-upload-cancel"
                                        on:click=move |_| cancel_chat_image_uploads()
                                    >
                                        {i18n::composer_upload_cancel(locale.get_untracked())}
                                    </button>
                                }
                            })}
                    }
                }}
            </div>
        </Show>
    }
}

fn upload_item_row(locale: RwSignal<Locale>, item: ChatUploadItem) -> impl IntoView {
    let id = item.id;
    let failed = matches!(item.state, ChatUploadState::Failed(_));
    let state_text = upload_state_text(locale.get_untracked(), &item.state);
    let name = item.name;
    view! {
        <div class="composer-upload-item" class:composer-upload-item-failed=failed>
            <span class="composer-upload-item-name" title=name.clone()>
                {name.clone()}
            </span>
            <span class="composer-upload-item-state">{state_text}</span>
            <button
                type="button"
                class="composer-upload-item-remove"
                prop:aria-label=move || i18n::composer_remove_image_aria(locale.get())
                on:click=move |_| remove_chat_image_upload(id)
            >
                {icon_x("")}
            </button>
        </div>
    }
}

fn upload_state_text(locale: Locale, state: &ChatUploadState) -> String {
    match state {
        ChatUploadState::Queued => i18n::composer_upload_queued_state(locale).to_string(),
        ChatUploadState::Uploading => i18n::composer_upload_uploading_state(locale).to_string(),
        ChatUploadState::Failed(reason) => i18n::composer_upload_failed_state(locale, reason),
    }
}
