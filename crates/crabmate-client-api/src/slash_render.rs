//! `/skills` 的行渲染：两端共用「抬头 + `  /id — 描述` + 截断尾行」这套格式。
//!
//! 输出 `Vec<String>` 而不是拼好的 `String` —— TUI 一次写多行（`push_lines`），Web 自己
//! `join("\n")` 塞进一条 system 气泡，两端的观感差异因此不受影响。
//!
//! **空列表不归这里**：两端对「为什么空」的解释不同（Web 会带上 serve 的 `skills_dir`，
//! TUI 会带上 cache 里记下的错误串），那部分留在端内。本模块只负责「已经确定非空之后
//! 怎么排」。

/// 一行 skill。
///
/// 两端的 skill 类型不同（Web 是 `SkillListItem`、TUI 是 `SkillEntry`），由端内 map 成
/// 这个中性结构 —— 与 `slash_menu` 处理「skill 来源留端」是同一手法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillRow<'a> {
    pub id: &'a str,
    pub description: &'a str,
}

/// 最多印多少条。
///
/// 与 `/` 菜单的候选上限同位：用户在菜单里能看到的条数与命令里列出的条数一致，
/// 否则「菜单里有、命令里没有」会被当成坏数据。
pub const MAX_LISTED_SKILLS: usize = 24;

/// 非空列表 → 行。`rows` 为空时返回空 `Vec`（空态文案由端内负责）。
#[must_use]
pub fn skill_lines(rows: &[SkillRow<'_>]) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(rows.len().min(MAX_LISTED_SKILLS) + 2);
    lines.push(format!("skills {} 个（用 /<id> 调用）：", rows.len()));
    for row in rows.iter().take(MAX_LISTED_SKILLS) {
        let desc = row.description.trim();
        if desc.is_empty() {
            // 描述为空时不留破折号：`/id — ` 末尾悬着分隔符会被读成「描述缺失」之外的
            // 东西（Web 此前就是这样印的）。
            lines.push(format!("  /{}", row.id));
        } else {
            lines.push(format!("  /{} — {}", row.id, desc));
        }
    }
    if rows.len() > MAX_LISTED_SKILLS {
        lines.push(format!(
            "  …还有 {} 个（`/` 菜单里可以按前缀过滤）",
            rows.len() - MAX_LISTED_SKILLS
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &'static str, description: &'static str) -> SkillRow<'static> {
        SkillRow { id, description }
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(skill_lines(&[]).is_empty());
    }

    #[test]
    fn header_carries_the_total_and_the_items_follow() {
        let lines = skill_lines(&[row("alpha", "第一条"), row("beta", "第二条")]);
        assert_eq!(lines[0], "skills 2 个（用 /<id> 调用）：");
        assert_eq!(lines[1], "  /alpha — 第一条");
        assert_eq!(lines[2], "  /beta — 第二条");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn empty_description_leaves_no_dangling_dash() {
        let lines = skill_lines(&[row("bare", ""), row("blank", "   ")]);
        assert_eq!(lines[1], "  /bare");
        assert_eq!(lines[2], "  /blank");
    }

    #[test]
    fn truncation_reports_the_remainder() {
        let many: Vec<SkillRow> = (0..MAX_LISTED_SKILLS + 3)
            .map(|_| SkillRow {
                id: "s",
                description: "d",
            })
            .collect();
        let lines = skill_lines(&many);
        // 抬头 + 上限条数 + 尾行
        assert_eq!(lines.len(), MAX_LISTED_SKILLS + 2);
        assert_eq!(
            lines[MAX_LISTED_SKILLS + 1],
            "  …还有 3 个（`/` 菜单里可以按前缀过滤）"
        );
    }

    #[test]
    fn exactly_at_the_limit_has_no_remainder_line() {
        let rows: Vec<SkillRow> = (0..MAX_LISTED_SKILLS)
            .map(|_| SkillRow {
                id: "s",
                description: "d",
            })
            .collect();
        let lines = skill_lines(&rows);
        assert_eq!(lines.len(), MAX_LISTED_SKILLS + 1);
    }
}
