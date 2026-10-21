//! 全屏像素背景：直接发 kitty graphics protocol 转义序列，`z` 取负值把图放到**文字下方**。
//!
//! 为什么不用 `ratatui-image`：它把 kitty 图渲染成 unicode placeholder（每格占位），
//! 不暴露 `z` 参数——只能「占住单元格」，不能「在文字之下」。背景层只有负 `z` 这一条路。
//!
//! 三条来自协议规范的硬约束（都直接决定本模块的形状）：
//!
//! 1. `z`（z-index）取负值才在文字下方；`C=1` 表示放置时不移动光标，否则每次放置都会
//!    把光标挪走，打乱 ratatui 对坐标的假设。
//! 2. **「When scrolling the screen… images must be scrolled along with text」**：图像会
//!    跟着文本一起滚动，所以每帧 `draw()` 之后都必须重新 place 一次，不能只在加载时放一次。
//! 3. 图像在备用屏幕切换 / `ESC[2J` 时被终端清空，于是退出路径天然有兜底；这里仍显式
//!    删除一次，避免用户长时间停留在本程序里时占着终端的图像配额。
//!
//! 两个档位（见 [`Mode`]）共用「解码 → 缩放 → 压暗」这一条流水线，只差最后一步落点：
//!
//! - [`Mode::Pixels`]：能力探测判定为 kitty 协议时用（见 `term_image`），真像素。
//! - [`Mode::Cells`]：其余终端（含整个 VTE 家族）退到**每格底色**，是马赛克，
//!   但不依赖任何终端协议，`Cell` 的前景文字与背景色本来就互不干扰。
//!
//! sixel / iTerm2 没有「文字下方」这层语义，做不了像素背景，但照样能走 Cells 档。

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageEncoder as _, ImageReader};
use ratatui::style::Color;

/// 压暗系数（0..1）。正文压在图上方，不压暗的话亮图会把 `#eef1f6` 的正文彻底糊掉。
/// TUI 层做不了半透明遮罩，只能在像素上做。
const DIM: f32 = 0.35;

/// 传输前把最长边缩到这个像素数以内。只影响内存与带宽——最终显示尺寸由 `c` / `r`
/// （跨多少列 / 行）决定，终端负责缩放，所以这里不需要知道单元格的像素大小。
const MAX_EDGE: u32 = 1920;

/// 单块 base64 上限：协议要求 ≤ 4096，且非末块长度必须是 4 的倍数。
const CHUNK: usize = 4096;

/// 格子档的压暗系数，比像素档亮：下采样到格子数后颜色被平均掉一部分反差，
/// 再按像素档的 0.35 压就几乎看不出图了。
const CELL_DIM: f32 = 0.4;

/// 格子档保留位图的最长边。最终只采样到「列 × 行」（几十×几十），
/// 留到 480 已经远超任何终端格子数，没必要占更多内存。
const MAX_CELL_EDGE: u32 = 480;

/// 读盘上限：背景是装饰，没有理由为一张超大图冒内存峰值（像素解码后还要再乘 4）。
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// 像素数上限（约 64 MP ≈ 256 MiB RGBA）。缩放在解码**之后**，所以必须先在头部拦一道。
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// 渲染方式：同一条「解码 → 缩放 → 压暗」流水线的两种落点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// kitty graphics：真像素，`z=-1` 压在文字下方。
    Pixels,
    /// 纯 ratatui：每一格的**底色**取图对应处的颜色（马赛克）。不依赖任何终端协议。
    Cells,
}

/// 格子背景：一张与终端格子一一对应的采样色表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BgCells {
    cols: u16,
    rows: u16,
    colors: Vec<Color>,
}

impl BgCells {
    /// 取某格底色。终端尺寸与采样表不一致时按比例映射——resize 后到下一帧重采样
    /// 之间会短暂处于这种状态，宁可先给个近似色，也不要闪一帧空白。
    #[must_use]
    pub(crate) fn get(&self, x: u16, y: u16, width: u16, height: u16) -> Option<Color> {
        if self.cols == 0 || self.rows == 0 || width == 0 || height == 0 {
            return None;
        }
        let cx = u32::from(x) * u32::from(self.cols) / u32::from(width);
        let cy = u32::from(y) * u32::from(self.rows) / u32::from(height);
        let cx = cx.min(u32::from(self.cols) - 1);
        let cy = cy.min(u32::from(self.rows) - 1);
        self.colors
            .get((cy * u32::from(self.cols) + cx) as usize)
            .copied()
    }

