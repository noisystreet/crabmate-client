//! `visualViewport` 键盘避让：把键盘占用高度写入 `--vv-keyboard-inset`，供窄屏 composer 抬高。

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

fn apply_keyboard_inset_css() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(doc) = window.document() else {
        return;
    };
    let Some(root) = doc.document_element() else {
        return;
    };
    let Ok(root) = root.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };

    let inset_px = measured_keyboard_inset_px(&window);

    let _ = root
        .style()
        .set_property("--vv-keyboard-inset", &format!("{inset_px:.0}px"));
    // `data-kb-inset` 是 CSS 侧的「键盘确实在」开关：宽屏纯浏览器平板既拿不到壳侧
    // `--cm-ime-inset`，也不命中窄屏媒体查询（mobile.css），只能靠这个属性让 `.app-root` 抬高。
    apply_kb_inset_attr(&root, inset_px > 0.0);
}

/// `vv.scale()` 与 1 的容许偏差：pinch-zoom 会把它推离 1，浮点比较留一点余量。
const KB_INSET_SCALE_EPSILON: f64 = 0.01;

/// 可当键盘高度用的遮挡量（CSS px）；差值不可信时返回 0。
///
/// 两道闸门挡掉「不是键盘」的视口变化 —— 桌面浏览器缩放 / 滚动同样会让
/// `visualViewport` 变矮，照单全收就会把 `.app-root` 凭空顶起一截：
/// - **无触控输入**（`maxTouchPoints == 0`）：桌面鼠标环境本来就没有软键盘；
/// - **`vv.scale() != 1`**：pinch-zoom 改的是 `scale`，软键盘不改。
fn measured_keyboard_inset_px(window: &web_sys::Window) -> f64 {
    if !has_touch_input(window) {
        return 0.0;
    }
    let Some(vv) = window.visual_viewport() else {
        return 0.0;
    };
    if (vv.scale() - 1.0).abs() > KB_INSET_SCALE_EPSILON {
        return 0.0;
    }
    let Ok(layout_h) = window.inner_height() else {
        return 0.0;
    };
    let Some(layout_h) = layout_h.as_f64() else {
        return 0.0;
    };
    // 键盘弹出时 visualViewport 变矮；offsetTop 可能上移。
    // 若 Activity 已 adjustResize，innerHeight 与 vv 同步缩小 → inset≈0（由窗口变矮抬起 composer）。
    (layout_h - vv.height() - vv.offset_top()).max(0.0)
}

/// 只在状态真的翻转时写属性：`visualViewport` 的 scroll 与 window 的 resize 共用
/// 一个回调，键盘在时反复写同值只会白白触发样式重算。
fn apply_kb_inset_attr(root: &web_sys::HtmlElement, present: bool) {
    if present == root.has_attribute("data-kb-inset") {
        return;
    }
    if present {
        let _ = root.set_attribute("data-kb-inset", "");
    } else {
        let _ = root.remove_attribute("data-kb-inset");
    }
}

/// 设备是否可能有软键盘（`maxTouchPoints > 0`）。
fn has_touch_input(window: &web_sys::Window) -> bool {
    window.navigator().max_touch_points() > 0
}

/// 立即重算 `--vv-keyboard-inset`（供 focus 等早于 `visualViewport` 事件的路径调用）。
pub(crate) fn refresh_keyboard_inset() {
    apply_keyboard_inset_css();
}

fn root_has_attr(name: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(doc) = window.document() else {
        return false;
    };
    let Some(root) = doc.document_element() else {
        return false;
    };
    root.has_attribute(name)
}

fn should_apply_keyboard_avoidance() -> bool {
    // 窄屏媒体查询、Android 远程壳（横屏可能 >768px），或设备本来就会有软键盘
    // （宽屏纯浏览器平板）。这里**不看** `data-kb-inset`：那个属性正是本函数放行之后、
    // 重算 inset 才挂上的，用它当门会让首次 focus 自锁在门外（宽屏平板上 focus 时的
    // 「滚进视口 + 重试」那一半就永远不跑）。
    let Some(window) = web_sys::window() else {
        return false;
    };
    root_has_attr("data-narrow-viewport")
        || root_has_attr("data-cm-mobile-shell")
        || has_touch_input(&window)
}

fn composer_bar_element(ta: &web_sys::HtmlTextAreaElement) -> web_sys::HtmlElement {
    let mut node: Option<web_sys::Node> = Some(ta.clone().into());
    while let Some(n) = node {
        if let Ok(el) = n.clone().dyn_into::<web_sys::HtmlElement>()
            && el.class_list().contains("composer-ds")
        {
            return el;
        }
        node = n.parent_node();
    }
    ta.clone().into()
}

/// 窄屏 / 移动壳聚焦输入元素时：立刻把可见容器滚入视口并重算键盘 inset（软键盘动画期间多次重试）。
///
/// `el` 传「需完整可见的容器」（如输入条 / 就地编辑表单），而非单个 `textarea`，
/// 使同容器内的按钮也留在键盘之上。
pub(crate) fn on_mobile_focus_keep_visible(el: &web_sys::HtmlElement) {
    if !should_apply_keyboard_avoidance() {
        return;
    }
    // false：尽量贴齐视口底边，避免软键盘动画期间把容器滚出可见区上方。
    el.scroll_into_view_with_bool(false);
    refresh_keyboard_inset();

    let el = el.clone();
    spawn_local(async move {
        for delay_ms in [50_u32, 150, 300, 500] {
            gloo_timers::future::TimeoutFuture::new(delay_ms).await;
            el.scroll_into_view_with_bool(false);
            refresh_keyboard_inset();
        }
    });
}

/// 聊天输入条聚焦：以 `.composer-ds` 输入条为可见容器。
pub(crate) fn on_composer_focus_keep_visible(ta: &web_sys::HtmlTextAreaElement) {
    on_mobile_focus_keep_visible(&composer_bar_element(ta));
}

/// 订阅 `visualViewport` 的 resize/scroll，维护 `--vv-keyboard-inset`。
pub fn wire_visual_viewport_keyboard_inset() {
    Effect::new(move |_| {
        let Some(window) = web_sys::window() else {
            return;
        };
        apply_keyboard_inset_css();

        let Some(vv) = window.visual_viewport() else {
            return;
        };

        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            apply_keyboard_inset_css();
        });
        let _ = vv.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
        let _ = vv.add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref());
        // 部分 WebView 只在 window resize 上反映键盘
        let _ = window.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}
