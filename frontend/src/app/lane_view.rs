//! 单窗口多 lane 的**视图切换**（#561 **S3**）：一条 lane 一个 `.lane-slot`，切换只改 class。
//!
//! # 为什么只改 class，不重建
//!
//! 切 lane 必须**只换显示**：重建子树会丢掉这条 lane 的 DOM —— 流式输出的片段、草稿、
//! 滚动位置都在里面，而且等于把 S1「视图不能靠响应式动态增删」的结论再踩一遍。所以每条
//! lane 挂到自己的 slot，切换只把 `lane-slot--active` 从旧的那条挪到新的那条。
//!
//! # 为什么 slot 用 `display: contents`
//!
//! 包一层 `div` 会改变布局（`app_body` 的根已经是 `app-root`）。`contents` 让这层包装对
//! 布局透明 —— 视觉与改造前逐字一致，而 `display: none` 照样能把它整棵隐藏。
//!
//! slot 由**命令式**创建：新 lane 的视图本来就走 `mount_to`（不在响应式树里），所以这里
//! 持有的是 `web_sys::HtmlElement` 句柄，不是 `NodeRef`。

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;

/// 本窗口各条 lane 的 slot 元素（下标 = lane 下标）。
#[derive(Clone, Default)]
pub(crate) struct LaneSlots {
    slots: Rc<RefCell<Vec<Option<web_sys::HtmlElement>>>>,
}

impl LaneSlots {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 登记（或更新）第 `index` 条 lane 的 slot。
    pub(crate) fn set_slot(&self, index: usize, el: web_sys::HtmlElement) {
        let mut slots = self.slots.borrow_mut();
        if slots.len() <= index {
            slots.resize_with(index + 1, || None);
        }
        slots[index] = Some(el);
    }

    /// 只改 class：`lane-slot--active` 挂到当前前台那一条上，其余保持隐藏。
    ///
    /// 这是 S3 的全部「切换」动作 —— 没有任何重建、没有重新 request。
    pub(crate) fn apply_active(&self, active: usize) {
        let slots = self.slots.borrow();
        for (index, el) in slots.iter().enumerate() {
            let Some(el) = el else { continue };
            let class = if index == active {
                "lane-slot lane-slot--active"
            } else {
                "lane-slot"
            };
            el.set_class_name(class);
        }
    }

    /// 为一条新 lane 建 slot、挂到容器末尾，返回句柄（`mount_to` 的挂载点）。
    pub(crate) fn create_slot(
        &self,
        layer: &web_sys::HtmlElement,
        index: usize,
    ) -> Option<web_sys::HtmlElement> {
        let doc = web_sys::window()?.document()?;
        let slot = doc.create_element("div").ok()?;
        slot.set_class_name("lane-slot");
        // 便于手测 / e2e 定位「第几条 lane」。
        let _ = slot.set_attribute("data-lane", &index.to_string());
        layer.append_child(&slot).ok()?;
        let el: web_sys::HtmlElement = slot.unchecked_into();
        self.set_slot(index, el.clone());
        Some(el)
    }
}