    /// 采样表尺寸（只有测试要看；生产侧走 `get` 的比例映射）。
    #[cfg(test)]
    pub(crate) fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// 造一张纯色采样表（渲染层测试用，避免在别的模块里复制字段布局）。
    #[cfg(test)]
    pub(crate) fn solid(cols: u16, rows: u16, color: Color) -> Self {
        Self {
            cols,
            rows,
            colors: vec![color; usize::from(cols) * usize::from(rows)],
        }
    }
}

/// 同进程内的 image id 递增，避免与终端里其他程序的图像撞号。
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// 一张已就绪的背景图。
pub(crate) struct Background {
    path: PathBuf,
    mode: Mode,
    /// Pixels 档：已压暗并重编码好的 PNG（终端尺寸变化要重传时不必再读盘）。
    png: Vec<u8>,
    id: u32,
    /// 当前 placement 覆盖的 (列, 行)；`None` = 还没传过图。
    placed: Option<(u16, u16)>,
    /// Cells 档：压暗后的位图，重采样时用；Pixels 档为 `None`。
    img: Option<image::RgbaImage>,
    /// Cells 档上一次采样的尺寸；`None` = 还没采过，需重采。
    cells_size: Option<(u16, u16)>,
}

impl Background {
    /// 读图 → 缩放 → 压暗 → 重编码 PNG → 传输并显示。
    ///
    /// 失败只返回可展示的错误串：背景是装饰，加载不了不该影响对话本身。
    pub(crate) fn load(path: &Path, cols: u16, rows: u16, mode: Mode) -> Result<Self, String> {
        let img = prepare(path, mode)?;
        let mut bg = Self {
            path: path.to_path_buf(),
            mode,
            png: Vec::new(),
            id: next_id(),
            placed: None,
            img: None,
            cells_size: None,
        };
        match mode {
            Mode::Pixels => {
                bg.png = encode_png(&img)?;
                bg.place(cols, rows)
                    .map_err(|e| format!("写入终端失败：{e}"))?;
            }
            // Cells 档不写终端：颜色由渲染层逐格铺到底色上，这里只留位图待采样。
            Mode::Cells => bg.img = Some(img),
        }
        Ok(bg)
    }

    /// 是否走 kitty 真像素（否则是格子底色马赛克）。
    #[must_use]
    pub(crate) fn is_pixels(&self) -> bool {
        matches!(self.mode, Mode::Pixels)
    }

    /// Cells 档按当前终端尺寸重采样；尺寸没变返回 `None`（表示这一帧不必动缓冲）。
    pub(crate) fn refresh_cells(&mut self, cols: u16, rows: u16) -> Option<BgCells> {
        let img = self.img.as_ref()?;
        let size = (cols.max(1), rows.max(1));
        if self.cells_size == Some(size) {
            return None;
        }
        self.cells_size = Some(size);
        Some(sample(img, size.0, size.1))
    }

    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 每帧重放（原因见模块头第 2 条）；尺寸变化时重传一张。
    pub(crate) fn place(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        if !self.is_pixels() {
            // Cells 档在终端侧没有图像可放，颜色由渲染层逐格铺到底色上。
            return Ok(());
        }
        let (cols, rows) = (cols.max(1), rows.max(1));
        let mut out = io::stdout();
        if self.placed != Some((cols, rows)) {
            // 旧 placement 的跨度已不成立：删掉它，再用新 id 传一张重排后的。
            if self.placed.is_some() {
                out.write_all(delete_seq(self.id).as_bytes())?;
                self.id = next_id();
            }
            out.write_all(&transmit_seq(self.id, cols, rows, &self.png))?;
            self.placed = Some((cols, rows));
        }
        out.write_all(place_seq(self.id, cols, rows).as_bytes())?;
        out.flush()
    }

