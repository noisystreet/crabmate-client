//! 全屏 TUI 的本地控制斜杠解析与 override 回显文本。
//!
//! 拆到独立模块以压住 `mod.rs` 行数/CCN 门禁；`apply_control` 仍在事件循环层。

use super::lane_cmd;
use super::settings::SESSION_MODES;
use crabmate_client_api::slash_heads::{Head, head_of};

/// 控制斜杠（本地处理，不发给模型）。`/` 开头的未知命令视为普通消息。
pub enum Control {
    Quit,
    Help,
    Model(Option<String>),
    Mode(Option<String>),
    Role(Option<String>),
    /// `/perm [<level>]`：本轮权限等级意图（`off` 清除）。
    Perm(Option<String>),
    Status,
    /// `/settings`：打开设置面板（等同 F2）。
    Settings,
    Find(Option<String>),
    /// `/workspace [路径]`（`/cd` 别名）：空参查当前根，带参按路径切换。
    Workspace(Option<String>),
    /// `/resume`：续传上一轮被中断的回合（`stream_resume:{job_id,after_seq}`）。
    Resume,
    /// `/older`（别名 `/history`）：把更早一页历史前插到 transcript 头部。
    Older,
    /// `/fold [n|all|close]`：切换执行过程的展开态（issue #369 / PR370）。
    ///
    /// 折叠是自动的（回合结束 / 回放插入后），所以这里只需要「反悔」那一个方向 ——
    /// 与 `/older` 相反：那个往回拿更多内容，这个把折起来的内容放出来。摘要行始终保留，
    /// 因此同一个命令既能展开也能折回；`n` 从底部数（1 起），`all` 全展开，`close` 全折回。
    Fold(Option<String>),
    ConvNew,
    ConvRefresh,
    ConvUse(String),
    /// `/conv export [json|markdown|both] [--out <路径>] [--id <id>]`。
    ///
    /// `Err` 是**给用户看**的用法错误（未知格式 / 缺 flag 值），与 `Control::Lane(Err(_))`
    /// 同一条约定：命令已经从输入框消失，解析失败也必须留下痕迹。
    ConvExport(Result<crate::conv_export::ExportCmd, String>),
    ConvUnknown(String),
    /// `/background [图片路径]`：全屏背景（kitty 走像素，其余用每格底色；空参查状态，`off` 关闭）。
    Background(Option<String>),
    /// `/copy [assistant|user|path]`（`/yank` 同义）：把内容或路径送进剪贴板。
    ///
    /// 默认复制最近一条助手回复；`user` 复制自己的上一条；`path` 复制 `/editor` 视图里那个
    /// 文件的**相对路径**（视图开着时不写目标也走这条 —— 那时屏幕上只有文件）。
    /// 投递走终端剪贴板（OSC52）或 `wl-copy` / `xclip` / `xsel`，见 [`super::clipboard`]。
    Copy(Option<String>),
    /// `/ctl`：查 `crabmatectl` 控制面的实例列表（本地 unix socket，**只读**）。
    ///
    /// 只是查询；新增连接走 [`Control::Lane`]（Server 已补上 `disable`/`enable` 与
    /// `add` 的字段面，所以运行期加 lane 不再被阻塞）。
    Ctl,
    /// `/lane [list|attach <name>|detach [<name>]|new <name> [workspace]]`：运行期连接管理。
    ///
    /// `detach` 只摘**本进程**的连接：声明与 serve 进程都不动（与 `attach` 对称），所以
    /// 它不经 ctl，也不进 worker。名字缺省 = **当前 lane**（不必先 `/lane` 查自己叫什么）。
    ///
    /// `Err` = 参数写错的用法提示：解析在 [`super::lane_cmd::parse`]，但报错文本要
    /// 写进 transcript，所以只在事件循环里打印。
    Lane(Result<lane_cmd::LaneCommand, String>),
    /// `/editor [<相对路径>]`：进入只读编辑器视图（issue #418）。
    ///
    /// 无参 = 进入视图并沿用上次打开的文件；带参数 = 直接打开该文件。只读：不做保存与编辑，
    /// 写回涉及变更集与冲突语义，属 Desktop IDE 的范围。
    Editor(Option<String>),
    /// `/chat`：从编辑器视图切回聊天（幂等）。
    ///
    /// 已经在聊天视图时只写一行说明 —— 命令成功过一次之后再执行它是无害的，
    /// 报错会让人怀疑自己是不是切错了地方。
    Chat,
    /// 信息类命令（issue #444）：`/version` `/skills` `/context` `/config` `/agent`。
    ///
    /// 五条共用**一个**变体：它们都是「读已有状态、写几行」的动作，处理函数整体在
    /// `info_cmd` 里。若各占一个变体，`apply_control` 要多五行（`mod.rs` 有 920 行门禁），
    /// 而事件循环那层其实只关心「这条要交给 info 处理」。
    Info(InfoCmd),
}

