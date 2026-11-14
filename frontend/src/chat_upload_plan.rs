//! 聊天附图上传：张数/大小预算与队列推进（纯逻辑，无 DOM / HTTP，可原生单测）。
//!
//! 上限取自 Server `src/web/chat_handlers/upload.rs`（`upload_handler` /
//! `upload_max_single_bytes`）：单张图片 8 MiB、单请求最多 20 个字段、单请求总量 200 MB。
//! 客户端唯一更严的是**张数**（[`CHAT_IMAGE_MAX_COUNT`]）—— 待发缩略图行本来就只放得下这么多，
//! 与其传完再截断（旧行为：`v.len() >= 6` 发生在请求成功之后，多出来的图已经白传了），
//! 不如在选择/拖放当场拒掉。

/// 与 Server `upload_max_single_bytes` 的图片分支对齐（8 MiB）。
///
/// 注意它**不是** [`crate::workspace_file_drop::WORKSPACE_UPLOAD_MAX_BYTES`]（16 MiB）：
/// 那条路写的是 `/workspace/file/raw`，与 `/upload` 是两套限制，别互相抄。
pub(crate) const CHAT_IMAGE_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// composer 最多保留的附图张数（含仍在上传的）。
pub(crate) const CHAT_IMAGE_MAX_COUNT: usize = 6;

/// 待上传的一张图：只带校验需要的字段，不持有 `File`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChatImageCandidate {
    pub name: String,
    pub size: u64,
}

/// 预算校验失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChatImagePlanError {
    /// 没有可上传的图片（例如只拖进了非图片文件）。
    Empty,
    /// 连上已有附图会超过 [`CHAT_IMAGE_MAX_COUNT`]；`dropped` 是必须少选的张数。
    TooMany { dropped: usize },
    /// 单张超过 [`CHAT_IMAGE_MAX_BYTES`]。
    TooLarge { name: String },
}

/// 上传前校验：整批要么全部入队，要么一张都不发。
///
/// `already` 是 composer 里已有（含仍在上传）的附图张数 —— 上限是「总共 6 张」，
/// 不是「每次选 6 张」。
pub(crate) fn plan_chat_image_uploads(
    candidates: &[ChatImageCandidate],
    already: usize,
) -> Result<Vec<ChatImageCandidate>, ChatImagePlanError> {
    if candidates.is_empty() {
        return Err(ChatImagePlanError::Empty);
    }
    let dropped = (already + candidates.len()).saturating_sub(CHAT_IMAGE_MAX_COUNT);
    if dropped > 0 {
        return Err(ChatImagePlanError::TooMany { dropped });
    }
    for c in candidates {
        if c.size > CHAT_IMAGE_MAX_BYTES {
            return Err(ChatImagePlanError::TooLarge {
                name: c.name.clone(),
            });
        }
    }
    Ok(candidates.to_vec())
}

/// 队列里一项的状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChatUploadState {
    Queued,
    Uploading,
    Failed(String),
}

/// 上传队列的一项。`id` 由调用方分配，使队列增删后仍能定位同一项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChatUploadItem {
    pub id: u64,
    pub name: String,
    pub state: ChatUploadState,
}

/// 下一张该传的图；`None` 表示取消中或已无排队项。
pub(crate) fn next_queued_item(items: &[ChatUploadItem], cancelled: bool) -> Option<u64> {
    if cancelled {
        return None;
    }
    items
        .iter()
        .find(|i| matches!(i.state, ChatUploadState::Queued))
        .map(|i| i.id)
}

/// 把某项置为上传中；返回 `false` 表示该项已被用户移除 —— 此时不该再发请求。
pub(crate) fn mark_item_uploading(items: &mut [ChatUploadItem], id: u64) -> bool {
    set_state(items, id, ChatUploadState::Uploading)
}

/// 一项上传结束。成功移出队列并返回 `true`（URL 由调用方收进 `pending_images`）；
/// 失败则留在队列里标 `Failed`（行内可见原因，用户可移除）；已取消时两种结果都不留痕。
pub(crate) fn finish_item(
    items: &mut Vec<ChatUploadItem>,
    id: u64,
    cancelled: bool,
    result: &Result<(), String>,
) -> bool {
    match result {
        Ok(()) => {
            items.retain(|i| i.id != id);
            !cancelled
        }
        Err(reason) => {
            if cancelled {
                items.retain(|i| i.id != id);
            } else {
                set_state(items, id, ChatUploadState::Failed(reason.clone()));
            }
            false
        }
    }
}

