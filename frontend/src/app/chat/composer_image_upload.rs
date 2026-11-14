//! Composer 图片上传队列：逐张 `POST /upload`，逐项状态、失败原因与取消。
//!
//! **为什么逐张而不是一次 multipart 传完**：`fetch` 拿不到上传字节进度（只有 XHR 的
//! `upload.onprogress` 有），而待发区本来就是逐张展示的。逐张发送让「第 2/6 张」
//! 与每张自己的失败原因都成为真状态，取消也有了明确语义（停队列；已传成功的附图保留）。
//!
//! **状态归属**：队列是每窗口一份的 `thread_local` 信号（与
//! [`connection_feed::connections_registered`](crate::app::connection_feed) 同款 `OnceCell`
//! 写法），因此不必把它穿进 `ChatComposerSignals` 的十几个构造点。代价是「每窗口一份」这个
//! 前提必须成立 —— 与本仓 `pending_images` 等单例同源，换窗口即换页面。

use std::cell::{OnceCell, RefCell};

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::upload_files_multipart;
use crate::chat_upload_plan::{
    CHAT_IMAGE_MAX_COUNT, ChatImageCandidate, ChatImagePlanError, ChatUploadItem, ChatUploadState,
    cancel_queue, finish_item, mark_item_uploading, next_queued_item, plan_chat_image_uploads,
    remove_item, upload_delete_targets,
};
use crate::i18n::{self, Locale};

