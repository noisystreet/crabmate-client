//! [`super`]（status.rs）的分段取舍 / 宽度预算 / 运行指示单测。
//!
//! 独立成文件：与 md_tests / settings_panel / render 同理，避免 status.rs 越过
//! 单文件 920 行的 fn-nloc 门禁。

use ratatui::style::Color;

use super::*;

fn base_info(api_base: &str) -> StatusInfo {
    StatusInfo {
        api_base: api_base.into(),
        model: None,
        role: None,
        mode: None,
        running: false,
        cancel_sent: false,
        spinner: 0,
        view_offset: 0,
        search_term: None,
        search_total: 0,
    }
}

#[test]
fn host_port_strips_scheme_and_path() {
    assert_eq!(host_port("http://127.0.0.1:8080"), "127.0.0.1:8080");
    assert_eq!(
        host_port("https://cloud.example.com/crabmate/base"),
        "cloud.example.com"
    );
    assert_eq!(host_port("  http://x  "), "x");
    // 没有 scheme 时原样保留首段，不做猜测。
    assert_eq!(host_port("localhost:9000"), "localhost:9000");
}

#[test]
fn serve_shows_full_base_when_it_fits_and_shortens_only_when_narrow() {
    let info = base_info("https://cloud.example.com/crabmate");
    // 宽终端：完整 base（含路径前缀）可见—— TUI 里没有第二个入口能看全量 URL。
    let wide = status_text(&info, 200);
    assert!(
        wide.contains("serve https://cloud.example.com/crabmate"),
        "{wide}"
    );
    // 窄终端：退到 host:port，否则一整行只够 serve 一项。
    let narrow = status_text(&info, 40);
    assert!(narrow.contains("serve cloud.example.com"), "{narrow}");
    assert!(!narrow.contains("/crabmate"), "{narrow}");
}

#[test]
fn narrow_bar_keeps_only_the_top_priority_segment() {
    let mut info = base_info("http://127.0.0.1:8080");
    info.model = Some("gpt-x".into());
    info.mode = Some("plan".into());
    info.search_term = Some("ab".into());
    info.search_total = 2;
    // 40 列：find(14) 之后连 `serve`(27) 都放不下 → **严格优先级**到此收手，
    // 不允许 `model` 之类的次要项插队（否则会出现「model 在、serve 没了」）。
    let text = status_text(&info, 40);
    assert!(text.contains("find"), "搜索反馈必须常驻：{text}");
    assert!(!text.contains("serve"), "serve 放不下就整体不收：{text}");
    assert!(!text.contains("model"), "更低的 model 更不该插队：{text}");
    assert!(!text.contains("conv"), "conv 应被丢弃：{text}");
}

/// 宽终端下每一段都在：本地 override 的 `*` 标记跟着值一起显示。
#[test]
fn wide_bar_lists_every_segment() {
    let mut info = base_info("http://127.0.0.1:8080");
    info.model = Some("gpt-x*".into());
    info.role = Some("coder".into());
    info.mode = Some("plan".into());
    let text = status_text(&info, 200);
    // 宽终端预算够：`serve` 显示完整 base（TUI 里没有第二个入口能看全量 URL）。
    assert!(text.contains("serve http://127.0.0.1:8080"), "{text}");
    assert!(text.contains("model gpt-x*"), "{text}");
    assert!(text.contains("role coder"), "{text}");
    assert!(text.contains("mode plan"), "{text}");
    assert!(
        !text.contains("conv"),
        "会话 id 不上状态栏（desktop 也没有）：{text}"
    );
    assert!(text.contains("○ 空闲"), "{text}");
}

#[test]
fn serve_default_model_shows_up_without_a_mode() {
    let mut info = base_info("http://x");
    info.model = Some("deepseek".into());
    let text = status_text(&info, 200);
    assert!(text.contains("model deepseek"), "{text}");
    // 未设置的值不该凭空出现（`model ` 里没有 `mode ` 子串）。
    assert!(!text.contains("mode "), "{text}");
}

