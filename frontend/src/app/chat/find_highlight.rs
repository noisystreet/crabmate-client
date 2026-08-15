//! 会话内查找：气泡内 `<mark>` 与当前命中 wrap 样式。

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, Node, Text};

use crate::session_search::is_safe_dom_token;

const MARK_CLASS: &str = "chat-find-mark";
const MARK_CURRENT_CLASS: &str = "chat-find-mark--current";
const WRAP_HIT: &str = "chat-tui-turn-wrap--find-hit";
const WRAP_CURRENT: &str = "chat-tui-turn-wrap--find-current";

/// 大小写不敏感、按字符窗口切出高亮段（`needle_lower` 已是小写）。
#[must_use]
pub fn split_highlight_parts(hay: &str, needle_lower: &str) -> Vec<(String, bool)> {
    if needle_lower.is_empty() {
        return vec![(hay.to_string(), false)];
    }
    let nchars = needle_lower.chars().count();
    if nchars == 0 {
        return vec![(hay.to_string(), false)];
    }
    let chars: Vec<char> = hay.chars().collect();
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + nchars <= chars.len() {
            let win: String = chars[i..i + nchars].iter().collect();
            if win.to_lowercase() == needle_lower {
                push_part(&mut out, win, true);
                i += nchars;
                continue;
            }
        }
        push_part(&mut out, chars[i].to_string(), false);
        i += 1;
    }
    if out.is_empty() {
        vec![(String::new(), false)]
    } else {
        out
    }
}

fn push_part(out: &mut Vec<(String, bool)>, piece: String, marked: bool) {
    if let Some(last) = out.last_mut()
        && last.1 == marked
    {
        last.0.push_str(&piece);
        return;
    }
    out.push((piece, marked));
}

/// 去掉先前查找 `<mark>`，并按当前查询包一层；`current_id` 为光标所在消息。
pub(crate) fn apply_chat_find_highlights(
    transcript: &HtmlElement,
    needle_lower: &str,
    match_ids: &[String],
    current_id: Option<&str>,
) {
    clear_find_highlights(transcript);
    if needle_lower.is_empty() {
        return;
    }
    for id in match_ids {
        highlight_one_match_wrap(
            transcript,
            id,
            needle_lower,
            current_id == Some(id.as_str()),
        );
    }
}

/// transcript 结构未变、仅 live 气泡被 patch 时，只重包该 wrap，避免每个 token 拆开全文 `<mark>`。
pub(crate) fn reapply_chat_find_highlight_on_wrap(
    transcript: &HtmlElement,
    wrap_id: &str,
    needle_lower: &str,
    match_ids: &[String],
    current_id: Option<&str>,
) {
    if needle_lower.is_empty() || !match_ids.iter().any(|id| id == wrap_id) {
        return;
    }
    highlight_one_match_wrap(
        transcript,
        wrap_id,
        needle_lower,
        current_id == Some(wrap_id),
    );
}

/// DOM 同步后查找高亮的恢复范围（与 overlay revision 解耦）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindRestoreScope {
    None,
    LiveWrap,
    Full,
}

#[must_use]
pub(crate) const fn find_restore_scope(
    structural_dom_change: bool,
    has_live_patch: bool,
) -> FindRestoreScope {
    if structural_dom_change {
        FindRestoreScope::Full
    } else if has_live_patch {
        FindRestoreScope::LiveWrap
    } else {
        FindRestoreScope::None
    }
}

fn highlight_one_match_wrap(
    transcript: &HtmlElement,
    id: &str,
    needle_lower: &str,
    is_current: bool,
) {
    if !is_safe_dom_token(id) {
        return;
    }
    let Ok(Some(wrap)) =
        transcript.query_selector(&format!(".chat-tui-turn-wrap[data-tui-wrap-id=\"{id}\"]"))
    else {
        return;
    };
    unwrap_find_marks(&wrap);
    let _ = wrap.class_list().add_1(WRAP_HIT);
    if is_current {
        let _ = wrap.class_list().add_1(WRAP_CURRENT);
    } else {
        let _ = wrap.class_list().remove_1(WRAP_CURRENT);
    }
    highlight_element_text(&wrap, needle_lower, is_current);
}

