//! 设置面板「本地行为」栏里的 `CM_TUI_*` 环境开关：**当前生效值**。
//!
//! 三条约束决定了这个模块的形状：
//!
//! 1. **不重新解析环境变量**：每个开关都调用运行时那**一个**判定函数
//!    （`event_loop::mouse_capture_enabled` / `paste_capture_enabled` / `loop_mode`、
//!    `clipboard::resolved_route`、`TermImageCaps::probe`、`status::reduced_motion`、
//!    `workspace_path::expand_tilde_enabled`）。两份解析必然漂移，而漂移的症状正是
//!    「面板说开、实际是关」—— 这一栏存在的唯一理由（#433）。
//! 2. **进程内只探一次**（[`cached`]）：环境变量在启动时定型，值不会变；而
//!    `CM_TUI_CLIPBOARD` 的判定要扫 PATH（`wl-copy` / `xclip` / `xsel`），每帧一次是浪费。
//! 3. **只展示，不能改**：这些开关在 `exec` 时定型，改需重启（分区说明行写着这句）。

use std::sync::OnceLock;

use super::clipboard::{self, CopyRoute};
use super::event_loop::{self, LoopMode};
use super::settings_panel::FieldId;
use super::status;
use super::term_image::{ImagePick, TermImageCaps};
use crate::workspace_path;

/// 本地行为栏里展示的一个 `CM_TUI_*` 开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvSwitch {
    /// `CM_TUI_MOUSE`：鼠标捕获（关掉才能拿回终端原生拖选复制）。
    Mouse,
    /// `CM_TUI_PASTE`：括号粘贴（关掉时粘贴是一串按键）。
    Paste,
    /// `CM_TUI_CLIPBOARD`：`/copy` 的投递路径（或关掉复制）。
    Clipboard,
    /// `CM_TUI_LOOP`：事件循环模式。
    Loop,
    /// `CM_TUI_COLOR`：色深。
    Color,
    /// `CM_TUI_IMAGE`：背景图档位。
    Image,
    /// `CM_TUI_REDUCED_MOTION`：动效。
    Motion,
    /// `CM_TUI_EXPAND_TILDE`：客户端是否把 `~` 展开成本机家目录。
    Tilde,
}

impl EnvSwitch {
    /// 面板显示顺序（行为相关在前，终端能力在后）。
    pub(crate) const ALL: [EnvSwitch; 8] = [
        EnvSwitch::Mouse,
        EnvSwitch::Paste,
        EnvSwitch::Clipboard,
        EnvSwitch::Loop,
        EnvSwitch::Color,
        EnvSwitch::Image,
        EnvSwitch::Motion,
        EnvSwitch::Tilde,
    ];

    /// 环境变量名：行值里带着它，用户才知道该改哪一个（改完要重启）。
    pub(crate) const fn env_name(self) -> &'static str {
        match self {
            Self::Mouse => "CM_TUI_MOUSE",
            Self::Paste => "CM_TUI_PASTE",
            Self::Clipboard => "CM_TUI_CLIPBOARD",
            Self::Loop => "CM_TUI_LOOP",
            Self::Color => "CM_TUI_COLOR",
            Self::Image => "CM_TUI_IMAGE",
            Self::Motion => "CM_TUI_REDUCED_MOTION",
            Self::Tilde => "CM_TUI_EXPAND_TILDE",
        }
    }

    /// 当前生效值（进程内只探一次）。
    pub(crate) fn value(self) -> &'static str {
        cached()[self as usize].as_str()
    }

    /// 探一次：调运行时的判定函数，再把结果翻成人读措辞。
    fn probe(self) -> String {
        match self {
            Self::Mouse => flag_text(
                event_loop::mouse_capture_enabled(),
                "捕获鼠标",
                "终端原生拖选",
            ),
            Self::Paste => flag_text(
                event_loop::paste_capture_enabled(),
                "打包粘贴",
                "逐字符按键",
            ),
            Self::Clipboard => route_text(clipboard::resolved_route()),
            Self::Loop => loop_text(event_loop::loop_mode()),
            Self::Color => TermImageCaps::probe().depth().name().to_string(),
            Self::Image => image_text(TermImageCaps::probe().pick()),
            Self::Motion => flag_text(!status::reduced_motion(), "带动画", "静态"),
            Self::Tilde => flag_text(
                workspace_path::expand_tilde_enabled(),
                "客户端展开",
                "原样交给 serve",
            ),
        }
    }
    /// 面板行 ↦ 开关（`None` = 不是 `CM_TUI_*` 行）。
    ///
    /// 只写这一向：`Section::Local` 的字段表是另一向的事实来源，两者的一致性由单测
    /// `local_section_lists_every_switch_once` 钉住（两向都手写就多一处可漂移的地方）。
    pub(crate) const fn of(field: FieldId) -> Option<EnvSwitch> {
        match field {
            FieldId::EnvMouse => Some(EnvSwitch::Mouse),
            FieldId::EnvPaste => Some(EnvSwitch::Paste),
            FieldId::EnvClipboard => Some(EnvSwitch::Clipboard),
            FieldId::EnvLoop => Some(EnvSwitch::Loop),
            FieldId::EnvColor => Some(EnvSwitch::Color),
            FieldId::EnvImage => Some(EnvSwitch::Image),
            FieldId::EnvMotion => Some(EnvSwitch::Motion),
            FieldId::EnvTilde => Some(EnvSwitch::Tilde),
            _ => None,
        }
    }
}