    /// 显式删图（`/background off` 与退出时）。
    pub(crate) fn clear(&mut self) -> io::Result<()> {
        if !self.is_pixels() {
            // Cells 档只需让渲染层别再铺色。
            self.cells_size = None;
            return Ok(());
        }
        let mut out = io::stdout();
        out.write_all(delete_seq(self.id).as_bytes())?;
        self.placed = None;
        out.flush()
    }
}

/// 读盘 → 缩放 → 压暗。两个档位共用这条流水线，只差缩放上限与压暗系数。
fn prepare(path: &Path, mode: Mode) -> Result<image::RgbaImage, String> {
    // 先在头部拦两道（字节数 + 像素数）再解码：`shrink` 发生在解码之后，挡不住一张
    // 20000×20000 的图在 decode 阶段吃掉的几百 MiB。
    let len = std::fs::metadata(path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?
        .len();
    if len > MAX_BYTES {
        return Err(format!(
            "图片太大（{} MiB，上限 {} MiB）：{}",
            len / (1024 * 1024),
            MAX_BYTES / (1024 * 1024),
            path.display()
        ));
    }
    let reader = ImageReader::open(path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| format!("不是可识别的图片 {}：{e}", path.display()))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("不是可识别的图片 {}：{e}", path.display()))?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!(
            "图片像素过多（{w}×{h}，上限 {} MP）：{}",
            MAX_PIXELS / (1024 * 1024),
            path.display()
        ));
    }
    // `into_dimensions` 消耗了 reader，重新打开一次（只解析头部那步很便宜）。
    let img = ImageReader::open(path)
        .map_err(|e| format!("读不到 {}：{e}", path.display()))?
        .decode()
        .map_err(|e| format!("不是可识别的图片 {}：{e}", path.display()))?;
    let (edge, factor) = match mode {
        Mode::Pixels => (MAX_EDGE, DIM),
        Mode::Cells => (MAX_CELL_EDGE, CELL_DIM),
    };
    let mut rgba = shrink(img, edge).to_rgba8();
    dim(&mut rgba, factor);
    Ok(rgba)
}

/// 重编码成 PNG（`f=100` 直传用）。
fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("PNG 重编码失败：{e}"))?;
    Ok(out)
}

/// 超长边等比缩到 `edge` 以内（背景不需要原始分辨率，传大图只会拖慢首帧）。
fn shrink(img: DynamicImage, edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= edge && h <= edge {
        return img;
    }
    let scale = edge as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, FilterType::Lanczos3)
}

/// 缩到「列 × 行」，每格取平均色——马赛克背景的采样。
fn sample(img: &image::RgbaImage, cols: u16, rows: u16) -> BgCells {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let small =
        image::imageops::resize(img, u32::from(cols), u32::from(rows), FilterType::Triangle);
    BgCells {
        cols,
        rows,
        colors: small
            .pixels()
            .map(|p| Color::Rgb(p.0[0], p.0[1], p.0[2]))
            .collect(),
    }
}

/// 逐像素乘系数，保留 alpha（透明背景图不该被当成纯色块）。
fn dim(buf: &mut image::RgbaImage, factor: f32) {
    for px in buf.pixels_mut() {
        let [r, g, b, a] = px.0;
        let f = |c: u8| (f32::from(c) * factor).round().clamp(0.0, 255.0) as u8;
        px.0 = [f(r), f(g), f(b), a];
    }
}

/// `a=T`：传一张 PNG 并立即显示。`q=2` 让终端不回应答——应答会和全屏事件循环抢 stdin。
fn transmit_seq(id: u32, cols: u16, rows: u16, png: &[u8]) -> Vec<u8> {
    let payload = B64.encode(png).into_bytes();
    let mut out = Vec::with_capacity(payload.len() + 128);
    let mut start = 0;
    let mut first = true;
    loop {
        let end = (start + CHUNK).min(payload.len());
        let more = end < payload.len();
        // 只有第一块带完整控制键；后续块按协议只带 `m`。
        let ctrl = if first {
            format!(
                "a=T,f=100,q=2,i={id},c={cols},r={rows},m={}",
                u8::from(more)
            )
        } else {
            format!("m={}", u8::from(more))
        };
        out.extend_from_slice(b"\x1b_G");
        out.extend_from_slice(ctrl.as_bytes());
        out.push(b';');
        out.extend_from_slice(&payload[start..end]);
        out.extend_from_slice(b"\x1b\\");
        if !more {
            break;
        }
        start = end;
        first = false;
    }
    out
}

