//! **一条 lane 该不该新建**的判定（纯逻辑，无 IO）。
//!
//! TUI 与 Web 都要回答同一个问题：`/lane attach`（或点「连接」某一行）时，是**新开一条**、
//! **切到已有的那条**、还是**被上限挡住**？三端的答案必须一致，否则同一台 `serve` 在 TUI 里
//! 判「同一条」、在 Web 里判「两条」，症状是同一台 serve 各开一条 SSE、各读一份会话。
//!
//! 判定顺序是有讲究的（照 TUI `attach_lane` 的实测结论）：
//!
//! 1. **地址形态**先拦 —— `api_base` 不是绝对 `http(s)` 时直接拒，不去重、不占名额。
//!    漏写 `http://` 极常见，放过去只会造出一条 `api_base = ""` 的「已连接」lane，
//!    之后每个请求才以「API base must be an absolute http(s) URL」失败。
//! 2. **同名**只切过去 —— 手误（`--connect a=x --connect a=y`）若静默再连一次，列表里会出现
//!    两条同名 lane，而控制面只有一条实例，之后所有判断都会错。
//! 3. **同 `api_base`** 也只切过去 —— 名字不同不代表是两台 serve。
//! 4. **名额检查放在两道去重之后** —— 已满时「切到现有那条」仍要能成功；被上限挡住会让用户
//!    以为「连接坏了」。那并不新增线程或连接。
//!
//! 文案不在这里：本模块只回答「该做什么」，措辞由各端决定（TUI 写 shell 行，Web 弹提示）。

use crate::serve_identity::{is_absolute_api_base, same_api_base};

/// 一条**已存在的** lane 的判定入参（只取判定需要的两个字段）。
#[derive(Debug, Clone, Copy)]
pub struct LaneSlot<'a> {
    pub label: &'a str,
    pub api_base: &'a str,
}

impl<'a> LaneSlot<'a> {
    #[must_use]
    pub fn new(label: &'a str, api_base: &'a str) -> Self {
        Self { label, api_base }
    }
}

/// [`decide_attach`] 的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachDecision {
    /// 新开一条 lane。
    Append,
    /// 切到已有那条（同名）。
    SwitchToSameName { index: usize },
    /// 切到已有那条（同一个 `api_base`，名字不同）。
    SwitchToSameBase { index: usize },
    /// 名额已满；`current` 是现有条数。
    CapacityFull { current: usize },
    /// 地址不是绝对 `http(s)` URL。
    InvalidBase,
}

/// 判定这次 attach 该做什么。
///
/// `max_lanes` 由调用方给（TUI 是 [`MAX_LANES`]，Web 沿用同一值），因为上限本身是**产品口径**
/// 而不是协议约束 —— 但两端的默认值应当相同，见 [`MAX_LANES`] 的注释。
#[must_use]
pub fn decide_attach(
    existing: &[LaneSlot<'_>],
    name: &str,
    base_url: &str,
    max_lanes: usize,
) -> AttachDecision {
    if !is_absolute_api_base(base_url) {
        return AttachDecision::InvalidBase;
    }
    if let Some(index) = existing.iter().position(|l| l.label == name) {
        return AttachDecision::SwitchToSameName { index };
    }
    if let Some(index) = existing
        .iter()
        .position(|l| same_api_base(l.api_base, base_url))
    {
        return AttachDecision::SwitchToSameBase { index };
    }
    if existing.len() >= max_lanes {
        return AttachDecision::CapacityFull {
            current: existing.len(),
        };
    }
    AttachDecision::Append
}

/// lane 数量上限的默认值（与 TUI 的 `MAX_LANES` 同源）。
///
/// 上限是**可用性**口径：每条 lane 一个 worker 线程 + 一条 SSE，8 条已经够看几路；
/// 再高只会让左栏列表本身变成噪音。
pub const MAX_LANES: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn slots() -> Vec<LaneSlot<'static>> {
        vec![
            LaneSlot::new("local", "http://127.0.0.1:8081"),
            LaneSlot::new("proj-b", "http://127.0.0.1:8721"),
        ]
    }

    #[test]
    fn a_new_address_appends() {
        assert_eq!(
            decide_attach(&slots(), "proj-c", "http://127.0.0.1:9000", MAX_LANES),
            AttachDecision::Append
        );
    }

    #[test]
    fn the_same_name_only_switches() {
        assert_eq!(
            decide_attach(&slots(), "proj-b", "http://127.0.0.1:9999", MAX_LANES),
            AttachDecision::SwitchToSameName { index: 1 }
        );
    }

    #[test]
    fn the_same_base_with_another_name_only_switches() {
        // 尾斜杠 / 大小写不参与比较（见 `serve_identity`）。
        for url in [
            "http://127.0.0.1:8721/",
            "HTTP://127.0.0.1:8721",
            "  http://127.0.0.1:8721  ",
        ] {
            assert_eq!(
                decide_attach(&slots(), "另一个名字", url, MAX_LANES),
                AttachDecision::SwitchToSameBase { index: 1 },
                "{url}"
            );
        }
    }

    #[test]
    fn localhost_is_not_the_same_as_the_loopback_ip() {
        assert_eq!(
            decide_attach(&slots(), "别名", "http://localhost:8081", MAX_LANES),
            AttachDecision::Append
        );
    }

    #[test]
    fn a_malformed_base_is_rejected_before_anything_else() {
        // 即使同名已存在也先拦地址：调用方不该因为「名字撞上了」就静默切过去。
        for bad in ["", "   ", "127.0.0.1:8081", "ws://a:1"] {
            assert_eq!(
                decide_attach(&slots(), "proj-b", bad, MAX_LANES),
                AttachDecision::InvalidBase,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn capacity_is_checked_after_both_dedupes() {
        // 已满时：切到现有那条仍然要成功；只有真正新增才被挡。
        let full: Vec<LaneSlot<'static>> = (0..MAX_LANES)
            .map(|_| LaneSlot::new("x", "http://h:1")) // 内容不重要，只数条数
            .collect();
        assert_eq!(
            decide_attach(&full, "新名字", "http://127.0.0.1:9999", MAX_LANES),
            AttachDecision::CapacityFull { current: MAX_LANES }
        );

        let mut at_limit = full.clone();
        at_limit[0] = LaneSlot::new("local", "http://127.0.0.1:8081");
        assert_eq!(
            decide_attach(&at_limit, "别的名字", "http://127.0.0.1:8081", MAX_LANES),
            AttachDecision::SwitchToSameBase { index: 0 },
            "满了也要能切到同一条"
        );
        assert_eq!(
            decide_attach(&at_limit, "local", "http://127.0.0.1:7777", MAX_LANES),
            AttachDecision::SwitchToSameName { index: 0 }
        );
    }

    #[test]
    fn an_empty_set_appends_even_at_max_one() {
        assert_eq!(
            decide_attach(&[], "首条", "http://127.0.0.1:8081", 1),
            AttachDecision::Append
        );
    }
}
