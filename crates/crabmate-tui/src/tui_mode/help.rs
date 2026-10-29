//! `/help` 的文案表。
//!
//! 单独成文件是为了压住 `mod.rs` 的 920 行门禁：事件循环层只保留「逐行推进 transcript」，
//! 文案随功能增删时也只改这里。顺序即显示顺序，按「命令 → 按键 → 各分区操作」分段。

/// `/help` 输出的每一行。
pub(super) const LINES: &[&str] = &[
    "本地命令：/quit（/exit、/q 同义） /help /status /settings /mode /role /perm /model /find /background /workspace /resume /older /conv /copy",
    "输入 / 弹出补全菜单（内建命令 + skill；↑↓ 选择 · Tab/Enter 补全 · Esc 关闭）",
    "/copy [assistant|user]（Ctrl+Y 同义）：复制最近一条消息到剪贴板（默认助手回复）",
    "/mode ask|plan|act · /role <id> · /perm <level>（off 清除；有会话时点选即落盘） · /model <name>",
    "/workspace <路径>（/cd 别名）切换 serve 工作区根；空参只看当前根；/workspace off 恢复 serve 默认根",
    "/settings（或 F6）：设置面板（模型名/API Base/温度/思考模式/API 密钥/Agent role/会话模式；F2 保存）",
    "/find <词> 搜索并高亮 · /find（空参）跳下一处 · /find off 清除",
    "/background <图片路径>：全屏背景（kitty 走真像素，其余终端用每格底色；图会压暗保正文可读；off 关闭）",
    "/conv list 刷新 · /conv use <id> 切换 · /conv new 新会话",
    "/resume 续传上一轮被中断的回合（serve 侧 job 仍在跑时；Ctrl+C 的 cancel 未送达也可续）",
    "/older（/history 同义）把更早一页历史前插到当前对话（切换会话只载入最新一页时用；滚到顶再按 PgUp 会自动续翻；跨页命中仍可 /find）",
    "按键：Alt+Enter 换行 · Ctrl+E 思考展开/折叠 · Ctrl+Y 复制最近回复 · Ctrl+W 工作区目录树 · PgUp/PgDn 翻页 · Ctrl+End 回底部 · 回合中 Enter 排队发送（输入框内 Esc 清队）",
    "设置面板：↑↓ 移动 · Enter 编辑 · Tab 切分区 · F2 保存 · Esc 关闭（有改动 Esc 先确认）",
    "会话栏：↑↓选 Enter用 n新建 r刷新 Del删除（再按 y/Enter 确认，Esc/n 或其它键取消）Shift+Del 直接删",
    "连接：左栏下半列出本进程的全部 serve 连接（单条也显示，> 为当前）· Alt+←/→ 切换 · 点一行也可切",
    "工作区树：↑↓选 Enter/→展开 ◀收起/回父 r刷新 w回会话列表 Tab/Esc 回输入",
    "工作区切换：按 p 从项目池选择/切换（未设置=选择，已设置=切到另一项目；↑↓选 Enter切换 r刷新 Esc 返回）",
    "审批浮层：Enter=一次 · a=始终 · Esc/n=拒绝 · Ctrl+C 先拒绝、回合随后继续需再按取消",
];
