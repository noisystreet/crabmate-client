//! 内联 SVG 图标的统一属性模板。
//!
//! 本模块是 `viewBox` / `fill` / `stroke` / `stroke-width` / 线帽端点 / `aria-hidden`
//! 的**唯一来源**：调用方只提供类名、填充风格与图形子节点（`path` / `polyline` /
//! `rect` / `circle` / `line`），不再各自手抄同一份属性模板。

use leptos::prelude::*;

/// 图标的填充 / 描边风格（决定模板给出的 `fill` 与 `stroke`）。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum IconStyle {
    /// 线性图标：`fill="none"` + `stroke="currentColor"`（2px 圆角端点）。
    #[default]
    Stroke,
    /// 实心图标：`fill="currentColor"`，不描边。
    Fill,
}

/// 统一属性模板的内联 SVG 图标。
///
/// `class` 为可变类名（默认空）：图标尺寸与配色由对应 CSS 规则负责，
/// 虚线/实心差异由 `style` 决定，图形本体由 `children` 提供。
#[component]
pub fn Icon(
    #[prop(optional)] class: &'static str,
    #[prop(default = IconStyle::Stroke)] style: IconStyle,
    children: Children,
) -> impl IntoView {
    let (fill, stroke) = match style {
        IconStyle::Stroke => ("none", "currentColor"),
        IconStyle::Fill => ("currentColor", "none"),
    };
    view! {
        <svg
            class=class
            viewBox="0 0 24 24"
            fill=fill
            stroke=stroke
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
        >
            {children()}
        </svg>
    }
}

/// 左向 chevron（`‹`）：设置页返回按钮的重复实现，合并为同一图标。
pub fn icon_chevron_left(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <polyline points="15 18 9 12 15 6" />
        </Icon>
    }
}

/// 右向 chevron（`›`）：子菜单 / 折叠展开指示。
pub fn icon_chevron_right(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <polyline points="9 18 15 12 9 6" />
        </Icon>
    }
}

/// 下向 chevron（`▾`）：下拉 / 展开指示共用同一多边形。
pub fn icon_chevron_down(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <polyline points="6 9 12 15 18 9" />
        </Icon>
    }
}

/// 关闭 / 移除（`×`）：对话框、标签页、待发附图、窗口关闭共用。
pub fn icon_x(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <path d="M18 6 6 18M6 6l12 12" />
        </Icon>
    }
}

/// 勾选（`✓`）：菜单项选中态。
pub fn icon_check(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <polyline points="20 6 9 17 4 12" />
        </Icon>
    }
}

/// 减号（`−`）：窗口最小化 / 数值步进减。
pub fn icon_minus(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <path d="M5 12h14" />
        </Icon>
    }
}

/// 加号（`+`）：新建对话 / 数值步进增。
pub fn icon_plus(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <path d="M12 5v14M5 12h14" />
        </Icon>
    }
}

/// 最大化（`□`）：窗口最大化。
pub fn icon_maximize(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <rect x="5" y="5" width="14" height="14" rx="1" />
        </Icon>
    }
}

/// 上箭头（`↑`）：查找栏上一个匹配。
pub fn icon_arrow_up(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <path d="M12 19V5M5 12l7-7 7 7" />
        </Icon>
    }
}

/// 下箭头（`↓`）：查找栏下一个匹配。
pub fn icon_arrow_down(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <path d="M12 5v14M19 12l-7 7-7-7" />
        </Icon>
    }
}

/// 搜索（`⌕`）：侧栏搜索面板开关。
pub fn icon_search(class: &'static str) -> impl IntoView {
    view! {
        <Icon class=class>
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.8-3.8" />
        </Icon>
    }
}