/// [`Control::Info`] 的载荷（issue #444）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoCmd {
    /// `/version`：本进程版本号（编译期常量，**不请求 serve**）。
    ///
    /// 与 `/status` 分工：那个回答「serve 是什么」，这个回答「我是什么版本」。
    Version,
    /// `/skills`：列出 serve 侧已发现的 skill（id + 描述）。
    ///
    /// 数据就是 `/` 补全菜单那份 `SkillsCache`：菜单已经拉过就直接印，
    /// **不**为了一条命令再打一次 `GET /skills`。
    Skills,
    /// `/context`：上下文用量明细（与状态行 `ctx` 段、Desktop 用量页同口径）。
    Context,
    /// `/config reload`：让 serve 重读磁盘配置（`POST /config/reload`）。
    /// 空参打用法 —— `/config` 猜不出用户想要什么，静默不做更糟。
    Config(Option<String>),
    /// `/agent [list]`：列可用 agent role（数据来自 `/status` 的 `agent_role_ids`）。
    ///
    /// 只做**只读列举**；设置走 `/role`，`set <id>` 这类写入口留在 Web。
    Agent(Option<String>),
}

pub fn clear_word(arg: &str) -> bool {
    matches!(arg, "off" | "none" | "clear")
}

/// 设置/查询一个字符串型 override 槽，返回给用户看的回显。
pub fn set_override_field(slot: &mut Option<String>, arg: Option<String>, name: &str) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            format!("{name} override cleared")
        }
        Some(v) => {
            *slot = Some(v.clone());
            format!("{name} override: {v}")
        }
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("{name} override: {v}"),
            _ => format!("{name} override: (none — 使用 serve 默认；/{name} <值> 设置)"),
        },
    }
}

/// 设置 `/mode` override（校验 ask/plan/act），返回给用户看的回显。
pub fn set_mode_field(slot: &mut Option<String>, arg: Option<String>) -> String {
    match arg {
        Some(v) if clear_word(&v) => {
            *slot = None;
            "mode override cleared".to_string()
        }
        Some(v) if SESSION_MODES.contains(&v.as_str()) => {
            *slot = Some(v.clone());
            format!("mode override: {v}")
        }
        Some(v) => format!("invalid session mode '{v}'; 可选 ask / plan / act（off 清除）"),
        None => match slot.as_deref() {
            Some(v) if !v.trim().is_empty() => format!("mode override: {v}"),
            _ => "mode override: (serve 默认；/mode ask|plan|act 设置)".to_string(),
        },
    }
}

