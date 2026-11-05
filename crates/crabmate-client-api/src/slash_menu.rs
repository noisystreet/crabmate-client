//! 跨端 `/` 补全菜单的纯逻辑（触发判定 / 候选组装 / 前缀过滤）。
//!
//! 两端原本各有一份逐字同构的实现 —— Web 在 `frontend/src/app/chat/composer_slash_menu.rs`，
//! TUI 在 `crates/crabmate-tui/src/tui_mode/slash_menu.rs`：触发条件、`MenuItem` 字段、
//! skill 项规则、「内建在前 + skill 按 id 排序 + 截断」全部一样。这里只收这些**机制**：
//!
//! - 命令**文案**留在端内（Web 走 i18n，TUI 是编译期常量）；
//! - skill **来源**两端的类型不同（`SkillListItem` / `SkillEntry`），由端内 map 成 [`MenuItem`]；
//! - 拦截判定（`is_*_control_slash`）**不**共享：两端 handler 集不同，合成一个超集判定就会
//!   出现「拦下来但没有 handler」，见 [`super::slash`] 的模块注释。

/// 候选来源：端内命令 / skill。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Builtin,
    Skill,
}

/// 菜单里的一个候选。两端字段一一对应，渲染层各自消费。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub kind: MenuKind,
    /// 前缀匹配键（不含 `/`）。
    pub match_key: String,
    /// 接受后写进输入框的全文。
    pub insert: String,
    /// 左侧展示（含 `/`）。
    pub label: String,
    pub description: String,
}

/// skill 候选上限：避免几百个 skill 把菜单撑成搜索页。
pub const MAX_SKILL_ROWS: usize = 24;

impl MenuItem {
    /// 内建命令项：`label` 由 `key` 推出；`insert` 由调用方给 —— 可能带尾随空格，
    /// 表示「接受后光标落在参数位」，与只有命令名的项必须能区分。
    #[must_use]
    pub fn builtin(key: &str, insert: &str, description: &str) -> Self {
        Self {
            kind: MenuKind::Builtin,
            match_key: key.to_string(),
            insert: insert.to_string(),
            label: format!("/{key}"),
            description: description.to_string(),
        }
    }

    /// skill 项：`description` 空白时回退 skill 的**名字**，名字与左侧 `/id` 相同时留空。
    ///
    /// 回退整条 `path` 会让每行都长成 `skills/<id>/SKILL.md` —— 又长又行行相同，反而认不出是
    /// 哪个 skill；名字（`skills/api-design/SKILL.md` → `api-design`）通常又正好等于 [`Self::label`]
    /// 里的 `id`，再写一遍只是把同一个名字印两次。所以只在名字**不同于** id 时才填第二列
    /// （例如 `skills/other/SKILL.md` 配 id `x`），否则留空让菜单只剩 `/alpha`。
    #[must_use]
    pub fn skill(id: &str, description: &str, path: &str) -> Self {
        let desc = if description.trim().is_empty() {
            let name = crate::timeline_skill::skill_display_name(path).unwrap_or_default();
            if name == id { String::new() } else { name }
        } else {
            description.trim().to_string()
        };
        Self {
            kind: MenuKind::Skill,
            match_key: id.to_string(),
            insert: format!("/{id} "),
            label: format!("/{id}"),
            description: desc,
        }
    }
}

/// 草稿是否处于「仅 `/` + 可选前缀、尚无空白」的浮层态；返回 `/` 之后的前缀。
///
/// 一旦出现空白或第二个 `/` 就按普通输入处理 —— `/conv use c1` 的参数段、
/// `/data/proj` 这类路径都不该被菜单挡住。
#[must_use]
pub fn slash_prefix(draft: &str) -> Option<&str> {
    let s = draft.trim_start();
    let rest = s.strip_prefix('/')?;
    if rest.contains(char::is_whitespace) || rest.contains('/') {
        return None;
    }
    Some(rest)
}

/// 前缀过滤（大小写不敏感；空前缀全过）。
///
/// 只比 `match_key`：任何 [`MenuItem`] 的 `label` 都是 `/{match_key}`，Web 旧实现里
/// 「再比一次去掉 `/` 的 label」在现有数据下与这一条等价，故合并成一条。
#[must_use]
pub fn matches_prefix(item: &MenuItem, prefix: &str) -> bool {
    prefix.is_empty()
        || item
            .match_key
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
}

