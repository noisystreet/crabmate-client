//! LLM 网关预设表（`client_llm.api_base` 的下拉 / 循环选项）。
//!
//! 渊源：与 Server `cm_types::llm_gateway_presets` 同源。本仓两端此前各持一份
//! **逐字相同**的 `id`+`url` 表——`frontend/src/client_llm_presets.rs` 与
//! `crates/crabmate-tui/src/tui_mode/settings_panel_schema.rs`，后者注释直接写着
//! 「同源拷贝；改 URL / 增删条目需两仓同步」。用注释管理漂移正是该消除的东西，
//! 故收拢为单一来源。
//!
//! 只存 `id` + `url`。端专属的东西留端：Web 的「选此项时建议模型名」是展示行为，
//! TUI 的「循环选择下标」由 [`index_for_url`] 投影得到。

/// 单条预设：`id` 用于 UI 与持久层标签；`url` 写入 `api_base`。
///
/// `server` 与 `custom` 的 `url` 都是空串——两者靠 `id` 与**表位置**区分，
/// 语义见 [`select_id_for_url`] / [`index_for_url`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub url: &'static str,
}

/// 「跟随 server」项：`url` 为空串，且必须是本表**首项**（[`index_for_url`] 依赖）。
pub const SERVER_ID: &str = "server";
/// 「自定义 URL」项：`url` 为空串，且必须是本表**末项**（[`index_for_url`] 依赖）。
pub const CUSTOM_ID: &str = "custom";

/// 常用 OpenAI 兼容网关（与 Server 预设表、README 供应商示例一致）。
pub const PRESETS: &[Preset] = &[
    Preset {
        id: SERVER_ID,
        url: "",
    },
    Preset {
        id: "ollama",
        url: "http://127.0.0.1:11434/v1",
    },
    Preset {
        id: "deepseek",
        url: "https://api.deepseek.com/v1",
    },
    Preset {
        id: "minimax",
        url: "https://api.minimaxi.com/v1",
    },
    Preset {
        id: "zhipu",
        url: "https://open.bigmodel.cn/api/paas/v4",
    },
    Preset {
        id: "moonshot",
        url: "https://api.moonshot.cn/v1",
    },
    Preset {
        id: CUSTOM_ID,
        url: "",
    },
];

/// `id` 是否为两个空 `url` 的占位项（`server` / `custom`）。
#[must_use]
pub fn is_placeholder_id(id: &str) -> bool {
    id == SERVER_ID || id == CUSTOM_ID
}

/// `api_base` → 具名预设 id（不含 `server` / `custom`）；未命中为 `None`。
///
/// 完全相等才命中，不做前缀 / 尾斜杠归一（`api_base` 由各端自己规范化）。
#[must_use]
pub fn display_id_for_url(url: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|p| !is_placeholder_id(p.id) && p.url == url)
        .map(|p| p.id)
}

/// 草稿 `api_base` → 选中项 id（Web `<select>` 值语义）。
///
/// `trim` 后为空 → [`SERVER_ID`]；与具名预设 `url` 完全一致 → 该 id；
/// 其余一律 [`CUSTOM_ID`]。
#[must_use]
pub fn select_id_for_url(draft: &str) -> &'static str {
    let t = draft.trim();
    if t.is_empty() {
        return SERVER_ID;
    }
    display_id_for_url(t).unwrap_or(CUSTOM_ID)
}

/// 草稿 `api_base` → 选项下标（TUI 面板 `pick` 语义）。
///
/// `None` / 空白 → `0`（[`SERVER_ID`] 在首位）；与具名预设 `url` 一致 → 该下标；
/// 其余 → 末位（[`CUSTOM_ID`]）。
#[must_use]
pub fn index_for_url(value: Option<&str>) -> usize {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        None => 0,
        Some(v) => PRESETS
            .iter()
            .position(|p| !is_placeholder_id(p.id) && p.url == v)
            .unwrap_or(PRESETS.len() - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `index_for_url` 把「空 → 0」「未知 → 末位」当作 server / custom 的代理，
    /// 所以表首尾必须是这两个占位项。
    #[test]
    fn placeholders_are_first_and_last() {
        assert_eq!(PRESETS.first().map(|p| p.id), Some(SERVER_ID));
        assert_eq!(PRESETS.last().map(|p| p.id), Some(CUSTOM_ID));
        assert_eq!(PRESETS.first().map(|p| p.url), Some(""));
        assert_eq!(PRESETS.last().map(|p| p.url), Some(""));
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                assert_ne!(a.id, b.id, "duplicate preset id: {}", a.id);
            }
        }
    }

    /// 具名预设的 `url` 必须非空且互不相同，否则反查会命中错误条目。
    #[test]
    fn named_urls_are_unique_and_non_empty() {
        let named: Vec<&str> = PRESETS
            .iter()
            .filter(|p| !is_placeholder_id(p.id))
            .map(|p| p.url)
            .collect();
        assert!(!named.is_empty());
        assert!(named.iter().all(|u| !u.is_empty()));
        for (i, a) in named.iter().enumerate() {
            assert!(!named[i + 1..].contains(a), "duplicate preset url: {a}");
        }
    }

    #[test]
    fn select_blank_is_server() {
        assert_eq!(select_id_for_url(""), SERVER_ID);
        assert_eq!(select_id_for_url("   "), SERVER_ID);
    }

    #[test]
    fn select_matches_named_url() {
        assert_eq!(select_id_for_url("https://api.deepseek.com/v1"), "deepseek");
        assert_eq!(select_id_for_url("http://127.0.0.1:11434/v1"), "ollama");
        // 前后空白由 trim 吃掉
        assert_eq!(
            select_id_for_url("  https://api.moonshot.cn/v1  "),
            "moonshot"
        );
    }

    #[test]
    fn select_unknown_is_custom() {
        assert_eq!(select_id_for_url("https://example.com/v1"), CUSTOM_ID);
        // 只差一个尾斜杠也不算命中：归一化不是本函数的职责
        assert_eq!(select_id_for_url("https://api.deepseek.com/v1/"), CUSTOM_ID);
    }

    #[test]
    fn index_none_or_blank_is_first() {
        assert_eq!(index_for_url(None), 0);
        assert_eq!(index_for_url(Some("")), 0);
        assert_eq!(index_for_url(Some("  ")), 0);
    }

    #[test]
    fn index_matches_named_url() {
        let expected = PRESETS
            .iter()
            .position(|p| p.id == "deepseek")
            .expect("deepseek present");
        assert_eq!(index_for_url(Some("https://api.deepseek.com/v1")), expected);
    }

    #[test]
    fn index_unknown_is_last() {
        assert_eq!(
            index_for_url(Some("https://example.com/v1")),
            PRESETS.len() - 1
        );
    }

    #[test]
    fn display_id_ignores_placeholders() {
        assert_eq!(display_id_for_url(""), None);
        assert_eq!(
            display_id_for_url("https://api.deepseek.com/v1"),
            Some("deepseek")
        );
    }

    /// select 与 index 必须对同一输入给出一致结论（两个端各自的投影不能打架）。
    #[test]
    fn select_and_index_agree_on_named_presets() {
        for p in PRESETS.iter().filter(|p| !is_placeholder_id(p.id)) {
            assert_eq!(select_id_for_url(p.url), p.id);
            assert_eq!(PRESETS[index_for_url(Some(p.url))].id, p.id);
        }
    }
}