#[test]
fn search_and_view_back_segments_show_up() {
    let mut info = base_info("http://x");
    info.view_offset = 12;
    info.search_term = Some("grep".into());
    info.search_total = 2;
    let text = status_text(&info, 200);
    assert!(text.contains("find 「grep」"), "{text}");
    assert!(text.contains("↑12"), "{text}");
}

/// 窄屏滚动时回看提示优先于常驻项：它是那一瞬间唯一在变、也唯一能回答
/// 「我在哪 / 下面还有多少」的反馈，丢掉等于这次滚动没有回应。
#[test]
fn view_back_outranks_serve_on_a_narrow_bar() {
    let mut info = base_info("http://127.0.0.1:8080");
    info.view_offset = 12;
    // 40 列：右缘 `RIGHT_W`(8) → 左区预算 32。`↑12`(3) + 分隔(3) + serve(27) = 33
    // 差一列装不下，于是让位的是 serve（滚动一停它就回来）。
    let text = status_text(&info, 40);
    assert!(text.contains("↑12"), "滚动反馈必须留下：{text}");
    assert!(!text.contains("serve"), "常驻项让位给滚动反馈：{text}");
}

/// 回看提示定宽：跨过 10 的倍数时宽度不变。
///
/// 不定宽的话 `↑9`(3 列) → `↑10`(4 列) 会让左区预算忽多忽少，临界宽度下
/// `serve` 会随滚轮闪进闪出（`view_back_outranks_serve_on_a_narrow_bar` 正是
/// 那个临界点）。
#[test]
fn view_back_width_is_stable_across_digit_boundaries() {
    let width_at = |n: usize| {
        let mut info = base_info("http://x");
        info.view_offset = n;
        view_back_text(&info).map(|s| UnicodeWidthStr::width(s.as_str()))
    };
    assert_eq!(width_at(9), width_at(10), "一位数到两位数宽度必须不变");
    assert_eq!(width_at(10), width_at(99), "两位数内宽度必须不变");
}

#[test]
fn serve_outranks_model_when_both_compete() {
    let mut info = base_info("http://127.0.0.1:8080");
    info.model = Some("gpt-x".into());
    info.search_term = Some("ab".into());
    info.search_total = 2;
    // 60 列：find(14) + serve(27) + 分隔(3) = 44 装得下，`model` 就轮不上了。
    let text = status_text(&info, 60);
    assert!(text.contains("find"), "{text}");
    assert!(text.contains("serve"), "serve 优先于 model：{text}");
    assert!(!text.contains("model"), "model 应被丢弃：{text}");
}

#[test]
fn dropped_segments_never_truncate_the_tail() {
    let mut info = base_info("http://127.0.0.1:8080");
    info.model = Some("gpt-x".into());
    info.search_term = Some("needle".into());
    info.search_total = 3;
    // 20 列：只够 find 独占（它自己也超宽，按宽截断），`conv` / `model` 整段丢弃，
    // 而不是被砍成半截留在行尾。
    let text = status_text(&info, 20);
    assert!(!text.contains("conv"), "丢弃是整段丢弃：{text}");
    assert!(!text.contains("model"), "丢弃是整段丢弃：{text}");
    assert!(text.starts_with("find"), "留下的应是最高优先级段：{text}");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 20, "整行铺满");
}

#[test]
fn labels_use_muted_and_values_use_fg() {
    let pal = Palette::default();
    let mut info = base_info("http://127.0.0.1:8080");
    // 至少两段才有分隔符可断言（`conv` 段移除后，单段不再插分隔符）。
    info.model = Some("gpt-x".into());
    let line = status_line(&info, 200, &pal);
    let spans = &line.spans;
    assert_eq!(spans[0].content, "serve");
    assert_eq!(spans[0].style.fg, Some(pal.muted), "label 走淡色");
    assert_eq!(spans[1].content, " ");
    assert_eq!(spans[2].style.fg, Some(pal.fg), "值走正文色");
    // 分隔符必须是极暗竖线，否则窄终端里像多了一列正文。
    assert!(
        spans.iter().any(|s| s.content == SEPARATOR),
        "缺分隔符：{spans:?}"
    );
    assert_eq!(
        spans
            .iter()
            .find(|s| s.content == SEPARATOR)
            .map(|s| s.style.fg),
        Some(Some(pal.border))
    );
}

