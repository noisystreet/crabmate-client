//! 快捷键的「键位 + 人读措辞」表：设置面板「快捷键」栏与 `/help` 的**单一来源**。
//!
//! 单独成文件有两个原因：
//!
//! 1. #433 的验收就是「新增一个顶层热键后，`/settings` 的快捷键栏不改代码就能显示它」。
//!    要做到这点，措辞必须与判定同源：`TopHotkey` 的每个变体在这里都有一条展示条目，
//!    [`TopHotkey::doc_index`] 是**穷尽 match**，加变体不补措辞编译不过。
//! 2. `hotkeys.rs` 里的判定与测试贴着 `fn-nloc` 门禁的观测方式：`lizard` 的 C 风格词法器
//!    把单引号当字符字面量的开头，会一路吞到下一个单引号（连同中间的花括号），于是函数
//!    跨度被算成一整团。表本身因此一个单引号都不写（`const` 里的 `&str` 同样是静态生命周期，
//!    只是写法上不带那个关键字）；单测里的字符字面量成对出现，词法器扫得过去。
//!
//! 分组语义是「在哪儿按」而不是「谁实现」：`F4` 的实现跨到外壳层，但它属全局。

use super::TopHotkey;

/// 快捷键一览的分组：面板「快捷键」栏按它分行，`/help` 也按同一份顺序拼。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotkeyGroup {
    /// 顶层功能键：任何焦点都能按（输入框 / 会话栏 / 工作区树下）。
    Global,
    /// 对话区（输入框 + transcript）。
    Chat,
    /// 设置面板内。
    Panel,
    /// 编辑器 / 文件视图内。
    Editor,
}

impl HotkeyGroup {
    /// 面板行与 `/help` 的显示顺序。
    pub(crate) const ALL: [HotkeyGroup; 4] = [
        HotkeyGroup::Global,
        HotkeyGroup::Chat,
        HotkeyGroup::Panel,
        HotkeyGroup::Editor,
    ];

    /// 分组裸名（面板行标签在 `field_label` 里另加「键」字，`/help` 用这里的裸名）。
    pub(crate) fn label(self) -> String {
        match self {
            Self::Global => "全局",
            Self::Chat => "对话",
            Self::Panel => "面板",
            Self::Editor => "编辑器",
        }
        .to_string()
    }
}

/// 顶层热键的展示条目：`(键位, 人读措辞, 分组)`，顺序与 [`TopHotkey::doc_index`] 一致。
const TOP_DOCS: [(&str, &str, HotkeyGroup); 8] = [
    ("F6", "设置面板", HotkeyGroup::Global),
    ("F3", "状态栈展开/折叠", HotkeyGroup::Global),
    ("F4", "跳到在等审批的连接", HotkeyGroup::Global),
    ("Alt+←", "切上一条连接", HotkeyGroup::Global),
    ("Alt+→/Ctrl+N", "切下一条连接", HotkeyGroup::Global),
    ("Alt+1..8", "直达第 n 条连接", HotkeyGroup::Global),
    ("Ctrl+T", "聊天/文件视图切换", HotkeyGroup::Chat),
    (
        "Ctrl+Y",
        "复制最近一条回复（文件视图里是文件路径）",
        HotkeyGroup::Chat,
    ),
];

/// 不属于 [`TopHotkey`] 的键（判定不在 `hotkeys::of` 的查表里），但同样要进面板与 `/help`。
///
/// 分两类：对话区那几条就是 `hotkeys.rs` 里的 `is_*` 判据；面板与编辑器那几条判定在
/// 设置面板的状态机与 `editor_view` / `input_keys` 里（它们本来就没有可查询的 id），
/// 这里只产出文案 —— 改键位时那两处各自都有单测，措辞漂移则由「面板与表同源」的单测看守。
const FIXED_DOCS: [(&str, &str, HotkeyGroup); 15] = [
    ("Ctrl+C", "取消回合（再按一次强退）", HotkeyGroup::Chat),
    ("Ctrl+E", "思考展开/折叠", HotkeyGroup::Chat),
    ("Ctrl+O", "提交输入（Enter 同义）", HotkeyGroup::Chat),
    ("Ctrl+W", "进工作区目录树", HotkeyGroup::Chat),
    ("Alt+Enter", "输入框内换行", HotkeyGroup::Chat),
    ("↑↓", "移动行", HotkeyGroup::Panel),
    ("Tab/Shift+Tab", "切分区", HotkeyGroup::Panel),
    ("Enter", "编辑", HotkeyGroup::Panel),
    ("F2/S", "保存", HotkeyGroup::Panel),
    ("Esc", "关闭（有改动先确认）", HotkeyGroup::Panel),
    ("PgUp/PgDn", "翻页", HotkeyGroup::Editor),
    ("退格", "关预览", HotkeyGroup::Editor),
    ("滚轮", "滚动文件", HotkeyGroup::Editor),
    ("Ctrl+F", "文件内查找（不是 /find）", HotkeyGroup::Editor),
    (
        "Ctrl+Y",
        "复制当前文件路径（聊天视图里是复制回复）",
        HotkeyGroup::Editor,
    ),
];

impl TopHotkey {
    /// 全部顶层热键，**顺序即显示顺序**。
    ///
    /// 新增变体时编译会先在 [`TopHotkey::doc_index`] 的穷尽 match 上失败（强制补条目），
    /// 再把它加到这里；漏加的表现是面板少一项。
    pub(super) const ALL: [TopHotkey; 8] = [
        TopHotkey::Settings,
        TopHotkey::StatusStack,
        TopHotkey::NextApproval,
        TopHotkey::PrevLane,
        TopHotkey::NextLane,
        TopHotkey::GotoLane(1),
        TopHotkey::ToggleView,
        TopHotkey::Copy,
    ];