/// 组装候选：内建（按传入顺序）在前，skill 按 id 排序并截断到 [`MAX_SKILL_ROWS`]。
#[must_use]
pub fn assemble(
    prefix: &str,
    builtin: impl IntoIterator<Item = MenuItem>,
    skills: impl IntoIterator<Item = MenuItem>,
) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = builtin
        .into_iter()
        .filter(|it| matches_prefix(it, prefix))
        .collect();
    let mut skill_rows: Vec<MenuItem> = skills
        .into_iter()
        .filter(|it| matches_prefix(it, prefix))
        .collect();
    skill_rows.sort_by(|a, b| {
        a.match_key
            .to_ascii_lowercase()
            .cmp(&b.match_key.to_ascii_lowercase())
    });
    skill_rows.truncate(MAX_SKILL_ROWS);
    out.extend(skill_rows);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(key: &str) -> MenuItem {
        MenuItem::builtin(key, &format!("/{key} "), "d")
    }

    fn skill(id: &str) -> MenuItem {
        MenuItem::skill(id, "", &format!("skills/{id}/SKILL.md"))
    }

    #[test]
    fn prefix_requires_bare_slash_without_space() {
        assert_eq!(slash_prefix("/"), Some(""));
        assert_eq!(slash_prefix("  /he"), Some("he"));
        // 出现空白或第二个 `/` 就不再是菜单态（参数段 / 路径都走这条路）。
        assert_eq!(slash_prefix("/conv use c1"), None);
        assert_eq!(slash_prefix("/data/proj"), None);
        assert_eq!(slash_prefix("hello"), None);
        assert_eq!(slash_prefix(""), None);
    }

    #[test]
    fn matches_prefix_is_case_insensitive_and_empty_means_all() {
        let it = builtin("model");
        assert!(matches_prefix(&it, ""));
        assert!(matches_prefix(&it, "MO"));
        assert!(matches_prefix(&it, "model"));
        assert!(!matches_prefix(&it, "mode "));
        assert!(!matches_prefix(&it, "x"));
    }

    #[test]
    fn builtin_item_labels_from_key_and_keeps_insert_verbatim() {
        let it = MenuItem::builtin("workspace", "/workspace ", "切换");
        assert_eq!(it.kind, MenuKind::Builtin);
        assert_eq!(it.label, "/workspace");
        assert_eq!(it.match_key, "workspace");
        // `insert` 原样保留：尾随空格决定接受后光标是否落在参数位。
        assert_eq!(it.insert, "/workspace ");
    }

    #[test]
    fn skill_item_falls_back_to_a_name_that_differs_from_its_id() {
        let with_desc = MenuItem::skill("a", " 说明 ", "p");
        assert_eq!(with_desc.description, "说明");
        assert_eq!(with_desc.insert, "/a ");
        assert_eq!(with_desc.label, "/a");

        // 名字与 `id` 不同：第二列仍是有效的补充信息（不是整条路径）。
        let blank = MenuItem::skill("x", "   ", "skills/other/SKILL.md");
        assert_eq!(blank.description, "other");
        assert_eq!(blank.kind, MenuKind::Skill);

        // 名字就是 `id`：再画一遍只是 `/b  b`，留空。
        let same = MenuItem::skill("b", "   ", "skills/b/SKILL.md");
        assert_eq!(same.description, "");
        assert_eq!(same.label, "/b");
    }

    #[test]
    fn assemble_puts_builtins_first_and_sorts_skills() {
        let items = assemble(
            "",
            vec![builtin("help"), builtin("model")],
            vec![skill("zeta"), skill("alpha")],
        );
        let labels: Vec<&str> = items.iter().map(|it| it.label.as_str()).collect();
        assert_eq!(labels, vec!["/help", "/model", "/alpha", "/zeta"]);
        // 内建保持传入顺序（端内自己排），不被字典序重排。
        assert_eq!(items[0].kind, MenuKind::Builtin);
        assert_eq!(items[2].kind, MenuKind::Skill);
    }

    #[test]
    fn assemble_filters_both_kinds_by_prefix() {
        let items = assemble("he", vec![builtin("help")], vec![skill("helper")]);
        let labels: Vec<&str> = items.iter().map(|it| it.label.as_str()).collect();
        assert_eq!(labels, vec!["/help", "/helper"]);

        let none = assemble("zzz", vec![builtin("help")], vec![skill("alpha")]);
        assert!(none.is_empty());
    }

    #[test]
    fn assemble_truncates_skill_rows_only() {
        let skills: Vec<MenuItem> = (0..MAX_SKILL_ROWS + 5)
            .map(|i| skill(&format!("s{i:02}")))
            .collect();
        let items = assemble("", vec![builtin("help")], skills);
        // 内建不受截断影响，skill 到上限为止。
        assert_eq!(items.len(), 1 + MAX_SKILL_ROWS);
        assert_eq!(items[0].label, "/help");
        assert_eq!(items[1].label, "/s00");
        assert_eq!(
            items[MAX_SKILL_ROWS].label,
            format!("/s{:02}", MAX_SKILL_ROWS - 1)
        );
    }
}