/// `a=p`：放置已有图像。`z=-1` 放到文字下方（背景层），`C=1` 不移动光标。
fn place_seq(id: u32, cols: u16, rows: u16) -> String {
    format!("\x1b_Ga=p,i={id},p=1,c={cols},r={rows},z=-1,C=1,q=2\x1b\\")
}

/// `a=d,d=i`：按 image id 删除。
fn delete_seq(id: u32) -> String {
    format!("\x1b_Ga=d,d=i,i={id},q=2\x1b\\")
}

/// 客户端自选 image id（协议要求非零）。用纳秒低位 + 进程内递增，避免连续调用撞号。
fn next_id() -> u32 {
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (nanos ^ seq.wrapping_mul(2) | 1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_scales_rgb_and_keeps_alpha() {
        let mut img = image::RgbaImage::from_pixel(2, 1, image::Rgba([200, 100, 0, 128]));
        dim(&mut img, 0.5);
        assert_eq!(
            img.get_pixel(0, 0).0,
            [100, 50, 0, 128],
            "RGB 减半，alpha 不动"
        );
        // 全黑仍是全黑（乘法不会把暗部抬起来）。
        let mut black = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        dim(&mut black, DIM);
        assert_eq!(black.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn short_payload_fits_in_one_chunk() {
        let seq = transmit_seq(7, 80, 24, &[1, 2, 3, 4]);
        let text = String::from_utf8_lossy(&seq);
        assert_eq!(text.matches("\x1b_G").count(), 1, "单块只发一次 APC");
        assert!(text.contains("a=T,f=100,q=2,i=7,c=80,r=24,m=0"));
        assert!(text.ends_with("\x1b\\"));
    }

    #[test]
    fn long_payload_chunks_on_four_byte_boundaries() {
        // 9000 字节 → base64 12000 字符，应切成 3 块（4096/4096/3808）。
        let seq = transmit_seq(9, 10, 5, &vec![0u8; 9000]);
        let text = String::from_utf8_lossy(&seq);
        let chunks: Vec<&str> = text.split("\x1b_G").skip(1).collect();
        assert_eq!(chunks.len(), 3, "按 4096 分块");
        for (i, chunk) in chunks.iter().enumerate() {
            let (ctrl, payload) = chunk.split_once(';').expect("控制键与载荷用 ; 分隔");
            let payload = payload.trim_end_matches("\x1b\\");
            assert!(payload.len() <= CHUNK, "第 {i} 块超长");
            assert_eq!(payload.len() % 4, 0, "第 {i} 块不是 4 的倍数");
            // 只有第一块带完整控制键，其余只带 m。
            assert_eq!(ctrl.contains("a=T"), i == 0, "第 {i} 块控制键不对");
            assert_eq!(ctrl.contains("m=0"), i == 2, "只有末块 m=0");
        }
    }

    #[test]
    fn place_and_delete_carry_background_semantics() {
        let place = place_seq(3, 120, 40);
        assert!(place.contains("a=p"));
        assert!(place.contains("z=-1"), "负 z 才在文字下方");
        assert!(place.contains("C=1"), "放置不得移动光标");
        assert!(place.contains("c=120,r=40"));

        let del = delete_seq(3);
        assert!(del.contains("a=d"));
        assert!(del.contains("d=i,i=3"));
    }

    /// 格子档：采样表必须正好「列 × 行」个格子，颜色取自图（纯色图 → 处处同色）。
    #[test]
    fn cells_mode_samples_one_color_per_cell() {
        let path = temp_png("cells.png", [200, 100, 50, 255]);
        let rgba = prepare(&path, Mode::Cells).expect("预处理成功");
        let cells = sample(&rgba, 10, 4);
        assert_eq!(cells.size(), (10, 4), "采样表尺寸 = 终端格子数");
        let top_left = cells.get(0, 0, 10, 4).expect("有颜色");
        assert_eq!(
            top_left,
            cells.get(9, 3, 10, 4).expect("有颜色"),
            "纯色图任意格应同色"
        );
        let Color::Rgb(r, _, _) = top_left else {
            panic!("格子底色必须是 RGB：{top_left:?}");
        };
        assert!(r > 0 && r < 200, "应被压暗但仍看得见，实际 {r}");
        let _ = std::fs::remove_file(&path);
    }

    /// 终端尺寸与采样表不一致时按比例映射 —— resize 后到重采样之间会短暂处于这种状态。
    #[test]
    fn cells_get_maps_across_size_mismatch() {
        let path = temp_png("map.png", [10, 20, 30, 255]);
        let rgba = prepare(&path, Mode::Cells).expect("预处理成功");
        let cells = sample(&rgba, 4, 2);
        // 采样表 4×2，却按 8×4 查询：每格都要拿到颜色，且不能越界 panic。
        for y in 0..4 {
            for x in 0..8 {
                assert!(cells.get(x, y, 8, 4).is_some(), "({x},{y}) 应有颜色");
            }
        }
        assert_eq!(
            cells.get(7, 3, 8, 4),
            cells.get(0, 0, 8, 4),
            "纯色图缩放到任意格仍是同色"
        );
        assert!(cells.get(0, 0, 0, 0).is_none(), "零尺寸不该崩");
        let _ = std::fs::remove_file(&path);
    }

    /// 写一张纯色测试 PNG，返回路径。
    fn temp_png(name: &str, rgba: [u8; 4]) -> PathBuf {
        let dir = std::env::temp_dir().join("crabmate-tui-bg-test");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(name);
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
        image::DynamicImage::ImageRgba8(img)
            .save(&path)
            .expect("写测试 PNG");
        path
    }

    #[test]
    fn ids_are_nonzero_and_distinct() {
        let a = next_id();
        let b = next_id();
        assert_ne!(a, 0, "协议要求 id 非零");
        assert_ne!(b, 0);
        assert_ne!(a, b, "连续两次分配不能撞号");
    }

    /// 端到端最强的一条：把多条 APC 里的载荷拼回去，必须还原成原始字节。
    /// 分块边界、base64、控制键拆分任何一处错位都会在这里暴露。
    #[test]
    fn chunks_round_trip_back_to_the_original_bytes() {
        let png: Vec<u8> = (0..=255u8).cycle().take(7000).collect();
        let seq = transmit_seq(5, 80, 24, &png);
        let text = String::from_utf8_lossy(&seq);
        let mut joined = String::new();
        for chunk in text.split("\x1b_G").skip(1) {
            let (_, payload) = chunk.split_once(';').expect("每块都有控制键与载荷");
            joined.push_str(payload.trim_end_matches("\x1b\\"));
        }
        let decoded = B64
            .decode(joined.as_bytes())
            .expect("拼接后应是合法 base64");
        assert_eq!(decoded, png, "分块拼接必须还原原始 PNG 字节");
    }

    /// 真实文件走一遍 解码 → 缩放 → 压暗 → 重编码，确认压暗真的落到了像素上。
    #[test]
    fn prepare_png_dims_a_real_image() {
        let dir = std::env::temp_dir().join("crabmate-tui-bg-test");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("white.png");
        let white = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
        image::DynamicImage::ImageRgba8(white)
            .save(&path)
            .expect("写测试 PNG");

        let rgba = prepare(&path, Mode::Pixels).expect("预处理成功");
        let png = encode_png(&rgba).expect("重编码");
        let back = image::load_from_memory(&png)
            .expect("重编码的 PNG 可解码")
            .to_rgba8();
        let px = back.get_pixel(0, 0).0;
        assert!(px[0] < 255, "白色应被压暗，实际 {}", px[0]);
        assert!(px[0] > 0, "不该压成纯黑，实际 {}", px[0]);
        assert_eq!(px[3], 255, "alpha 保留");

        let _ = std::fs::remove_file(&path);
    }
}
