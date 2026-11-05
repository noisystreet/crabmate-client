//! `/help` 的文案表。
//!
//! 单独成文件是为了压住 `mod.rs` 的 920 行门禁：事件循环层只保留「逐行推进 transcript」，
//! 文案随功能增删时也只改这里。顺序即显示顺序，按「命令 → 按键 → 各分区操作」分段。
//!
//! **快捷键那几行不在这里手写**：由 [`super::hotkeys`] 的键位表生成，加一个顶层热键时
//! `/help` 与设置面板「快捷键」栏一起自动跟上（#433）。这里的两个常量只放判定散在各
//! 模块、没有热键条目的那几条键。

use super::hotkeys::{self, HotkeyGroup};
use super::state::{LineKind, UiState};

/// 命令清单（`/x`）。
const COMMAND_LINES: &[&str] = &[
    "本地命令：/quit（/exit、/q 同义） /help /status /settings /mode /role /perm /model /find /background /workspace /resume /older /fold /conv /copy /ctl /lane /editor /chat /version /skills /context /config /agent /export",
    "输入 / 弹出补全菜单（内建命令 + skill；↑↓ 选择 · Tab/Enter 补全 · Esc 关闭）",
    "/copy [assistant|user|path]（Ctrl+Y 同义）：复制到剪贴板 —— 默认最近一条助手回复，`user` 你自己的上一条，`path` 当前文件视图里那个文件的**相对路径**（视图开着时不写目标也走路径；内容是终端原生选择，不走这条）",
    "/ctl：查 crabmatectl 控制面实例（本地 unix socket，只读；需 --ctl-socket 或默认 socket 已存在）",
    "/lane：运行期连接管理 —— /lane（同 /ctl 列实例）· /lane attach <name>（连已就绪的实例）· /lane detach [<name>]（只断开本进程这条连接，名字缺省就是当前这条；声明与进程都不动）· /lane cancel <name>（取消它正在跑的回合，不必先切过去）· /lane new <name> [workspace] [profile=<name>]（新增声明并等它就绪；工作区缺省用当前 lane 的；profile 只按名字引用 profiles.toml）· /lane close <name>（把声明标成 enabled=false 并让 ctl 停掉实例，声明与手写配置都留着）· /lane open <name>（撤销 close）；连接条数有上限，超限时按提示 `/lane detach <name>` 摘掉不用的",
    "/mode ask|plan|act · /role <id> · /perm <level>（off 清除；有会话时点选即落盘） · /model <name>（只在本进程有效，退出即失效；Web 的 /model set 存在本机）",
    "/workspace <路径>（/cd 别名）切换 serve 工作区根；空参只看当前根；/workspace off 恢复 serve 默认根",
    "/editor [--md|--src] [<相对路径>]（/view 同义）把主区切成只读文件视图（右侧目录树按 Enter 也能打开）；`.md` / `.markdown` 默认按 **markdown 预览**渲染（不画行号、按宽度折行，连续空行折叠），`--src` 强制源码视图、`--md` 强制预览（任意文件都行）；PgUp/PgDn 滚一屏、Alt+↑/↓ 滚一行、鼠标滚轮滚 3 行、Ctrl+Home/End 跳首尾；Ctrl+F 文件内查找（Enter/Shift+Enter 跳下/上一处、Esc 只关查找，只搜**这个文件** —— 与聊天区的 `/find` 是两件事）；Ctrl+Y 复制当前文件的相对路径；只读，不做保存",
    "/chat 从编辑器视图切回聊天（Ctrl+T 双向切换、输入框为空时 Esc 同义）；切视图不打断正在跑的回合；状态栏 view 段显示当前在哪，点它也能切",
    "/settings（或 F6）：设置面板（模型名/API Base/温度/思考模式/API 密钥/Agent role/会话模式；F2 保存）",
    "/find <词> 搜索并高亮聊天记录 · /find（空参）跳下一处 · /find off 清除（文件视图里的查找是 Ctrl+F，两者互不影响）",
    "/background <图片路径>：全屏背景（kitty 走真像素，其余终端用每格底色；图会压暗保正文可读；off 关闭）",
    "/conv list 刷新 · /conv use <id> 切换 · /conv new 新会话 · /conv export [json|markdown|both] [--out <路径>] [--id <id>] 导出整段会话（与 Desktop 同形的 md/json；不给 --out 落到当前目录的 chat_export_<时间戳>，可导别的会话；md 同 markdown，--out 不建父目录） · /clear 不是 TUI 命令：要丢上下文用 /conv new（历史仍在 serve，切会话会重放回来）",
    "/resume 续传上一轮被中断的回合（serve 侧 job 仍在跑时；Ctrl+C 的 cancel 未送达也可续）",
    "/fold [n|all|close]：切换执行过程的展开态（每轮结束自动折叠；/older 回放与回看同样折叠）—— 空参=最近一条，n 从底部数，all 全展开，close 全折回；折叠导航开着时光标会跟到切过的那条",
    "/older（/history 同义）把更早一页历史前插到当前对话（切换会话只载入最新一页时用；滚到顶再按 PgUp 会自动续翻；跨页命中仍可 /find）",
    "/version 本进程版本（不请求 serve） · /skills 列出 serve 侧 skill（读 / 菜单同一份缓存，不重新拉） · /context 上下文用量明细（与状态行 ctx 段同口径） · /config reload 让 serve 重读磁盘配置 · /agent list 列出可用 agent role（设置仍用 /role <id>） · /export 导出当前会话（等价 /conv export，同一产物）",
];

