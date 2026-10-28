//! `/background` 的完整生命周期：加载 / 每帧维护 / 关闭 / 退出清理。
//!
//! 从 `mod.rs` 拆出：那里顶着 920 行门禁，而这四段都围绕同一个 [`Background`]
//! （像素档每帧重放、格子档按当前尺寸重采样），放一处更好读。
//!
//! [`Background`]: super::background::Background

use std::path::Path;

use ratatui::layout::Rect;

use super::LaneApp;
use super::background;
use super::controls::clear_word;
use super::state::LineKind;
use super::term_image;

impl LaneApp<'_> {
    /// 每帧维护背景：像素档重放一次图像，格子档按当前尺寸重采样。
    pub(super) fn paint_background(&mut self, screen: Rect) {
        let (w, h) = (screen.width, screen.height);
        let Some(bg) = self.bg.as_mut() else {
            return;
        };
        if !bg.is_pixels() {
            if let Some(cells) = bg.refresh_cells(w, h) {
                self.st.bg_cells = Some(cells);
            }
            return;
        }
        if bg.place(w, h).is_ok() {
            return;
        }
        // 写终端失败：关掉，免得每帧重试同一个错误。
        self.bg = None;
        self.st.push_line(
            LineKind::System,
            "背景图写入失败，已关闭（/background 可重设）",
        );
    }

    /// 退出前显式删背景图（备用屏幕本会清空，但别把终端图像配额留到下次运行）。
    pub(super) fn clear_background(&mut self) {
        if let Some(mut bg) = self.bg.take() {
            let _ = bg.clear();
        }
        self.st.bg_cells = None;
    }

    /// `/background [路径]`：空参查状态，清除词关闭，其余按路径加载。
    pub(super) fn set_background(&mut self, arg: Option<String>) {
        // 清除词大小写不敏感（`/workspace` 的 `is_default_word` 同样宽松）：否则
        // `/background OFF` 会被当成文件路径 "OFF" 去读盘，报一个莫名其妙的「读不到」。
        let word = arg.as_deref().map(|v| v.trim().to_ascii_lowercase());
        if word
            .as_deref()
            .is_some_and(|w| clear_word(w) || w == "default")
        {
            if let Some(mut bg) = self.bg.take() {
                let _ = bg.clear();
            }
            self.st.bg_cells = None;
            self.st.push_line(LineKind::System, "背景图已关闭");
            return;
        }
        match arg {
            Some(path) => self.load_background(&path),
            None => {
                let echo = match self.bg.as_ref() {
                    Some(bg) => format!("背景图：{}", bg.path().display()),
                    None => "背景图：(未设置；/background <图片路径> 设置，off 关闭)".to_string(),
                };
                self.st.push_line(LineKind::System, &echo);
            }
        }
    }

    /// 加载背景图。像素协议可用就画真像素；否则退到格子底色（不依赖任何终端协议）。
    fn load_background(&mut self, path: &str) {
        let caps = term_image::TermImageCaps::probe();
        // `Off` 是「不要画图」的明确结论（`CM_TUI_IMAGE=none`，或无色 / 16 色终端）：
        // 格子档虽然不依赖任何终端协议，但不该绕过用户的显式关闭。
        if caps.pick() == term_image::ImagePick::Off {
            self.st.push_line(
                LineKind::System,
                &format!(
                    "背景图未启用（picked=none）；检查 CM_TUI_IMAGE / CM_TUI_COLOR / NO_COLOR（16 色终端也不行）：{}",
                    caps.summary()
                ),
            );
            return;
        }
        let pixels = caps.pick() == term_image::ImagePick::Kitty;
        // 格子底色需要 256 色：16 色下色阶太少，铺出来是噪声而不是图。
        if !pixels && !caps.depth().can_half_block() {
            self.st.push_line(
                LineKind::System,
                &format!(
                    "背景图至少要 256 色才铺格子底色（当前：{}）",
                    caps.summary()
                ),
            );
            return;
        }
        let mode = if pixels {
            background::Mode::Pixels
        } else {
            // 没有像素协议也能有背景：把图缩成「列 × 行」，逐格当底色用。
            background::Mode::Cells
        };
        let (cols, rows) = self.last_size;
        match background::Background::load(Path::new(path), cols, rows, mode) {
            Ok(mut bg) => {
                // 格子档在这里先采一次样：`paint_bg_cells` 在 `draw` 里读 `bg_cells`，
                // 而重采样发生在 `draw` 之后，不先采就会有一帧完全没有背景。
                let cells = if bg.is_pixels() {
                    None
                } else {
                    bg.refresh_cells(cols.max(1), rows.max(1))
                };
                let how = if bg.is_pixels() {
                    "真像素（kitty 图形协议）"
                } else {
                    "格子马赛克（无像素协议，退到每格底色）"
                };
                if let Some(mut old) = self.bg.replace(bg) {
                    let _ = old.clear();
                }
                self.st.bg_cells = cells;
                self.st.push_line(
                    LineKind::System,
                    &format!("背景图：{path} — {how}，已压暗（/background off 关闭）"),
                );
            }
            Err(e) => self
                .st
                .push_line(LineKind::System, &format!("背景图加载失败：{e}")),
        }
    }
}
