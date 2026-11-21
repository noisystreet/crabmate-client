//! 切 lane 的键位（#561 **S3**），与 TUI 同一张表（设计文档 §6.6）。
//!
//! | 键 | 行为 | 单 lane 时 |
//! |---|---|---|
//! | `Alt+→` / `Ctrl+N` | 下一条（循环） | no-op |
//! | `Alt+←` | 上一条 | no-op |
//! | `Alt+1..8` | 直达第 n 条（1-based，越界不切） | no-op |
//! | `F4` | 下一条**在等审批**的 lane | no-op |
//!
//! 三条口径与 TUI 对齐：
//!
//! - **精确匹配修饰符**，不放宽成 `contains`：`Alt+Shift+→` 不算，否则会和窗口管理器的组合键
//!   撞车（`Alt+数字` 在部分 Linux 桌面已被 WM 占用，这条要进冒烟检查项）。
//! - **全局监听**：切换键不能漏进输入框 —— TUI 也是在进入 lane 之前拦住。但只在**真的切了**
//!   才 `preventDefault`：越界 / 单 lane 时没有动作，就别顺手把默认行为吞掉。
//! - 单 lane 时全部 no-op：`LaneSet` 的三个方法自带这条判断，所以这里不必再问「有几条」。
//!
//! 监听随页面存活（`forget`），与 `layout_toggle_hotkey` 同一处置：它绑的是窗口级信号，
//! 没有「某个组件卸载就该摘掉」的语义。

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::app_signals::LaneSet;

/// 一次按键要做的 lane 动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneIntent {
    Next,
    Prev,
    Goto(usize),
    FocusPendingApproval,
}

/// 按键的修饰符组合（从 `KeyboardEvent` 提出来，好让判定成为纯函数）。
#[derive(Clone, Copy, Debug, Default)]
struct KeyMods {
    alt: bool,
    ctrl: bool,
    meta: bool,
    shift: bool,
    composing: bool,
}

impl KeyMods {
    fn alt_only(self) -> bool {
        self.alt && !self.ctrl && !self.meta && !self.shift
    }

    fn ctrl_only(self) -> bool {
        self.ctrl && !self.alt && !self.meta && !self.shift
    }

    fn bare(self) -> bool {
        !self.alt && !self.ctrl && !self.meta && !self.shift
    }
}

/// `Alt` 家族：方向键切相对，数字键直达（1-based）。
fn alt_intent(key: &str) -> Option<LaneIntent> {
    match key {
        "ArrowRight" => return Some(LaneIntent::Next),
        "ArrowLeft" => return Some(LaneIntent::Prev),
        _ => {}
    }
    let n = key.parse::<usize>().ok()?;
    (1..=8).contains(&n).then(|| LaneIntent::Goto(n - 1))
}

/// 按键 → 意图（**纯函数**，可单测；修饰符精确匹配）。
fn intent_for(key: &str, mods: KeyMods) -> Option<LaneIntent> {
    if mods.composing {
        return None;
    }
    if mods.alt_only() {
        return alt_intent(key);
    }
    if mods.ctrl_only() {
        return key.eq_ignore_ascii_case("n").then_some(LaneIntent::Next);
    }
    if mods.bare() && key == "F4" {
        return Some(LaneIntent::FocusPendingApproval);
    }
    None
}

/// 装 window 级 `keydown` 监听（`App` 挂载时调一次 —— 不是每条 lane 一次）。
pub(crate) fn wire_lane_hotkeys(lanes: LaneSet) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let cb =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            let mods = KeyMods {
                alt: ev.alt_key(),
                ctrl: ev.ctrl_key(),
                meta: ev.meta_key(),
                shift: ev.shift_key(),
                composing: ev.is_composing(),
            };
            let Some(intent) = intent_for(&ev.key(), mods) else {
                return;
            };
            let switched = match intent {
                LaneIntent::Next => lanes.cycle(1),
                LaneIntent::Prev => lanes.cycle(-1),
                LaneIntent::Goto(index) => lanes.set_active(index),
                LaneIntent::FocusPendingApproval => lanes.focus_next_pending_approval(),
            };
            // 只有真的换了前台才拦默认行为：越界 / 单 lane 时没有动作，别顺手吞掉按键。
            if switched {
                ev.prevent_default();
            }
        });
    let _ = win.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
    cb.forget();
}

#[cfg(test)]
mod tests {
    use super::{KeyMods, LaneIntent, intent_for};

    fn alt() -> KeyMods {
        KeyMods {
            alt: true,
            ..KeyMods::default()
        }
    }

    #[test]
    fn alt_arrows_and_digits_map_to_lane_intents() {
        assert_eq!(intent_for("ArrowRight", alt()), Some(LaneIntent::Next));
        assert_eq!(intent_for("ArrowLeft", alt()), Some(LaneIntent::Prev));
        assert_eq!(intent_for("1", alt()), Some(LaneIntent::Goto(0)));
        assert_eq!(intent_for("8", alt()), Some(LaneIntent::Goto(7)));
        // 契约只到 8：`Alt+9` 不是 lane 键。
        assert_eq!(intent_for("9", alt()), None);
        assert_eq!(intent_for("0", alt()), None);
    }

    #[test]
    fn modifier_matching_is_exact_not_contains() {
        let alt_shift = KeyMods {
            alt: true,
            shift: true,
            ..KeyMods::default()
        };
        assert_eq!(intent_for("ArrowRight", alt_shift), None);
    }

    #[test]
    fn ctrl_n_cycles_and_f4_is_bare_only() {
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::default()
        };
        assert_eq!(intent_for("n", ctrl), Some(LaneIntent::Next));
        assert_eq!(intent_for("N", ctrl), Some(LaneIntent::Next));
        assert_eq!(
            intent_for("F4", KeyMods::default()),
            Some(LaneIntent::FocusPendingApproval)
        );
        assert_eq!(intent_for("F4", ctrl), None);
        assert_eq!(intent_for("a", KeyMods::default()), None);
    }

    #[test]
    fn composition_and_unrelated_keys_are_ignored() {
        let composing = KeyMods {
            alt: true,
            composing: true,
            ..KeyMods::default()
        };
        assert_eq!(intent_for("ArrowRight", composing), None);
        assert_eq!(intent_for("Tab", KeyMods::default()), None);
    }
}
