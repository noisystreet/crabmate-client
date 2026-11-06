//! 澄清问卷的**作答状态**（Web 与 TUI 共用一份口径）。
//!
//! 分两半：
//!
//! 1. [`PendingClarification`]：SSE 下发后待填的那份题面 + 每题草稿；
//! 2. [`PendingClarification::answers_json`]：必填校验 + 按题目 `id` 序列化成
//!    `chat` 请求体的 `clarify_questionnaire_answers`。
//!
//! 第 2 步是从 Web 的 `composer_wires/helpers.rs` 搬过来的 —— 两端各写一遍的话，
//! 「`trim` 后算不算空」「required 缺一题就整份不发」这类判定迟早分叉，而分叉的表现是
//! 同一份问卷在 Web 能提交、在 TUI 提交不上去（或反过来），用户只会觉得「这个表单坏了」。
//!
//! 纯计算：不碰 IO、不碰 UI，也不做 i18n —— 错误只给出「哪一题」，文案由各端自己拼
//! （Web 有既有 i18n 串，TUI 要连题目名一起说）。

use crate::sse_dispatch::{ClarificationFormField, ClarificationQuestionnaireInfo};

/// 一份待填问卷。
#[derive(Debug, Clone)]
pub struct PendingClarification {
    pub questionnaire_id: String,
    pub intro: String,
    pub fields: Vec<ClarificationFormField>,
    /// 与 [`Self::fields`] 等长；用户逐题的草稿（未 trim，提交时才 trim）。
    pub values: Vec<String>,
}

/// 提交校验失败的**唯一**原因：某道必答题是空的。
///
/// 带 `index` / `label` 而不只是布尔：TUI 要把光标跳到那一题并说出题目名，
/// 只说「有必答题没填」等于让用户从头找一遍。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingRequired {
    /// 在 `fields` 里的下标（0-based）。
    pub index: usize,
    pub id: String,
    pub label: String,
}

impl std::fmt::Display for MissingRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "必答题「{}」还没填", self.label)
    }
}

/// 必填校验 + 按题目 `id` 序列化。
///
/// 自由函数形态是为了让**两端都能直接用**：TUI 持 [`PendingClarification`]，
/// Web 的 `PendingClarificationForm` 是 leptos 信号里的另一份适配结构 —— 让它俩
/// 各自持有字段、共用这一个判定入口，比强行统一数据结构改动面小得多
/// （Web 那侧还有 UI 渲染按 `values[i]` 逐题绑定）。
///
/// 形状与契约 `ClarifyQuestionnaireAnswersBody` 一致：
/// `{ "questionnaire_id": …, "answers": { "<题目 id>": "<值>" } }`。
pub fn answers_json(
    questionnaire_id: &str,
    fields: &[ClarificationFormField],
    values: &[String],
) -> Result<serde_json::Value, MissingRequired> {
    if let Some(missing) = first_missing_required(fields, values) {
        return Err(missing);
    }
    let mut answers = serde_json::Map::new();
    for (i, f) in fields.iter().enumerate() {
        let raw = values.get(i).map(String::as_str).unwrap_or("");
        answers.insert(
            f.id.clone(),
            serde_json::Value::String(raw.trim().to_string()),
        );
    }
    Ok(serde_json::json!({
        "questionnaire_id": questionnaire_id,
        "answers": serde_json::Value::Object(answers),
    }))
}

/// 第一道「必答但空着」的题（`trim` 后判空）；全填齐则 `None`。
#[must_use]
pub fn first_missing_required(
    fields: &[ClarificationFormField],
    values: &[String],
) -> Option<MissingRequired> {
    fields
        .iter()
        .enumerate()
        .find(|(i, f)| f.required && values.get(*i).map(|v| v.trim().is_empty()).unwrap_or(true))
        .map(|(index, f)| MissingRequired {
            index,
            id: f.id.clone(),
            label: f.label.clone(),
        })
}

impl PendingClarification {
    /// 从 SSE 下发体构造：草稿全空。
    #[must_use]
    pub fn from_sse(info: ClarificationQuestionnaireInfo) -> Self {
        let n = info.fields.len();
        Self {
            questionnaire_id: info.questionnaire_id,
            intro: info.intro,
            fields: info.fields,
            values: vec![String::new(); n],
        }
    }
    /// 已填题数（trim 后非空）—— 给「3/5 已填」这类进度显示用。
    #[must_use]
    pub fn filled_count(&self) -> usize {
        self.values
            .iter()
            .filter(|v| !v.trim().is_empty())
            .take(self.fields.len())
            .count()
    }

    /// 第一道「必答但空着」的题；全填齐则 `None`。
    #[must_use]
    pub fn first_missing_required(&self) -> Option<MissingRequired> {
        first_missing_required(&self.fields, &self.values)
    }

    /// 序列化成 `chat` 请求体的 `clarify_questionnaire_answers` 值。
    ///
    /// 必答题缺失时**整份不发**（`Err`）而不是发一半：半份答案在服务端读起来像
    /// 「用户对没填的题回答空」，而实际是他还没填完。
    pub fn answers_json(&self) -> Result<serde_json::Value, MissingRequired> {
        answers_json(&self.questionnaire_id, &self.fields, &self.values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse_dispatch::ClarificationKind;

    fn field(id: &str, label: &str, required: bool) -> ClarificationFormField {
        ClarificationFormField {
            id: id.to_string(),
            label: label.to_string(),
            hint: None,
            required,
            kind: ClarificationKind::Text,
        }
    }

    fn form(fields: Vec<ClarificationFormField>) -> PendingClarification {
        PendingClarification::from_sse(ClarificationQuestionnaireInfo {
            questionnaire_id: "q1".into(),
            intro: "说明".into(),
            fields,
        })
    }

    #[test]
    fn from_sse_keeps_one_empty_draft_per_question() {
        let f = form(vec![field("a", "A", true), field("b", "B", false)]);
        assert_eq!(f.values.len(), 2);
        assert!(f.values.iter().all(String::is_empty));
        assert_eq!(f.filled_count(), 0);
    }

    #[test]
    fn answers_align_by_question_id() {
        let mut f = form(vec![field("a", "A", true), field("b", "B", false)]);
        f.values[0] = "  答案甲  ".into();
        f.values[1] = "答案乙".into();
        let v = f.answers_json().expect("必答已填");
        assert_eq!(v["questionnaire_id"], "q1");
        assert_eq!(v["answers"]["a"], "答案甲", "两端都要 trim");
        assert_eq!(v["answers"]["b"], "答案乙");
    }

    #[test]
    fn missing_required_blocks_the_whole_submission() {
        let mut f = form(vec![field("a", "名字", true), field("b", "备注", false)]);
        f.values[0] = "   ".into(); // 只有空白 = 没填
        let err = f.answers_json().expect_err("必答空着不能提交");
        assert_eq!(err.id, "a");
        assert_eq!(err.index, 0);
        assert_eq!(err.label, "名字", "错误要能说出是哪一题");
        assert!(err.to_string().contains("名字"));
    }

    #[test]
    fn optional_blank_answers_still_go_out() {
        let mut f = form(vec![field("a", "A", true), field("b", "B", false)]);
        f.values[0] = "x".into();
        let v = f.answers_json().expect("可选空着不影响提交");
        assert_eq!(v["answers"]["b"], "", "空的可选项也要出现（键齐才好对齐）");
    }

    #[test]
    fn filled_count_ignores_whitespace() {
        let mut f = form(vec![field("a", "A", false), field("b", "B", false)]);
        f.values[0] = "  ".into();
        f.values[1] = "y".into();
        assert_eq!(f.filled_count(), 1);
    }
}
