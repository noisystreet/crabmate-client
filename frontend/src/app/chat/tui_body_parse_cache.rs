//! 流式正文的增量解析：只吸收新增行，已冻结块不重跑 Markdown（pulldown-cmark + ammonia）。

use super::tui_line_markdown::{BlockAbsorbState, TuiBodyChunks, open_plain_for_state};

/// 流式正文的**增量解析缓存**：记住已吸收到行边界的前缀与块状态。
///
/// 流式正文只追加，因此每帧只需把 `source[absorbed..]` 的新行喂给 [`BlockAbsorbState`]，
/// 已冻结块不再重跑 Markdown（pulldown-cmark + ammonia）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiBodyParseCache {
    message_id: String,
    /// 上一帧完整源文本，仅用于校验「新文本仍以旧文本为前缀」。
    source: String,
    /// 已吸收进 `state` 的字节偏移（必为行边界）。
    absorbed: usize,
    state: BlockAbsorbState,
    markdown_render: bool,
}

impl TuiBodyParseCache {
    #[must_use]
    pub(crate) fn new(message_id: &str, markdown_render: bool) -> Self {
        Self {
            message_id: message_id.to_string(),
            source: String::new(),
            absorbed: 0,
            state: BlockAbsorbState::new(markdown_render),
            markdown_render,
        }
    }

    #[must_use]
    pub(crate) fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub(crate) fn markdown_render(&self) -> bool {
        self.markdown_render
    }

    /// 回到「从未解析」状态（文本被改写 / 清空时兜底）。
    fn reset(&mut self) {
        self.source.clear();
        self.absorbed = 0;
        self.state = BlockAbsorbState::new(self.markdown_render);
    }
}

/// 增量解析：只吸收 `cache` 未覆盖的新行，已冻结块直接复用。
///
/// 文本不是上一帧的前缀（被改写 / 重载）时自动回退为从头吸收，结果与
/// [`parse_tui_body_chunks_with`](super::tui_line_markdown::parse_tui_body_chunks_with) 完全一致。
/// `finalize_open_block` 只影响活跃末行，且**不推进**缓存状态（消息完成后不再 live，
/// 避免把收口状态带入下一次渲染）。
#[must_use]
pub(crate) fn parse_tui_body_chunks_incremental(
    cache: &mut TuiBodyParseCache,
    text: &str,
    finalize_open_block: bool,
) -> TuiBodyChunks {
    if text.is_empty() {
        cache.reset();
        return TuiBodyChunks::default();
    }
    if !text.starts_with(cache.source.as_str()) {
        cache.reset();
    }

    let ends_with_nl = text.ends_with('\n');
    let tail_start = if ends_with_nl {
        text.len()
    } else {
        text.rfind('\n').map_or(0, |i| i + 1)
    };
    // 行边界之间吸收完整行（含空行）；末尾未闭合行留给 open_plain。
    for line in text[cache.absorbed..tail_start].split_terminator('\n') {
        cache.state.absorb_complete_line(line);
    }
    cache.absorbed = tail_start;
    cache.source.clear();
    cache.source.push_str(text);

    let open_line = if ends_with_nl {
        None
    } else {
        Some(&text[tail_start..])
    };
    if finalize_open_block && let Some(open) = open_line {
        let mut state = cache.state.clone();
        state.absorb_complete_line(open);
        state.seal_for_finalize();
        return TuiBodyChunks {
            think: None,
            closed: state.closed,
            open_plain: None,
            markdown_render: cache.markdown_render,
        };
    }

    TuiBodyChunks {
        think: None,
        closed: cache.state.closed.clone(),
        open_plain: open_plain_for_state(&cache.state, open_line),
        markdown_render: cache.markdown_render,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tui_line_markdown::parse_tui_body_chunks_with;
    use super::*;

    #[test]
    fn incremental_matches_full_parse_on_every_growth_prefix() {
        let src = "# Title\n\npara one\nline two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n\
                   ```rust\nlet x = 1;\n```\n\ntail **bold** 与中文";
        let mut cache = TuiBodyParseCache::new("m1", true);
        for end in 1..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let text = &src[..end];
            assert_eq!(
                parse_tui_body_chunks_incremental(&mut cache, text, false),
                parse_tui_body_chunks_with(text, false, true),
                "mismatch at byte {end}: {text:?}"
            );
        }
    }

    #[test]
    fn incremental_matches_full_parse_with_markdown_off() {
        let src = "# Title\n\npara\n\n```\ncode\n```\n\ntail **bold**";
        let mut cache = TuiBodyParseCache::new("m2", false);
        for end in 1..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let text = &src[..end];
            assert_eq!(
                parse_tui_body_chunks_incremental(&mut cache, text, false),
                parse_tui_body_chunks_with(text, false, false),
                "mismatch at byte {end}: {text:?}"
            );
        }
    }

    #[test]
    fn incremental_finalize_matches_full_parse_and_stays_stable() {
        let src = "```rust\nlet x = 1;";
        let mut cache = TuiBodyParseCache::new("m3", true);
        let streaming = parse_tui_body_chunks_incremental(&mut cache, src, false);
        assert_eq!(streaming, parse_tui_body_chunks_with(src, false, true));
        let finalized = parse_tui_body_chunks_incremental(&mut cache, src, true);
        assert_eq!(finalized, parse_tui_body_chunks_with(src, true, true));
        assert_eq!(
            parse_tui_body_chunks_incremental(&mut cache, src, true),
            finalized
        );
    }

    #[test]
    fn incremental_falls_back_when_prefix_changes() {
        let mut cache = TuiBodyParseCache::new("m4", true);
        let first = parse_tui_body_chunks_incremental(&mut cache, "alpha\n\nbeta", false);
        assert_eq!(
            first,
            parse_tui_body_chunks_with("alpha\n\nbeta", false, true)
        );
        let rewritten = parse_tui_body_chunks_incremental(&mut cache, "ALPHA\n\nbeta", false);
        assert_eq!(
            rewritten,
            parse_tui_body_chunks_with("ALPHA\n\nbeta", false, true)
        );
    }

    #[test]
    fn incremental_empty_text_resets_cache() {
        let mut cache = TuiBodyParseCache::new("m5", true);
        let _ = parse_tui_body_chunks_incremental(&mut cache, "a\n\nb", false);
        let empty = parse_tui_body_chunks_incremental(&mut cache, "", false);
        assert_eq!(empty, TuiBodyChunks::default());
        let again = parse_tui_body_chunks_incremental(&mut cache, "a\n\nb", false);
        assert_eq!(again, parse_tui_body_chunks_with("a\n\nb", false, true));
    }
}