/// 判定散在别的模块、没有热键条目的键（快捷键分组之外的那些）。
const MISC_KEYS_LINE: &str = "其它按键：Ctrl+G 折叠导航（任何焦点都能进；↑↓ 选摘要 · Enter 开合 · Esc 退出 · 再按一次回到最近一条） · PgUp/PgDn 翻页 · Ctrl+End 回底部 · 回合中 Enter 排队发送（输入框内 Esc 清队）";

/// 分区域操作（会话栏 / 连接 / 工作区 / 审批浮层）。
const TAIL_LINES: &[&str] = &[
    "会话栏：↑↓选 Enter用 n新建 r刷新 Del删除（再按 y/Enter 确认，Esc/n 或其它键取消）Shift+Del 直接删",
    "连接：左栏下半列出本进程的全部 serve 连接（单条也显示，> 为当前）· 切换见上面「全局」 · 点一行也可切",
    "工作区树：↑↓选 Enter/→展开 ◀收起/回父 r刷新 w回会话列表 Tab/Esc 回输入",
    "工作区切换：按 p 从项目池选择/切换（未设置=选择，已设置=切到另一项目；↑↓选 Enter切换 r刷新 Esc 返回）",
    "审批浮层：Enter=一次 · a=始终 · Esc/n=拒绝 · Ctrl+C 先拒绝、回合随后继续需再按取消",
];

impl UiState {
    /// `/help`：命令 → 快捷键（按键位表生成）→ 分区域操作，逐行推进 transcript。
    ///
    /// 与文案表同处一文件：文案随功能增删时只改这里，`mod.rs` 只需一行分发。
    /// 从 `mod.rs` 搬过来是为了把它从 920 行文件门禁线差几行的位置退开。
    pub(super) fn show_help(&mut self) {
        for line in COMMAND_LINES {
            self.push_line(LineKind::System, line);
        }
        // 四行由 `hotkeys` 的键位表生成：新增顶层热键时这里与设置面板一起自动跟上，
        // 不会再出现「面板写了、/help 漏了」这种手写字符串的漂移（#433）。
        for group in HotkeyGroup::ALL {
            let line = format!(
                "快捷键（{}）：{}",
                group.label(),
                hotkeys::group_line(group)
            );
            self.push_line(LineKind::System, &line);
        }
        self.push_line(LineKind::System, MISC_KEYS_LINE);
        for line in TAIL_LINES {
            self.push_line(LineKind::System, line);
        }
    }
}

#[cfg(test)]
mod tests {
    use crabmate_client_api::slash_caps::{Surface, supported};

    use super::COMMAND_LINES;

    /// `/help` 的命令清单必须覆盖本端**每一个**能敲的共享 head。
    ///
    /// 这条夹逼以前只有 Web 那边（`help_text_lists_every_intercepted_head`）：`/version`
    /// 曾经「能敲但 help 不写」，查起来像命令坏了。TUI 的清单是手写字面量，同样会漂 ——
    /// 加 head 时忘了补文案，这里就红。
    #[test]
    fn help_lists_every_tui_head() {
        let all = COMMAND_LINES.join("\n");
        for head in supported(Surface::Tui) {
            let needle = format!("/{}", head.as_str());
            assert!(all.contains(&needle), "/help 命令清单漏了 {needle}");
        }
    }
}