/// 生成中右缘的字形 span。
///
/// 取尾部而不是固定下标：左区选了几段、右缘前置补了几个空格都不影响这里。
/// 只挑字形 span（不按固定步长跳），这样条里有没有格间空格都对。
fn bar_cells<'a>(line: &'a Line<'static>) -> Vec<&'a Span<'static>> {
    let tail = &line.spans[line.spans.len() - BAR_W..];
    tail.iter()
        .filter(|s| s.content.as_ref() == BAR_CELL_DIM || s.content.as_ref() == BAR_CELL_LIT)
        .collect()
}

/// 字形常量里唯一的那个字符（字形都是单字符）。
fn glyph_of(cell: &str) -> char {
    cell.chars().next().expect("字形不能为空")
}

fn dim_ch() -> char {
    glyph_of(BAR_CELL_DIM)
}

fn lit_ch() -> char {
    glyph_of(BAR_CELL_LIT)
}

/// 行尾仪器条的 6 个字形（跳过分隔空格）。
///
/// 断言形态时不能用字面串：`●` 的位置随帧号变，只有「字符合法 + 恰好一个实心」
/// 才是稳定不变式。
fn bar_glyphs(text: &str) -> Vec<char> {
    text.chars()
        .rev()
        .take(BAR_W)
        .filter(|c| *c != ' ')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// 取行的前 `width` 列（按显示列切，不按字符切——`✳ 取消中` 是 8 列 5 字符）。
fn head_cols(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut acc = 0usize;
    let mut out = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if acc + w > width {
            break;
        }
        acc += w;
        out.push(ch);
    }
    out
}

#[test]
fn running_indicator_right_aligned_across_states() {
    let pal = Palette::default();
    let mut info = base_info("http://x");
    info.model = Some("gpt-x".into());
    let mut head: Option<String> = None;
    for (running, cancel, want) in [
        (false, false, Some("○ 空闲")),
        (true, false, None),
        (true, true, Some("✳ 取消中")),
    ] {
        info.running = running;
        info.cancel_sent = cancel;
        let line = status_line(&info, 40, &pal);
        let text = line.to_string();
        match want {
            Some(w) => assert!(text.ends_with(w), "{text} 应以 {w} 结尾"),
            // 仪器条的实心格位置随帧号变，不能写死字面串。
            None => {
                let glyphs = bar_glyphs(&text);
                assert!(
                    glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
                    "应该只有仪器条字形：{text}"
                );
                assert_eq!(
                    glyphs.iter().filter(|c| **c == lit_ch()).count(),
                    BAR_BLOCK,
                    "应恰好一颗方块点亮：{text}"
                );
            }
        }
        assert_eq!(UnicodeWidthStr::width(text.as_str()), 40, "整行铺满");
        // 三态的左区必须逐字相同：右缘统一到 `RIGHT_W` 列，切换时 `serve` / `model`
        // 才不会整体平移。这是「统一宽度」要买的那一个性质。
        let now = head_cols(&text, 40 - RIGHT_W);
        match &head {
            Some(first) => assert_eq!(&now, first, "左区随状态变了：{text}"),
            None => head = Some(now),
        }
    }
}