/// 全部开关的生效值；首次调用时探一次，之后复用（见模块文档第 2 条）。
fn cached() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(|| EnvSwitch::ALL.iter().map(|s| s.probe()).collect())
}

/// 开/关类开关：`开（捕获鼠标）` / `关（终端原生拖选）`。
fn flag_text(on: bool, when_on: &str, when_off: &str) -> String {
    let (state, detail) = if on {
        ("开", when_on)
    } else {
        ("关", when_off)
    };
    format!("{state}（{detail}）")
}

/// 事件循环模式：`事件驱动`（默认） / `固定节流`（`CM_TUI_LOOP=interval` 的回退口）。
fn loop_text(mode: LoopMode) -> String {
    match mode {
        LoopMode::Event => "事件驱动".to_string(),
        LoopMode::Interval => "固定节流".to_string(),
    }
}

/// `/copy` 会走哪条路；不可用时把原因一起说出来（谁来承接内容、为什么不行）。
fn route_text(route: Result<CopyRoute, String>) -> String {
    match route {
        Ok(route) => route.label().to_string(),
        Err(reason) => format!("不可用：{reason}"),
    }
}

/// 背景图档位：`none` 时补一句它为什么是 none（无色终端 / 复用器不确认透传都可能）。
fn image_text(pick: ImagePick) -> String {
    match pick {
        ImagePick::Off => "none（不画图）".to_string(),
        other => other.name().to_string(),
    }
}

/// 面板「本地行为」栏的一行值：`<生效值> · <环境变量名>`。
///
/// 环境变量名必须与值同行：只说「关」而不说关的是哪一个、怎么改回来，对排障没用；
/// 「改需重启」由分区说明行统一交代一次，不必每行重复。
pub(crate) fn row_value(switch: EnvSwitch) -> String {
    format!("{} · {}", switch.value(), switch.env_name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_mode::settings_panel::Section;

    /// 「本地行为」分区的字段表与开关表必须一一对应（同序）。
    ///
    /// 两向映射只写了一半（[`EnvSwitch::of`]），另一半是 `Section::Local` 的字段表；
    /// 这条把两者钉在一起：漏加一个 `FieldId::EnvX` 就少一行，加错顺序就与 `ALL` 不等。
    #[test]
    fn local_section_lists_every_switch_once() {
        let listed: Vec<EnvSwitch> = Section::Local
            .fields()
            .iter()
            .filter_map(|f| EnvSwitch::of(*f))
            .collect();
        assert_eq!(listed, EnvSwitch::ALL.to_vec());
    }

    /// 分组条目与面板字段同源：`EnvSwitch` 的每一项都得有环境变量名与生效值。
    #[test]
    fn every_switch_has_a_name_and_a_value() {
        for switch in EnvSwitch::ALL {
            assert!(switch.env_name().starts_with("CM_TUI_"), "{switch:?}");
            assert!(!switch.value().is_empty(), "{switch:?}");
            // 行值与取值的单一来源：行值 = 值 + 变量名。
            let row = row_value(switch);
            assert!(row.starts_with(switch.value()), "{row}");
            assert!(row.contains(switch.env_name()), "{row}");
        }
    }

    /// 判别式必须与 `ALL` 同序：[`EnvSwitch::value`] 用 `self as usize` 索引缓存。
    ///
    /// 这条与 `Section::ALL` 的 `debug_assert` 是同一类隐式契约：调换枚举顺序而不动 `ALL`
    /// 不会编译失败，只会让面板显示**别人**的生效值 —— 那是这一栏最坏的故障。
    #[test]
    fn all_order_matches_the_discriminants() {
        for (i, switch) in EnvSwitch::ALL.iter().enumerate() {
            assert_eq!(*switch as usize, i, "{switch:?}");
        }
    }

    /// 探头只跑一次：两次取到同一个地址（同一份缓存），而不是每帧重算。
    #[test]
    fn values_are_probed_once() {
        let first = EnvSwitch::Mouse.value();
        let second = EnvSwitch::Mouse.value();
        assert_eq!(first.as_ptr(), second.as_ptr());
    }

    /// 措辞映射（不依赖真实环境变量）：关态必须写成「关」，不能只写「（终端原生拖选）」。
    #[test]
    fn flag_text_says_on_or_off_first() {
        assert_eq!(
            flag_text(true, "捕获鼠标", "终端原生拖选"),
            "开（捕获鼠标）"
        );
        assert_eq!(
            flag_text(false, "捕获鼠标", "终端原生拖选"),
            "关（终端原生拖选）"
        );
    }

    #[test]
    fn loop_text_names_both_modes() {
        assert_eq!(loop_text(LoopMode::Event), "事件驱动");
        assert_eq!(loop_text(LoopMode::Interval), "固定节流");
    }

    #[test]
    fn route_text_reports_the_route_or_the_reason() {
        assert_eq!(route_text(Ok(CopyRoute::Osc52)), "终端剪贴板");
        assert_eq!(route_text(Ok(CopyRoute::Tool("wl-copy"))), "wl-copy");
        // 不可用时不能只写「关」：原因（关了 / screen 不转发 / 没装工具）处置完全不同。
        assert!(route_text(Err("CM_TUI_CLIPBOARD 已关闭复制".to_string())).contains("已关闭"));
    }

    #[test]
    fn image_text_marks_off_explicitly() {
        assert_eq!(image_text(ImagePick::Off), "none（不画图）");
        assert_eq!(image_text(ImagePick::Kitty), "kitty");
    }
}