fn clear_find_highlights(transcript: &HtmlElement) {
    unwrap_find_marks(transcript);
    if let Ok(hits) = transcript.query_selector_all(".chat-tui-turn-wrap--find-hit") {
        for i in 0..hits.length() {
            if let Some(n) = hits.item(i)
                && let Ok(el) = n.dyn_into::<Element>()
            {
                let _ = el.class_list().remove_1(WRAP_HIT);
                let _ = el.class_list().remove_1(WRAP_CURRENT);
            }
        }
    }
}

fn unwrap_find_marks(root: &Element) {
    let Ok(marks) = root.query_selector_all(&format!("mark.{MARK_CLASS}")) else {
        return;
    };
    for i in 0..marks.length() {
        let Some(node) = marks.item(i) else {
            continue;
        };
        let Some(parent) = node.parent_node() else {
            continue;
        };
        while let Some(child) = node.first_child() {
            let _ = parent.insert_before(&child, Some(&node));
        }
        let _ = parent.remove_child(&node);
    }
    root.unchecked_ref::<Node>().normalize();
}

fn highlight_element_text(root: &Element, needle_lower: &str, is_current_wrap: bool) {
    let mut texts: Vec<Text> = Vec::new();
    collect_text_nodes(root, &mut texts);
    for text in texts {
        highlight_one_text_node(&text, needle_lower, is_current_wrap);
    }
}

fn collect_text_nodes(node: &Node, out: &mut Vec<Text>) {
    if let Some(el) = node.dyn_ref::<Element>() {
        let tag = el.tag_name();
        if tag == "TEXTAREA" || tag == "SCRIPT" || tag == "STYLE" {
            return;
        }
        if el.class_list().contains("chat-user-edit") {
            return;
        }
    }
    if let Ok(text) = node.clone().dyn_into::<Text>() {
        out.push(text);
        return;
    }
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i) {
            collect_text_nodes(&child, out);
        }
    }
}

fn highlight_one_text_node(text: &Text, needle_lower: &str, is_current_wrap: bool) {
    let hay = text.data();
    if hay.is_empty() {
        return;
    }
    let parts = split_highlight_parts(&hay, needle_lower);
    if parts.len() == 1 && !parts[0].1 {
        return;
    }
    let Some(parent) = text.parent_node() else {
        return;
    };
    let Some(doc) = text.owner_document() else {
        return;
    };
    for (piece, marked) in parts {
        if piece.is_empty() {
            continue;
        }
        if marked {
            let Ok(mark) = doc.create_element("mark") else {
                continue;
            };
            let class = if is_current_wrap {
                format!("{MARK_CLASS} {MARK_CURRENT_CLASS}")
            } else {
                MARK_CLASS.to_string()
            };
            mark.set_class_name(&class);
            mark.set_text_content(Some(&piece));
            let _ = parent.insert_before(&mark, Some(text.as_ref()));
        } else {
            let tn = doc.create_text_node(&piece);
            let _ = parent.insert_before(tn.as_ref(), Some(text.as_ref()));
        }
    }
    let _ = parent.remove_child(text.as_ref());
}

#[cfg(test)]
mod tests {
    use super::{FindRestoreScope, find_restore_scope, split_highlight_parts};

    #[test]
    fn split_marks_ascii_case_insensitive() {
        let parts = split_highlight_parts("Hello HELLO", "hello");
        assert_eq!(
            parts,
            vec![
                ("Hello".into(), true),
                (" ".into(), false),
                ("HELLO".into(), true)
            ]
        );
    }

    #[test]
    fn split_no_needle_is_single_plain() {
        assert_eq!(
            split_highlight_parts("abc", ""),
            vec![("abc".into(), false)]
        );
    }

    #[test]
    fn find_restore_scope_skips_full_walk_on_live_patch() {
        assert_eq!(find_restore_scope(false, true), FindRestoreScope::LiveWrap);
        assert_eq!(find_restore_scope(true, true), FindRestoreScope::Full);
        assert_eq!(find_restore_scope(false, false), FindRestoreScope::None);
    }
}