#[test]
fn bar_cell_is_one_column() {
    // 两个字形都必须 1 列：仪器条靠「6 格 × 1 列」参与预算，
    // 任一被算成 2 列（或终端按全角渲染）右缘就会被推出屏幕。
    for cell in [BAR_CELL_DIM, BAR_CELL_LIT] {
        assert_eq!(UnicodeWidthStr::width(cell), 1, "{cell:?} 必须 1 列");
    }
    // 亮 / 灭必须等宽，否则扫描时行宽会跟着字形变。
    assert_eq!(
        UnicodeWidthStr::width(BAR_CELL_DIM),
        UnicodeWidthStr::width(BAR_CELL_LIT)
    );
}

#[test]
fn bar_block_walks_with_frame() {
    let pal = Palette::default();
    let mut info = base_info("http://x");
    info.running = true;
    // 覆盖两圈：每 `BAR_TICKS_PER_CELL` 拍，亮块右移一格。
    let mut prev_start: Option<usize> = None;
    for frame in 0..BAR_CELLS * BAR_TICKS_PER_CELL * 2 {
        info.spinner = frame;
        let start = (frame / BAR_TICKS_PER_CELL) % BAR_CELLS;
        let want: Vec<usize> = (0..BAR_CELLS)
            .filter(|i| (i + BAR_CELLS - start) % BAR_CELLS < BAR_BLOCK)
            .collect();
        let line = status_line_with(&info, 60, &pal, false);
        let cells = bar_cells(&line);
        let lit: Vec<usize> = (0..BAR_CELLS)
            .filter(|i| cells[*i].style.fg == Some(pal.accent))
            .collect();
        assert_eq!(lit, want, "第 {frame} 拍亮块应从第 {start} 格起");
        // 块长固定：单颗，不多不少。数量变化会被读成进度。
        assert_eq!(lit.len(), BAR_BLOCK, "块长应恒为 {BAR_BLOCK}");
        // 亮块必须是**单一连续段**（含环绕）：只有一个「灭→亮」的入口。
        // 散点或两段就没有可追踪的边界，平移也就读不出来。
        let rises = (0..BAR_CELLS)
            .filter(|i| {
                let cur = want.contains(i);
                let prev = want.contains(&((i + BAR_CELLS - 1) % BAR_CELLS));
                cur && !prev
            })
            .count();
        assert_eq!(rises, 1, "第 {frame} 拍亮块应是单一连续段：{want:?}");
        // 熄灭格仍要可见，否则块的边界看不出来。
        for i in (0..BAR_CELLS).filter(|i| !want.contains(i)) {
            assert_eq!(cells[i].style.fg, Some(pal.muted), "第 {i} 格应熄灭");
            assert_eq!(cells[i].content, BAR_CELL_DIM, "熄灭格应是熄灭字形");
        }
        // 起点每换一拍就前进一格（模 6）——「行进」就是这么来的。
        // 同一相位持续 `BAR_TICKS_PER_CELL` 拍，只在真的换格时比。
        if let Some(prev) = prev_start
            && prev != start
        {
            assert_eq!(start, (prev + 1) % BAR_CELLS, "起点应逐拍前进一格");
        }
        prev_start = Some(start);
    }
}

#[test]
fn bar_keeps_constant_width_across_frames() {
    // 换格不改预算：右缘宽度只由「形态」决定，状态行才不会随帧左右抖。
    let pal = Palette::default();
    let mut info = base_info("http://x");
    info.running = true;
    let mut widths = std::collections::BTreeSet::new();
    // 含极大帧号：`(spinner / 2) % BAR_CELLS` 必须对任意 usize 安全，且不改字形。
    for frame in (0..BAR_CELLS + 3).chain([usize::MAX]) {
        info.spinner = frame;
        let text = status_line_with(&info, 60, &pal, false).to_string();
        // 仪器条的实心格随帧号移动，只断言「字符合法」；位置由上面那个测试守。
        let glyphs = bar_glyphs(&text);
        assert!(
            glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
            "帧 {frame}: {text}"
        );
        widths.insert(UnicodeWidthStr::width(text.as_str()));
    }
    assert_eq!(widths.len(), 1, "各帧行宽必须一致：{widths:?}");
}