/// 解析控制斜杠：`/model [x]` `/mode [ask|plan|act]` `/role [id]` `/status`
/// `/find [词]` `/help` `/conv`(刷新) `/conv new` `/quit`（`/exit`、`/q` 同义）
/// `/workspace [路径]`（`/cd` 别名）`/settings` `/resume`。
/// 返回 `None` 表示按普通消息处理。
pub fn parse_control(text: &str) -> Option<Control> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?.trim();
    let (head, arg) = match rest.split_once(char::is_whitespace) {
        Some((h, a)) => (h.trim(), a.trim().to_string()),
        None => (rest, String::new()),
    };
    // 别名（`/q` `/h` `/cd` `/yank` …）由 `head_of` 归一，这里只写**主命令**的臂：
    // `match` 是穷尽的，新增 `Head` 变体时编译器会在这里拦下缺的那一条。
    let head = head_of(head)?;
    let arg_opt = (!arg.is_empty()).then_some(arg);
    match head {
        Head::Quit => Some(Control::Quit),
        Head::Help => Some(Control::Help),
        Head::Model => Some(Control::Model(arg_opt)),
        Head::Mode => Some(Control::Mode(arg_opt)),
        Head::Role => Some(Control::Role(arg_opt)),
        Head::Perm => Some(Control::Perm(arg_opt)),
        Head::Status => Some(Control::Status),
        Head::Settings => Some(Control::Settings),
        Head::Workspace => Some(Control::Workspace(arg_opt)),
        Head::Resume => Some(Control::Resume),
        Head::Older => Some(Control::Older),
        Head::Find => Some(Control::Find(arg_opt)),
        Head::Fold => Some(Control::Fold(arg_opt)),
        Head::Background => Some(Control::Background(arg_opt)),
        Head::Copy => Some(Control::Copy(arg_opt)),
        // `ctl` 只读查询控制面实例；`lane` 是它的超集（空参等价 `/ctl`，另加
        // `attach` / `new` 两个写入路径）。两者刻意不合并：`/ctl` 已在文档与 muscle
        // memory 里，改语义比多一个入口更容易出错。
        Head::Ctl => Some(Control::Ctl),
        Head::Lane => Some(Control::Lane(lane_cmd::parse(arg_opt.as_deref()))),
        // `/editor`（别名 `/view`）与 `/chat` 是一对：前者把主区切成只读文件视图，
        // 后者切回来。做成命令而不是纯快捷键，是因为「回到聊天」必须能被说出来 ——
        // 编辑器视图占的正是聊天区，快捷键只在提示行里有，命令还在 `/help` 与补全里。
        Head::Editor => Some(Control::Editor(arg_opt)),
        Head::Chat => Some(Control::Chat),
        Head::Conv => Some(parse_conv(arg_opt.as_deref())),
        // `/export` 落到与 `/conv export` **同一个** `Control`：产物、投影、落盘报告都复用，
        // 两条命令的差异只在入口。若给它单独一个 `Control`，两处渲染迟早走偏。
        Head::Export => Some(Control::ConvExport(crate::conv_export::parse_args(
            arg_opt.as_deref().unwrap_or(""),
        ))),
        Head::Version => Some(Control::Info(InfoCmd::Version)),
        Head::Skills => Some(Control::Info(InfoCmd::Skills)),
        Head::Context => Some(Control::Info(InfoCmd::Context)),
        Head::Config => Some(Control::Info(InfoCmd::Config(arg_opt))),
        Head::Agent => Some(Control::Info(InfoCmd::Agent(arg_opt))),
    }
}

/// `/conv <sub> …` 的分派。
///
/// 单独一层是因为 `parse_control` 的臂已经很多（CCN 门禁）：子命令每加一个都往那个 match
/// 里塞，迟早超 10。
fn parse_conv(arg: Option<&str>) -> Control {
    let Some(arg) = arg else {
        return Control::ConvRefresh;
    };
    match arg.split_whitespace().next().unwrap_or("") {
        "new" | "clear" => Control::ConvNew,
        "list" | "ls" | "show" => Control::ConvRefresh,
        "use" | "switch" => match arg.split_whitespace().nth(1) {
            Some(id) if !id.is_empty() => Control::ConvUse(id.to_string()),
            _ => Control::ConvUnknown(arg.to_string()),
        },
        "export" => Control::ConvExport(export_args(arg)),
        _ => Control::ConvUnknown(arg.to_string()),
    }
}

/// `export` 之后的那截参数交给 [`crate::conv_export::parse_args`]；只写 `/conv export`
/// 就是缺省格式（`json`，与 Web `/export` 同口径）。
fn export_args(arg: &str) -> Result<crate::conv_export::ExportCmd, String> {
    let rest = arg.trim().strip_prefix("export").unwrap_or("").trim();
    crate::conv_export::parse_args(rest)
}