    /// 该热键在 [`TOP_DOCS`] 里的下标。
    ///
    /// **穷尽 match**：新增变体不补措辞编译不过。`GotoLane` 的具体序号不参与措辞，
    /// `Alt+1..8` 是一个整体说明。
    const fn doc_index(self) -> usize {
        match self {
            Self::Settings => 0,
            Self::StatusStack => 1,
            Self::NextApproval => 2,
            Self::PrevLane => 3,
            Self::NextLane => 4,
            Self::GotoLane(_) => 5,
            Self::ToggleView => 6,
            Self::Copy => 7,
        }
    }
}

/// 条目显示成一行：`键位 措辞`。
fn item_text((keys, text, _): (&str, &str, HotkeyGroup)) -> String {
    format!("{keys} {text}")
}

/// 某一分组的全部条目（面板按宽度换行、`/help` 拼成一行，都用它）。
pub(crate) fn group_items(group: HotkeyGroup) -> Vec<String> {
    // 直接按 `doc_index` 索引表：不在中间函数里写返回值类型，就能避开静态生命周期关键字
    // （本文件刻意一个单引号都不出现，理由见文件头）。
    let top = TopHotkey::ALL.iter().map(|h| TOP_DOCS[h.doc_index()]);
    top.chain(FIXED_DOCS)
        .filter(|doc| doc.2 == group)
        .map(item_text)
        .collect()
}

/// 某一分组的一行文案：`键 动作 · 键 动作`（`/help` 用；面板自己按宽度换行）。
pub(crate) fn group_line(group: HotkeyGroup) -> String {
    group_items(group).join(" · ")
}

#[cfg(test)]
mod tests {
    use super::super::of;
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// 每个顶层热键都必须**恰好**有一条展示条目（面板与 `/help` 的唯一来源）。
    ///
    /// `doc_index()` 是穷尽 match，所以「加了变体忘了写措辞」编译不过；这里守的是另一半：
    /// 措辞写了但没进 `ALL`（面板少一项）或进重了。
    #[test]
    fn every_top_hotkey_has_exactly_one_doc_entry() {
        for hotkey in TopHotkey::ALL {
            let item = item_text(TOP_DOCS[hotkey.doc_index()]);
            let hits: usize = HotkeyGroup::ALL
                .iter()
                .map(|g| group_items(*g).iter().filter(|i| **i == item).count())
                .sum();
            assert_eq!(hits, 1, "{item} 在分组条目里出现 {hits} 次");
        }
    }

    /// 分组条目总数 = 顶层热键 + 固定表；四个分组都非空。
    ///
    /// 多一项说明 `ALL` 里有重复，少一项说明 `FIXED_DOCS` 的条目没被任何分组认领。
    #[test]
    fn group_items_cover_both_tables() {
        let total: usize = HotkeyGroup::ALL.iter().map(|g| group_items(*g).len()).sum();
        assert_eq!(total, TopHotkey::ALL.len() + FIXED_DOCS.len());
        for group in HotkeyGroup::ALL {
            assert!(!group_items(group).is_empty(), "{group:?} 没有条目");
            assert!(!group_line(group).is_empty());
            assert!(!group.label().is_empty());
        }
    }

    /// 条目措辞与 `of()` 的判定同源：抽样按键 → 热键 → 条目里是那一条键位。
    #[test]
    fn doc_keys_match_the_actual_bindings() {
        let samples = [
            (KeyCode::F(6), KeyModifiers::NONE, "F6"),
            (KeyCode::F(3), KeyModifiers::NONE, "F3"),
            (KeyCode::F(4), KeyModifiers::NONE, "F4"),
            (KeyCode::Char('t'), KeyModifiers::CONTROL, "Ctrl+T"),
            (KeyCode::Char('y'), KeyModifiers::CONTROL, "Ctrl+Y"),
            (KeyCode::Left, KeyModifiers::ALT, "Alt+←"),
            // 两个键同一语义，条目里两条键位都写着 —— 抽样用哪一条都得命中它。
            (KeyCode::Right, KeyModifiers::ALT, "Alt+→/Ctrl+N"),
            (KeyCode::Char('2'), KeyModifiers::ALT, "Alt+1..8"),
        ];
        for (code, modifiers, keys) in samples {
            let hotkey =
                of(&KeyEvent::new(code, modifiers)).unwrap_or_else(|| panic!("{keys} 未命中"));
            assert_eq!(TOP_DOCS[hotkey.doc_index()].0, keys);
        }
    }

    /// 同一个键位（`Ctrl+Y`）在两种视图里是两件事：两条措辞都得把**另一边的**语义写出来，
    /// 否则看面板的人会以为其中一条是错的。
    #[test]
    fn ctrl_y_documents_both_views() {
        let chat = TOP_DOCS[TopHotkey::Copy.doc_index()].1;
        let editor = FIXED_DOCS
            .iter()
            .find(|doc| doc.0 == "Ctrl+Y")
            .map(|doc| doc.1)
            .expect("编辑器分组里要有 Ctrl+Y");
        assert!(chat.contains("路径"), "{chat}");
        assert!(editor.contains("回复"), "{editor}");
    }
}