#[test]
fn reduced_motion_lights_the_whole_bar() {
    let pal = Palette::default();
    let mut info = base_info("http://x");
    info.running = true;
    info.spinner = 8;
    // 动效关掉后条不扫描，改成整条常亮：静止的「全亮条」与「○ 空闲」靠形状就能分辨，
    // 也就不必再加「生成中」字样（右缘定宽是硬约束）。
    let still = status_line_with(&info, 60, &pal, true);
    let cells = bar_cells(&still);
    assert!(
        cells.iter().all(|s| s.style.fg == Some(pal.accent)),
        "静态时应整条点亮：{cells:?}"
    );
    // 亮态必须是实心方块：形状与颜色一起变，底色消失时也分得出「运行中」和「空闲」。
    assert!(
        cells.iter().all(|s| s.content == BAR_CELL_LIT),
        "静态时应整条是实心方块：{cells:?}"
    );
    // 同一份输入在开动效时变成「亮块从第 4 格起」——证明开关真的生效。
    let moving = status_line_with(&info, 60, &pal, false);
    let moving_cells = bar_cells(&moving);
    assert_eq!(moving_cells[4].style.fg, Some(pal.accent));
    assert_eq!(moving_cells[1].style.fg, Some(pal.muted));
}

#[test]
fn narrow_bar_keeps_only_the_bar() {
    // 右缘固定 8 列，窄终端也照旧：`serve` 被截断，但状态指示不会被挤掉。
    let pal = Palette::default();
    let mut info = base_info("http://long");
    info.model = Some("model-name".into());
    info.running = true;
    let text = status_line_with(&info, 16, &pal, false).to_string();
    // 窄条右缘仍是完整的 8 列（6 格 + 前导补空），只是左区被截短。
    let glyphs = bar_glyphs(&text);
    assert!(
        glyphs.iter().all(|c| *c == dim_ch() || *c == lit_ch()),
        "窄条也保留仪器条：{text}"
    );
    assert_eq!(
        glyphs.iter().filter(|c| **c == lit_ch()).count(),
        BAR_BLOCK,
        "窄条照样一颗完整方块：{text}"
    );
    assert!(!text.contains("生成中"), "生成中已不再带文案：{text}");
    assert_eq!(UnicodeWidthStr::width(text.as_str()), 16, "仍恰好铺满");
}

#[test]
fn mono_bar_falls_back_to_reversed_cell() {
    // `mono` 下所有槽都是 `Reset`，不换修饰位就完全看不出哪一格在动。
    let pal = Palette::mono();
    let mut info = base_info("http://x");
    info.running = true;
    // 4 拍 = 亮块首格前进到第 2 格（每 2 拍走一格）：2/3/4 亮，0/1/5 灭。
    info.spinner = 4;
    let line = status_line_with(&info, 60, &pal, false);
    let cells = bar_cells(&line);
    assert!(
        cells[2].style.add_modifier.contains(Modifier::REVERSED),
        "点亮格应走反色：{:?}",
        cells[2]
    );
    assert!(
        cells[1].style.add_modifier.contains(Modifier::DIM),
        "熄灭格应走 DIM：{:?}",
        cells[1]
    );
    assert!(
        cells
            .iter()
            .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset)),
        "无色模式不该出现彩色 span"
    );
}

#[test]
fn mono_palette_drops_every_color() {
    // 无色模式：分段取舍与文本不变，但不再有任何一个 span 指定彩色
    // （`Palette::mono()` 的所有槽都是 `Reset`）。
    let info = base_info("http://127.0.0.1:8080");
    let line = status_line(&info, 80, &Palette::mono());
    let text = line.to_string();
    assert!(text.contains("serve"), "{text}");
    assert!(
        !text.contains("conv"),
        "会话 id 不上状态栏（desktop 也没有）：{text}"
    );
    assert!(
        line.spans
            .iter()
            .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset)),
        "无色模式下不该有彩色 span：{:?}",
        line.spans
    );
}