/// 取消：排队中 / 在传项全部移除；失败项保留（用户还要看原因、自己移除）。
pub(crate) fn cancel_queue(items: &mut Vec<ChatUploadItem>) {
    items.retain(|i| matches!(i.state, ChatUploadState::Failed(_)));
}

/// 是否还有排队中 / 在传项（决定「取消」按钮是否露面）。
pub(crate) fn has_pending_uploads(items: &[ChatUploadItem]) -> bool {
    items
        .iter()
        .any(|i| !matches!(i.state, ChatUploadState::Failed(_)))
}

pub(crate) fn remove_item(items: &mut Vec<ChatUploadItem>, id: u64) {
    items.retain(|i| i.id != id);
}

/// 可作为 `POST /uploads/delete` 目标的 URL：只留单段 `/uploads/<file>`，去重保序。
///
/// Server 的删除端点自己也会跳过非法项，但把过滤放在客户端有两个好处：非暂存路径
/// （例如工作区 `/workspace/...`）不会被误发出去，且这段判定是可单测的纯逻辑。
pub(crate) fn upload_delete_targets(urls: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for url in urls {
        if crate::chat_upload_src::safe_chat_upload_path(url).is_none() {
            continue;
        }
        if !out.contains(url) {
            out.push(url.clone());
        }
    }
    out
}