thread_local! {
    static QUEUE: OnceCell<RwSignal<Vec<ChatUploadItem>>> = const { OnceCell::new() };
    static CANCELLED: OnceCell<RwSignal<bool>> = const { OnceCell::new() };
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

/// 队列信号：UI 订阅它，写入统一走本模块的函数（保证取消标记与队列同步）。
pub(crate) fn upload_queue() -> RwSignal<Vec<ChatUploadItem>> {
    QUEUE.with(|cell| *cell.get_or_init(|| RwSignal::new(Vec::new())))
}

fn cancelled_signal() -> RwSignal<bool> {
    CANCELLED.with(|cell| *cell.get_or_init(|| RwSignal::new(false)))
}

fn next_id() -> u64 {
    NEXT_ID.with(|cell| {
        let mut next = cell.borrow_mut();
        let id = *next;
        *next += 1;
        id
    })
}

/// 把一批本地图片排进上传队列。
///
/// 预算不合规时**只报错、不发任何请求**（校验在 [`plan_chat_image_uploads`] 里，可单测）；
/// 合规时按选择顺序逐张发送。
pub(crate) fn enqueue_chat_image_uploads(
    files: Vec<(String, web_sys::File)>,
    locale: RwSignal<Locale>,
    pending_images: RwSignal<Vec<String>>,
    status_err: RwSignal<Option<String>>,
) {
    if files.is_empty() {
        return;
    }
    let queue = upload_queue();
    let candidates: Vec<ChatImageCandidate> = files
        .iter()
        .map(|(name, file)| ChatImageCandidate {
            name: name.clone(),
            size: file_size(file),
        })
        .collect();
    // 上限是「总共 6 张」：待发区已完成的 + 队列里还在传 / 失败的一起算。
    let already = pending_images.get_untracked().len() + queue.get_untracked().len();
    let planned = match plan_chat_image_uploads(&candidates, already) {
        Ok(planned) => planned,
        Err(ChatImagePlanError::Empty) => return,
        Err(err) => {
            status_err.set(Some(plan_error_message(locale.get_untracked(), &err)));
            return;
        }
    };

    cancelled_signal().set(false);
    status_err.set(None);
    let queued: Vec<ChatUploadItem> = planned
        .into_iter()
        .map(|c| ChatUploadItem {
            id: next_id(),
            name: c.name,
            state: ChatUploadState::Queued,
        })
        .collect();
    let ids: Vec<u64> = queued.iter().map(|item| item.id).collect();
    queue.update(|items| items.extend(queued));
    spawn_upload_driver(files, ids, locale, pending_images);
}

/// 顺序发送：每张一个请求，前一张结束（成功或失败）才发下一张。
fn spawn_upload_driver(
    files: Vec<(String, web_sys::File)>,
    ids: Vec<u64>,
    locale: RwSignal<Locale>,
    pending_images: RwSignal<Vec<String>>,
) {
    let queue = upload_queue();
    spawn_local(async move {
        for (idx, (name, file)) in files.iter().enumerate() {
            let Some(id) = ids.get(idx).copied() else {
                break;
            };
            // 取消 / 该项已被移除：后面的都不再发（在途那一张的结果由 `finish_item` 丢弃）。
            if next_queued_item(&queue.get_untracked(), cancelled_signal().get_untracked())
                .is_none()
            {
                break;
            }
            let mut started = false;
            queue.update(|items| started = mark_item_uploading(items, id));
            if !started {
                continue;
            }
            let form = form_with_one_image(file, name);
            let (result, url) = match upload_files_multipart(&form, locale.get_untracked()).await {
                Ok(mut urls) => (Ok(()), urls.drain(..).next()),
                Err(err) => (Err(err), None),
            };
            let cancelled_now = cancelled_signal().get_untracked();
            let mut accepted = false;
            queue.update(|items| {
                accepted = finish_item(items, id, cancelled_now, &result);
            });
            match url {
                // 取消之后才传完：结果不要，但文件已经落在 Server 上，顺手删掉。
                Some(url) if !accepted => delete_uploaded_silently(url, locale),
                Some(url) => push_pending_image(pending_images, url),
                None => {}
            }
        }
    });
}

fn push_pending_image(pending_images: RwSignal<Vec<String>>, url: String) {
    pending_images.update(|urls| {
        if urls.len() < CHAT_IMAGE_MAX_COUNT && !urls.contains(&url) {
            urls.push(url);
        }
    });
}

/// 静默删除线上暂存文件：失败一律忽略，Server 的暂存目录自己会按龄 / 按总量清理。
fn delete_uploaded_silently(url: String, locale: RwSignal<Locale>) {
    let targets = upload_delete_targets(std::slice::from_ref(&url));
    if targets.is_empty() {
        return;
    }
    spawn_local(async move {
        let _ = crate::api::delete_upload_urls(&targets, locale.get_untracked()).await;
    });
}

fn form_with_one_image(file: &web_sys::File, name: &str) -> web_sys::FormData {
    let form = web_sys::FormData::new().expect("FormData");
    let _ = form.append_with_blob_and_filename("file", file, name);
    form
}

fn file_size(file: &web_sys::File) -> u64 {
    let size = file.size();
    if size.is_finite() && size > 0.0 {
        size as u64
    } else {
        0
    }
}

fn plan_error_message(locale: Locale, err: &ChatImagePlanError) -> String {
    match err {
        ChatImagePlanError::Empty => String::new(),
        ChatImagePlanError::TooMany { dropped } => i18n::composer_upload_too_many(locale, *dropped),
        ChatImagePlanError::TooLarge { name } => i18n::composer_upload_too_large(locale, name),
    }
}

/// 用户点「取消」：丢弃排队中 / 在传项，失败项留在界面上（原因还要给人看）。
pub(crate) fn cancel_chat_image_uploads() {
    cancelled_signal().set(true);
    upload_queue().update(cancel_queue);
}

/// 移除某一项（失败项、以及还在排队的项都走这里）。
pub(crate) fn remove_chat_image_upload(id: u64) {
    upload_queue().update(|items| remove_item(items, id));
}

/// 发送后 / 切换会话时清空：在途请求的结果随取消一并丢弃，不会漏进下一条消息。
pub(crate) fn clear_chat_image_uploads() {
    cancelled_signal().set(true);
    upload_queue().set(Vec::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn too_many_message_carries_both_numbers() {
        let msg = plan_error_message(Locale::ZhHans, &ChatImagePlanError::TooMany { dropped: 2 });
        assert!(msg.contains('6'), "应说明总上限: {msg}");
        assert!(msg.contains('2'), "应说明要少选几张: {msg}");
        let en = plan_error_message(Locale::En, &ChatImagePlanError::TooMany { dropped: 2 });
        assert!(en.contains('6') && en.contains('2'), "{en}");
    }

    #[test]
    fn too_large_message_names_the_file_and_cap() {
        let err = ChatImagePlanError::TooLarge {
            name: "huge.png".to_string(),
        };
        let msg = plan_error_message(Locale::ZhHans, &err);
        assert!(msg.contains("huge.png"), "{msg}");
        assert!(msg.contains("8 MiB"), "上限须与 Server 一致: {msg}");
    }

    #[test]
    fn empty_plan_error_has_no_message() {
        assert!(plan_error_message(Locale::En, &ChatImagePlanError::Empty).is_empty());
    }
}