/// `/conv` 未知子命令的提示。
///
/// 放在这里而不是 `mod.rs` 的 match 里：那里贴着 920 行门禁，一条多行 `format!` 就超标。
#[must_use]
pub fn conv_unknown_message(arg: &str) -> String {
    format!("不支持的 /conv 子命令：{arg}（支持 new / list / use <id> / export）")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// issue #444 的五条信息命令（`/export` 别名见下一条测试）。
    #[test]
    fn info_commands_parse() {
        let info = |line: &str| match parse_control(line) {
            Some(Control::Info(cmd)) => Some(cmd),
            _ => None,
        };
        assert_eq!(info("/version"), Some(InfoCmd::Version));
        assert_eq!(info("/skills"), Some(InfoCmd::Skills));
        // `ctx` 是状态行里的既有缩写，认它不必再解释。
        assert_eq!(info("/ctx"), Some(InfoCmd::Context));
        assert_eq!(
            info("/config reload"),
            Some(InfoCmd::Config(Some("reload".into())))
        );
        assert_eq!(info("/config"), Some(InfoCmd::Config(None)));
        assert_eq!(info("/agent"), Some(InfoCmd::Agent(None)));
        assert_eq!(
            info("/agent list"),
            Some(InfoCmd::Agent(Some("list".into())))
        );
    }

    /// `/export` 与 `/conv export` 必须**落到同一个 `Control`**：产物、投影、用法错误
    /// 都只有一份实现，两条入口的差别只在字面量。
    #[test]
    fn export_and_conv_export_share_one_parser() {
        for args in ["", "markdown", "--out /tmp/x.md", "--id c1", "bogus"] {
            let direct = parse_control(&format!("/export {args}"));
            let nested = parse_control(&format!("/conv export {args}"));
            match (direct, nested) {
                (Some(Control::ConvExport(a)), Some(Control::ConvExport(b))) => {
                    assert_eq!(a, b, "参数 `{args}` 在两条入口下解析结果不一致");
                }
                _ => panic!("参数 `{args}` 没有落到 Control::ConvExport"),
            }
        }
    }

    #[test]
    fn parse_control_known_slashes() {
        assert!(matches!(parse_control("/quit"), Some(Control::Quit)));
        assert!(matches!(parse_control("/EXIT"), Some(Control::Quit)));
        // `/exit`、`/q` 与 `/quit` 同义；带尾随空白同样接受。
        assert!(matches!(parse_control("/exit"), Some(Control::Quit)));
        assert!(matches!(parse_control(" /Exit  "), Some(Control::Quit)));
        assert!(matches!(parse_control("/q"), Some(Control::Quit)));
        assert!(matches!(parse_control("/status"), Some(Control::Status)));
        assert!(matches!(
            parse_control("/settings"),
            Some(Control::Settings)
        ));
        assert!(matches!(parse_control("/conv"), Some(Control::ConvRefresh)));
        assert!(matches!(parse_control("/conv new"), Some(Control::ConvNew)));
        assert!(matches!(
            parse_control("/model gpt-x"),
            Some(Control::Model(Some(v))) if v == "gpt-x"
        ));
        assert!(matches!(
            parse_control("/model"),
            Some(Control::Model(None))
        ));
        assert!(matches!(
            parse_control("/mode act"),
            Some(Control::Mode(Some(v))) if v == "act"
        ));
        assert!(matches!(
            parse_control("/role coder"),
            Some(Control::Role(Some(v))) if v == "coder"
        ));
        // `/perm` 与 `/mode` `/role` 同类：带参设意图、空参查询/清除。
        assert!(matches!(
            parse_control("/perm read"),
            Some(Control::Perm(Some(v))) if v == "read"
        ));
        assert!(matches!(parse_control("/perm"), Some(Control::Perm(None))));
    }

    #[test]
    fn parse_control_copy_targets() {
        // 空参默认助手回复；`yank` 是别名。
        assert!(matches!(parse_control("/copy"), Some(Control::Copy(None))));
        assert!(matches!(parse_control("/yank"), Some(Control::Copy(None))));
        assert!(matches!(
            parse_control("/copy user"),
            Some(Control::Copy(Some(v))) if v == "user"
        ));
    }

    /// `/conv export`：格式词只有 Web 那三种，其余走 `--out` / `--id`；未知格式给用法。
    #[test]
    fn parse_control_conv_export() {
        use crabmate_client_api::chat_export::ExportFormat;
        assert!(matches!(
            parse_control("/conv export"),
            Some(Control::ConvExport(Ok(cmd))) if cmd.format == ExportFormat::Json
        ));
        assert!(matches!(
            parse_control("/conv export markdown --out /tmp/a.md"),
            Some(Control::ConvExport(Ok(cmd)))
                if cmd.format == ExportFormat::Markdown
                    && cmd.out.as_deref() == Some("/tmp/a.md")
        ));
        assert!(matches!(
            parse_control("/conv export both --id c-1"),
            Some(Control::ConvExport(Ok(cmd)))
                if cmd.format == ExportFormat::Both && cmd.id.as_deref() == Some("c-1")
        ));
        // 解析失败不是「什么都不做」：`Err` 带着用法文案一路走到 transcript。
        assert!(matches!(
            parse_control("/conv export pdf"),
            Some(Control::ConvExport(Err(usage))) if usage.contains("用法")
        ));
        // 未知子命令仍然落到 `ConvUnknown`（别被 `export` 前缀吃掉）。
        assert!(matches!(
            parse_control("/conv exportt"),
            Some(Control::ConvUnknown(_))
        ));
    }

    /// `/lane` 的子命令解析与 `/ctl` 的分工：`/ctl` 仍只管只读查询。
    #[test]
    fn parse_control_lane_subcommands() {
        assert!(matches!(
            parse_control("/lane"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::List)))
        ));
        assert!(matches!(
            parse_control("/lane list"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::List)))
        ));
        assert!(matches!(
            parse_control("/lane attach proj-a"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Attach(n)))) if n == "proj-a"
        ));
        assert!(matches!(
            parse_control("/LANE new proj-b /data/proj-b"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::New { .. })))
        ));
        // `detach` 只摘本进程的连接；名字缺省摘当前 lane（`cancel` 仍必须点名）。
        assert!(matches!(
            parse_control("/lane detach proj-a"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Detach(Some(n))))) if n == "proj-a"
        ));
        assert!(matches!(
            parse_control("/lane detach"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Detach(None))))
        ));
        // `cancel` 取消一条 lane 的回合（后台审批没有它就只能等 600s 超时）。
        assert!(matches!(
            parse_control("/lane cancel proj-a"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Cancel(n)))) if n == "proj-a"
        ));
        assert!(
            matches!(parse_control("/lane cancel"), Some(Control::Lane(Err(_)))),
            "缺名字要给用法，不能静默变成 list"
        );
        // `close` / `open` 翻声明里的 `enabled`：软删与撤销（`disable` / `enable` 是别名）。
        assert!(matches!(
            parse_control("/lane close proj-a"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Close(n)))) if n == "proj-a"
        ));
        assert!(matches!(
            parse_control("/lane open proj-a"),
            Some(Control::Lane(Ok(lane_cmd::LaneCommand::Open(n)))) if n == "proj-a"
        ));
        // 认不出的子命令给用法提示，不当成普通消息发给模型。
        assert!(matches!(
            parse_control("/lane zzz proj-a"),
            Some(Control::Lane(Err(_)))
        ));
        // `/ctl` 没有被子命令语法改写：带参也照旧按只读查询处理。
        assert!(matches!(parse_control("/ctl"), Some(Control::Ctl)));
        assert!(matches!(parse_control("/ctl ls"), Some(Control::Ctl)));
    }

    #[test]
    fn parse_control_conv_use_and_unknown() {
        assert!(matches!(
            parse_control("/conv use c1"),
            Some(Control::ConvUse(v)) if v == "c1"
        ));
        assert!(matches!(
            parse_control("/conv bogus"),
            Some(Control::ConvUnknown(_))
        ));
        assert!(matches!(
            parse_control("/conv use"),
            Some(Control::ConvUnknown(_))
        ));
    }

    #[test]
    fn parse_control_find_and_help() {
        assert!(matches!(parse_control("/help"), Some(Control::Help)));
        assert!(matches!(parse_control("/h"), Some(Control::Help)));
        assert!(matches!(parse_control("/find"), Some(Control::Find(None))));
        assert!(matches!(
            parse_control("/search x y"),
            Some(Control::Find(Some(v))) if v == "x y"
        ));
        assert!(matches!(
            parse_control("/grep off"),
            Some(Control::Find(Some(v))) if v == "off"
        ));
    }

    #[test]
    fn parse_control_workspace_and_cd_alias() {
        assert!(matches!(
            parse_control("/workspace"),
            Some(Control::Workspace(None))
        ));
        assert!(matches!(
            parse_control("/workspace /data/proj"),
            Some(Control::Workspace(Some(v))) if v == "/data/proj"
        ));
        // 路径带空格：整体作为参数（`split_whitespace` 只切命令头）。
        assert!(matches!(
            parse_control("/cd /data/my proj"),
            Some(Control::Workspace(Some(v))) if v == "/data/my proj"
        ));
        // 大小写与首尾空白同样接受。
        assert!(matches!(
            parse_control("  /WorkSpace   /tmp/x  "),
            Some(Control::Workspace(Some(v))) if v == "/tmp/x"
        ));
    }

    #[test]
    fn parse_control_background() {
        // 空参 = 查询状态；带参 = 路径（含空格要整体保留）。
        assert!(matches!(
            parse_control("/background"),
            Some(Control::Background(None))
        ));
        assert!(matches!(
            parse_control("/bg /tmp/my bg.png"),
            Some(Control::Background(Some(v))) if v == "/tmp/my bg.png"
        ));
        // 清除词由上层 `clear_word` 判定，这里原样带出。
        assert!(matches!(
            parse_control("/BACKGROUND off"),
            Some(Control::Background(Some(v))) if v == "off"
        ));
        // 普通消息不被拦。
        assert!(parse_control("set the background please").is_none());
    }

    #[test]
    fn parse_control_passes_plain_through() {
        assert!(parse_control("hello").is_none());
        assert!(parse_control("/my-skill").is_none());
        assert!(parse_control(" /model ").is_some());
    }

    #[test]
    fn older_is_a_local_control() {
        assert!(matches!(parse_control("/older"), Some(Control::Older)));
        assert!(matches!(parse_control("/HISTORY"), Some(Control::Older)));
        assert!(matches!(parse_control("  /Older  "), Some(Control::Older)));
        // 不带 `/` 就是普通消息，不能拦。
        assert!(parse_control("older stuff").is_none());
    }

    #[test]
    fn resume_is_a_local_control() {
        assert!(matches!(parse_control("/resume"), Some(Control::Resume)));
        assert!(matches!(parse_control("/RESUME"), Some(Control::Resume)));
        assert!(matches!(
            parse_control("  /Resume  "),
            Some(Control::Resume)
        ));
        // 只有 `/` 开头才拦截：普通消息即使首词是 resume 也要发给模型。
        assert!(parse_control("resume the turn").is_none());
    }

    #[test]
    fn override_field_set_clear_query() {
        let mut slot = None;
        let echo = set_override_field(&mut slot, Some("gpt-x".into()), "model");
        assert!(echo.contains("gpt-x"));
        assert_eq!(slot.as_deref(), Some("gpt-x"));
        let echo = set_override_field(&mut slot, Some("off".into()), "model");
        assert!(echo.contains("cleared"));
        assert!(slot.is_none());
        let echo = set_override_field(&mut slot, None, "model");
        assert!(echo.contains("(none"));
    }

    #[test]
    fn mode_field_validates() {
        let mut slot = None;
        let bad = set_mode_field(&mut slot, Some("bogus".into()));
        assert!(bad.contains("invalid"));
        assert!(slot.is_none());
        let ok = set_mode_field(&mut slot, Some("plan".into()));
        assert!(ok.contains("plan"));
        assert_eq!(slot.as_deref(), Some("plan"));
    }
}