fn set_state(items: &mut [ChatUploadItem], id: u64, state: ChatUploadState) -> bool {
    match items.iter_mut().find(|i| i.id == id) {
        Some(item) => {
            item.state = state;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(name: &str, size: u64) -> ChatImageCandidate {
        ChatImageCandidate {
            name: name.to_string(),
            size,
        }
    }

    fn queued(id: u64) -> ChatUploadItem {
        ChatUploadItem {
            id,
            name: format!("{id}.png"),
            state: ChatUploadState::Queued,
        }
    }

    #[test]
    fn empty_selection_is_not_an_upload() {
        assert_eq!(
            plan_chat_image_uploads(&[], 0),
            Err(ChatImagePlanError::Empty)
        );
    }

    #[test]
    fn selection_within_budget_is_planned_in_order() {
        let picked = vec![cand("a.png", 1024), cand("b.png", 2048)];
        assert_eq!(
            plan_chat_image_uploads(&picked, 0),
            Ok(picked.clone()),
            "入队顺序必须与选择顺序一致（驱动按同一下标取 File）"
        );
    }

    #[test]
    fn too_many_counts_images_already_attached() {
        // 已有 3 张 + 这次 5 张 = 8 张，超出 6 张 2 张。
        let picked = vec![
            cand("1.png", 10),
            cand("2.png", 10),
            cand("3.png", 10),
            cand("4.png", 10),
            cand("5.png", 10),
        ];
        assert_eq!(
            plan_chat_image_uploads(&picked, 3),
            Err(ChatImagePlanError::TooMany { dropped: 2 })
        );
    }

    #[test]
    fn exactly_at_capacity_is_allowed() {
        let picked = vec![cand("1.png", 10); 5];
        assert!(plan_chat_image_uploads(&picked, 1).is_ok());
    }

    #[test]
    fn oversized_file_is_reported_by_name() {
        let picked = vec![cand("big.png", CHAT_IMAGE_MAX_BYTES + 1)];
        assert_eq!(
            plan_chat_image_uploads(&picked, 0),
            Err(ChatImagePlanError::TooLarge {
                name: "big.png".to_string()
            })
        );
    }

    #[test]
    fn exactly_at_size_cap_is_allowed() {
        let picked = vec![cand("edge.png", CHAT_IMAGE_MAX_BYTES)];
        assert!(plan_chat_image_uploads(&picked, 0).is_ok());
    }

    #[test]
    fn too_large_wins_over_whole_batch_dropping() {
        // 整批拒绝：一张超限就不上传任何一张，避免「传了 3 张才发现第 4 张不行」。
        let picked = vec![
            cand("ok.png", 10),
            cand("big.png", CHAT_IMAGE_MAX_BYTES + 1),
        ];
        assert_eq!(
            plan_chat_image_uploads(&picked, 0),
            Err(ChatImagePlanError::TooLarge {
                name: "big.png".to_string()
            })
        );
    }

    #[test]
    fn queue_advances_in_order_and_stops_when_cancelled() {
        let items = vec![queued(1), queued(2)];
        assert_eq!(next_queued_item(&items, false), Some(1));
        assert_eq!(next_queued_item(&items, true), None);
    }

    #[test]
    fn marked_item_is_no_longer_next() {
        let mut items = vec![queued(1), queued(2)];
        assert!(mark_item_uploading(&mut items, 1));
        assert_eq!(items[0].state, ChatUploadState::Uploading);
        assert_eq!(next_queued_item(&items, false), Some(2));
    }

    #[test]
    fn marking_a_removed_item_is_a_no_op() {
        let mut items = vec![queued(1)];
        assert!(!mark_item_uploading(&mut items, 99));
        assert_eq!(items.len(), 1, "移除后的项不该被复活");
    }

    #[test]
    fn successful_upload_leaves_the_queue() {
        let mut items = vec![queued(1), queued(2)];
        let accepted = finish_item(&mut items, 1, false, &Ok(()));
        assert!(accepted, "成功且未取消：URL 应收进 pending_images");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 2);
    }

    #[test]
    fn failed_upload_stays_visible_with_reason() {
        let mut items = vec![queued(1)];
        let accepted = finish_item(&mut items, 1, false, &Err("磁盘满".to_string()));
        assert!(!accepted, "失败不得进入 pending_images");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].state,
            ChatUploadState::Failed("磁盘满".to_string())
        );
    }

    #[test]
    fn completion_after_cancel_is_discarded() {
        let mut items = vec![queued(1)];
        // 取消发生在请求在途期间：结果到达时必须丢弃，否则取消后还会冒出附图。
        assert!(!finish_item(&mut items, 1, true, &Ok(())));
        assert!(items.is_empty());
        let mut items = vec![queued(1)];
        assert!(!finish_item(&mut items, 1, true, &Err("超时".to_string())));
        assert!(items.is_empty());
    }

    #[test]
    fn cancel_drops_pending_but_keeps_failures() {
        let mut items = vec![
            queued(1),
            ChatUploadItem {
                id: 2,
                name: "2.png".to_string(),
                state: ChatUploadState::Uploading,
            },
            ChatUploadItem {
                id: 3,
                name: "3.png".to_string(),
                state: ChatUploadState::Failed("x".to_string()),
            },
        ];
        cancel_queue(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 3);
        assert!(!has_pending_uploads(&items), "只剩失败项时不再显示取消");
    }

    #[test]
    fn pending_flag_covers_queued_and_uploading() {
        assert!(has_pending_uploads(&[queued(1)]));
        assert!(has_pending_uploads(&[ChatUploadItem {
            id: 1,
            name: "1.png".to_string(),
            state: ChatUploadState::Uploading,
        }]));
        assert!(!has_pending_uploads(&[]));
    }

    #[test]
    fn removing_by_id_keeps_the_rest() {
        let mut items = vec![queued(1), queued(2)];
        remove_item(&mut items, 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 2);
    }

    #[test]
    fn delete_targets_keep_only_single_segment_uploads() {
        let urls = vec![
            "/uploads/u1_2_3.png".to_string(),
            "/workspace/notes.md".to_string(),
            "/uploads/../etc/passwd".to_string(),
            "/uploads/sub/a.png".to_string(),
            "/uploads/u1_2_3.png".to_string(),
        ];
        assert_eq!(
            upload_delete_targets(&urls),
            vec!["/uploads/u1_2_3.png".to_string()],
            "非暂存路径、带路径段或 `..` 的、以及重复项都要挡掉"
        );
    }

    #[test]
    fn delete_targets_of_nothing_is_empty() {
        assert!(upload_delete_targets(&[]).is_empty());
    }
}
