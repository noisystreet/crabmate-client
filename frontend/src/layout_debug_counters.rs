//! 流式布局过渡债观测计数（**仅 `debug_assertions`**；release 为空操作）。
//!
//! 对应 `docs/Turn布局设计.md` §15 过渡债观测 / 所有权收口：
//! - `empty_shell_skip`：本应挂载但因空助手壳被跳过的次数（读路径过滤生效）
//! - `commentary_handoff`：**兼容路径**清空非空 `loading.text`（与定稿同文）的次数；
//!   主路径旁白不写 loading，overlay 收口**不**计入此项。
//! - `hydration_layout_projection`：本页水合按会话级 `layout` 映射成 v2 行键（成功 / 回落 legacy）
//!   的计数——没有它，「为什么这个会话还在整段重建」只能靠肉眼看（#545）。

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(debug_assertions)]
static EMPTY_SHELL_SKIP: AtomicU64 = AtomicU64::new(0);
#[cfg(debug_assertions)]
static COMMENTARY_HANDOFF: AtomicU64 = AtomicU64::new(0);
#[cfg(debug_assertions)]
static HYDRATION_LAYOUT_MAPPED: AtomicU64 = AtomicU64::new(0);
#[cfg(debug_assertions)]
static HYDRATION_LAYOUT_FALLBACK: AtomicU64 = AtomicU64::new(0);

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
fn log_debug(msg: String) {
    web_sys::console::log_1(&msg.into());
}

#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
fn log_debug(_msg: String) {}

/// 空助手壳被读路径跳过时调用（TUI 过滤）。
#[inline]
pub(crate) fn note_empty_shell_skip() {
    #[cfg(debug_assertions)]
    {
        let n = EMPTY_SHELL_SKIP.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(25) {
            log_debug(format!("[layout_debug] empty_shell_skip={n}"));
        }
    }
}

/// I14 **兼容**：清空非空 `loading.text`（与定稿同文）时调用；overlay-only 收口不计。
#[inline]
pub(crate) fn note_commentary_handoff() {
    #[cfg(debug_assertions)]
    {
        let n = COMMENTARY_HANDOFF.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(10) {
            log_debug(format!("[layout_debug] commentary_handoff={n}"));
        }
    }
}

/// 水合按会话级 `layout` 映射历史行：`mapped = true` 记为成功，`false` 记回落 legacy（#545）。
#[inline]
pub(crate) fn note_hydration_layout_projection(mapped: bool) {
    #[cfg(debug_assertions)]
    {
        let (counter, label) = if mapped {
            (&HYDRATION_LAYOUT_MAPPED, "hydration_layout_mapped")
        } else {
            (&HYDRATION_LAYOUT_FALLBACK, "hydration_layout_fallback")
        };
        let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(25) {
            log_debug(format!("[layout_debug] {label}={n}"));
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = mapped;
}

#[cfg(all(test, debug_assertions))]
#[must_use]
fn snapshot() -> (u64, u64, u64, u64) {
    (
        EMPTY_SHELL_SKIP.load(Ordering::Relaxed),
        COMMENTARY_HANDOFF.load(Ordering::Relaxed),
        HYDRATION_LAYOUT_MAPPED.load(Ordering::Relaxed),
        HYDRATION_LAYOUT_FALLBACK.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment_under_debug() {
        #[cfg(debug_assertions)]
        {
            let (a0, b0, c0, d0) = snapshot();
            note_empty_shell_skip();
            note_commentary_handoff();
            note_hydration_layout_projection(true);
            note_hydration_layout_projection(false);
            let (a1, b1, c1, d1) = snapshot();
            assert!(a1 > a0);
            assert!(b1 > b0);
            assert!(c1 > c0);
            assert!(d1 > d0);
        }
        #[cfg(not(debug_assertions))]
        {
            note_empty_shell_skip();
            note_commentary_handoff();
            note_hydration_layout_projection(true);
        }
    }
}
