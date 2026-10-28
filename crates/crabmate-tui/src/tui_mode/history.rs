//! 历史回放：`GET /conversation/messages` → transcript 行。
//!
//! 单独成模块而不是写进 `mod.rs`：那里已贴着 920 行门禁（同 `body_memo.rs` 的理由）。
//!
//! 回放是**只读**的：不写会话索引、不做 revision 合并、不产生任何副作用。
//! Desktop 的「切换会话 = 换一份 transcript」在这里同样成立，但**先不清屏**：
//! 屏幕只在拿到内容后才被替换，请求期间保留原内容加一行进度提示。这样请求失败时
//! 用户（连同绑定回滚）还在原来的位置，而不是对着一块空白。
//!
//! 回包是异步的，所以「这次回包还该不该落到屏幕上」必须显式判断（[`should_apply_reply`]）：
//! 等它的这段时间里用户可能已经切走、新建会话或直接开聊。

use crabmate_client_api::markdown_normalize::balance_unclosed_fence;
use crabmate_client_api::prompt_tokens::TiktokenPromptTokensSnapshot;
use crabmate_client_api::tool_display::is_placeholder_summary;
use crabmate_client_api::{ConversationMessagesResponse, ReplayRow};
use crabmate_tool_card::parse_tool_envelope;
use unicode_width::UnicodeWidthChar;

use super::state::{HistoryPage, HistoryRequest, LineKind};
use super::tool_summary::{flatten_note, tool_end_text};
use super::{LaneApp, WorkerJob};

/// 历史工具行正文只留首行的前若干**显示列宽**。
///
/// 服务端存的是完整输出，一次 `read_file` 就可能上百行；实时路径本来也只显示一行摘要。
/// 按列宽而不是字符数截断：96 个汉字约 192 列，折行之后就不再是「一行摘要」了。
const TOOL_HEAD_WIDTH: usize = 96;

/// 请求回放：记下在途状态 → 绑定会话 → 清空 transcript → 发 worker job。
pub(super) fn replay_into(app: &mut LaneApp<'_>, conversation_id: String, title: Option<&str>) {
    let label = title.unwrap_or("(by id)").to_string();
    // 先记 pending 再绑定：回包到达时要靠它判断「是不是我这次请求」。
    app.st.history_pending = Some(HistoryRequest {
        conversation_id: conversation_id.clone(),
        prev_conversation_id: app.st.conversation_id.clone(),
        title: label.clone(),
    });
    app.st.conversation_id = Some(conversation_id.clone());
    // 旧会话的用量不能留在状态栏上等新快照覆盖（切换失败时它还会一直在）。
    app.st.clear_usage();
    // 先不清屏：拿到内容后才在 `apply_page` 里替换。请求失败时原来的对话还在。
    app.st
        .push_line(LineKind::System, &format!("正在载入会话「{label}」的历史…"));
    let sent = app
        .job_tx
        .send(WorkerJob::ConversationHistory {
            conversation_id,
            before_index: None,
        })
        .is_ok();
    if !sent {
        app.st.history_pending = None;
        app.st
            .push_line(LineKind::System, "载入历史失败：worker 已退出");
    }
}

/// 回灌一页历史；过期回包直接丢弃。
pub(super) fn on_result(
    app: &mut LaneApp<'_>,
    conversation_id: &str,
    before_index: Option<u32>,
    result: Result<ConversationMessagesResponse, String>,
    snapshot: Option<TiktokenPromptTokensSnapshot>,
) {
    // 前插页与「切换会话回放」是两条独立的落地路径：前者不清屏、不动绑定，
    // 只认自己那个在途标记（`history_page.loading`）。
    if before_index.is_some() {
        on_older_result(app, conversation_id, result);
        return;
    }
    let Some(pending) = app.st.history_pending.as_ref() else {
        return;
    };
    let pending_matches = pending.conversation_id == conversation_id;
    let applies = should_apply_reply(
        app.st.running,
        pending_matches,
        app.st.conversation_id.as_deref() == Some(conversation_id),
    );
    if !applies {
        // 只清「就是这次请求」的 pending：否则会把一个更新的在途请求一起清掉。
        if pending_matches {
            app.st.history_pending = None;
        }
        return;
    }
    let Some(pending) = app.st.history_pending.take() else {
        return;
    };
    // 用量快照只在这一支应用：回包被判为「不该生效」时（切会话失败 / 回合在途）跳过，
    // 与 Web 端一致 —— 否则会把上一次会话的水位记到当前会话上。
    if let Some(snap) = snapshot {
        app.st.set_prompt_tokens(snap);
    }
    match result {
        Ok(resp) => apply_page(app, &pending, &resp),
        Err(e) => match fail_fallback(pending.prev_conversation_id.as_deref()) {
            ReplayFailFallback::RestorePrevious => {
                // 先换回**原会话**的草稿，再把绑定回滚：`swap_draft_in` 用 `st.conversation_id`
                // 作存回的键，顺序反过来会把目标会话的草稿写进原会话的槽。
                app.swap_draft_in(pending.prev_conversation_id.as_deref());
                app.st.conversation_id = pending.prev_conversation_id.clone();
                app.st.push_line(
                    LineKind::System,
                    &format!(
                        "载入会话「{}」的历史失败：{e}（已保持原会话，可直接输入）",
                        pending.title
                    ),
                );
            }
            ReplayFailFallback::StartNewSession => {
                // 没有原会话可回（典型：刚删掉当前会话，删除路径先把绑定清空了）。
                // 就此空着的话，下一条消息会去开一个连索引都不知道的 `conversation_id`
                // —— 左栏、Web、Desktop 都看不见它。所以直接开一条新会话顶上。
                //
                // 输入框里此刻是**目标会话**的草稿（切走时换进来的），不能让它变成新会话的
                // 开场白：先按目标 id 存回、切到「未绑定」槽，再清绑定。
                app.swap_draft_in(None);
                app.st.conversation_id = None;
                app.st.push_line(
                    LineKind::System,
                    &format!(
                        "载入会话「{}」的历史失败：{e}（无原会话可回，正在开新会话）",
                        pending.title
                    ),
                );
                app.new_session();
            }
        },
    }
}

/// `/older` 的门控结果（纯函数便于单测：`load_older` 本身绑着 worker 通道）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OlderGate {
    Enqueue {
        conversation_id: String,
        before_index: u32,
    },
    Refuse(&'static str),
}

/// 能不能往前翻一页；不能时给出**为什么**，而不是静默无事发生。
fn older_gate(
    running: bool,
    replay_pending: bool,
    conversation_id: Option<&str>,
    page: Option<&HistoryPage>,
) -> OlderGate {
    if running {
        return OlderGate::Refuse("回合进行中：等这一轮结束（或 Ctrl+C 取消）后再翻历史");
    }
    // 切换会话的回放还在途：`replay_into` 已经改绑 `conversation_id`，但 `history_page`
    // 仍是**上一个会话**的游标。放行就等于拿旧游标去请求新会话，回包还能通过下面
    // `on_older_result` 的 `same_session` 门控 —— 前插进来的那页属于别的会话。
    if replay_pending {
        return OlderGate::Refuse("正在载入会话历史，稍后再翻更早的消息");
    }
    let Some(cid) = conversation_id.map(str::trim).filter(|c| !c.is_empty()) else {
        return OlderGate::Refuse(
            "当前没有可翻的会话历史（先聊一句让它落盘，或用 /conv use 打开一个会话）",
        );
    };
    let Some(page) = page else {
        return OlderGate::Refuse("当前没有分页游标（先 /conv use 打开已落盘的会话）");
    };
    if page.loading {
        return OlderGate::Refuse("正在加载更早的消息…");
    }
    if !page.has_older {
        return OlderGate::Refuse("已经是第一条消息了（没有更早的历史）");
    }
    OlderGate::Enqueue {
        conversation_id: cid.to_string(),
        before_index: page.window_start,
    }
}

/// `/older`：把游标之前的一页历史拉回来并前插到 transcript 头部。
pub(super) fn load_older(app: &mut LaneApp<'_>) {
    match gate_older(app) {
        OlderGate::Enqueue {
            conversation_id,
            before_index,
        } => enqueue_older(app, conversation_id, before_index, false),
        OlderGate::Refuse(msg) => app.st.push_line(LineKind::System, msg),
    }
}

/// 滚到顶继续上翻（PgUp）自动续翻：与 `/older` 同一条路，但**拒绝时静默** ——
/// 在顶连按 PgUp 是高频动作，每按一次就往 transcript 写一行提示只会变成噪音。
/// 发请求失败同样静默：worker 已断时连按会刷出一屏一样的错误。
pub(super) fn load_older_on_top(app: &mut LaneApp<'_>) {
    if let OlderGate::Enqueue {
        conversation_id,
        before_index,
    } = gate_older(app)
    {
        enqueue_older(app, conversation_id, before_index, true);
    }
}

/// 门控求值：`/older` 与「到顶续翻」必须共用同一组理由，否则两条路径会各自漂移。
fn gate_older(app: &LaneApp<'_>) -> OlderGate {
    older_gate(
        app.st.running,
        app.st.history_pending.is_some(),
        app.st.conversation_id.as_deref(),
        app.st.history_page.as_ref(),
    )
}

/// 真正发出翻页请求；`worker` 已退出时回滚在途标记，并按 `quiet` 决定是否说明原因。
fn enqueue_older(app: &mut LaneApp<'_>, conversation_id: String, before_index: u32, quiet: bool) {
    app.st.set_older_loading(true);
    let sent = app
        .job_tx
        .send(WorkerJob::ConversationHistory {
            conversation_id,
            before_index: Some(before_index),
        })
        .is_ok();
    if !sent {
        app.st.set_older_loading(false);
        if !quiet {
            app.st
                .push_line(LineKind::System, "加载更早的消息失败：worker 已退出");
        }
    }
}

/// 前插页回包：只在「仍停在同一会话、没有回合在跑、确实有一次翻页在途」时落地。
///
/// 其余情形（切走了 / 已开聊 / `/conv new`）把回包丢掉 —— 与 [`should_apply_reply`]
/// 对切换回放的门控同源，只是这里认的是另一份在途标记。
fn on_older_result(
    app: &mut LaneApp<'_>,
    conversation_id: &str,
    result: Result<ConversationMessagesResponse, String>,
) {
    let waiting = app.st.history_page.is_some_and(|page| page.loading);
    let same_session = app.st.conversation_id.as_deref() == Some(conversation_id);
    if !waiting || !same_session || app.st.running {
        app.st.set_older_loading(false);
        return;
    }
    match result {
        Ok(resp) => {
            let rows = rows_to_lines(&resp);
            let n = rows.len();
            app.st.prepend_replayed(rows);
            // 空页还带 `has_older: true` 是服务端异常态：收敛成「没有更早」，否则
            // `/older` 会反复请求同一页、每次前插 0 行。
            app.st
                .set_history_page(resp.window_start_index, resp.has_older && n > 0);
            let tail = if app.st.has_older_pages() {
                "（仍有更早：继续 /older）"
            } else {
                "（已到最早一条）"
            };
            app.st
                .push_line(LineKind::System, &format!("已加载更早的 {n} 条消息{tail}"));
        }
        Err(e) => {
            app.st.set_older_loading(false);
            app.st
                .push_line(LineKind::System, &format!("加载更早的消息失败：{e}"));
        }
    }
}

/// 回放失败后往哪儿落。
///
/// 决定了「失败提示怎么说」和「要不要顺势开一条新会话」，抽成纯函数便于单测：
/// [`on_result`] 本身绑着 worker 通道，测不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayFailFallback {
    /// 回到 `prev_conversation_id`：屏幕上原来那段对话还在，位置不动。
    RestorePrevious,
    /// 没有原会话可回：必须开一条新会话，否则下一条消息没有索引行可绑。
    StartNewSession,
}

fn fail_fallback(prev: Option<&str>) -> ReplayFailFallback {
    match prev {
        Some(_) => ReplayFailFallback::RestorePrevious,
        None => ReplayFailFallback::StartNewSession,
    }
}

/// 回包是否还能落到屏幕上。
///
/// 四种情况必须丢弃：
/// - 没有配套的在途请求（`replay_into` 之外来的，或已被用户的动作清掉）；
/// - 不是这次请求（连切两次会话，先发的那个回包晚到）；
/// - 当前绑定已经不是它（用户切走 / `/conv new`）；
/// - 回合在跑——屏幕上已经是新的对话内容，清屏就是丢用户的东西。
fn should_apply_reply(running: bool, pending_matches: bool, current_matches: bool) -> bool {
    !running && pending_matches && current_matches
}

/// 把一页历史写进 transcript。
fn apply_page(
    app: &mut LaneApp<'_>,
    pending: &HistoryRequest,
    resp: &ConversationMessagesResponse,
) {
    let rows = rows_to_lines(resp);
    app.st.reset_transcript();
    // 游标必须在 `reset_transcript` 之后写：它会连 `history_page` 一起清掉。
    app.st
        .set_history_page(resp.window_start_index, resp.has_older);
    if rows.is_empty() {
        // 空会话（刚建、还没发过消息）：说清可以直接输入，别报「载入了 0 行」。
        app.st.push_line(
            LineKind::System,
            &format!("会话「{}」暂无历史消息（可直接输入）", pending.title),
        );
        return;
    }
    for (kind, text, tool_ok) in rows {
        // `tool_ok` 来自信封的 `ok`：它决定正文颜色（成功绿 / 失败红），不回传就只剩
        // 文案里的 `✓/✗`，与现场的颜色对不上。纯文本结果没有信号，仍是 `None`。
        // 历史行一律记为「已结束」（`push_replayed_line` 内部就是这么落的），
        // 所以不会被画成「进行中」。
        app.st.push_replayed_line(kind, &text, tool_ok);
    }
    app.st
        .push_line(LineKind::System, &summary_line(pending, resp));
}

/// 载入完成后的汇总行。
///
/// 明说「已续接」：启动时自动回放会绑定该会话，之后输入的消息续进它而不是新开会话
/// —— 这一点必须让用户看得见，否则会以为自己在一个新对话里。
fn summary_line(pending: &HistoryRequest, resp: &ConversationMessagesResponse) -> String {
    let mut facts = vec![format!("{} 条消息", resp.total_count)];
    // 服务端没给 revision 时是 0：别打出「revision 0」，那看起来像 bug。
    if resp.revision > 0 {
        facts.push(format!("revision {}", resp.revision));
    }
    if resp.has_older {
        facts.push("更早的消息用 /older 加载".to_string());
    }
    format!(
        "已续接会话「{}」（{}；要开新会话用 `/conv new`）",
        pending.title,
        facts.join("，")
    )
}

/// 启动后第一次拿到会话列表：把「上次在用的会话」回放出来。
///
/// 只尝试一次：否则每次 `r` 刷新左栏都会重放并冲掉当前 transcript。之后只有用户的
/// 显式动作（左栏 Enter / `/conv use`）才触发回放。
pub(super) fn auto_replay_if_active(app: &mut LaneApp<'_>) {
    if app.index.history_auto_replayed {
        return;
    }
    app.index.history_auto_replayed = true;
    // 屏幕上已经有内容（索引首次拉取失败、用户先聊起来了，之后刷新才成功）：
    // 此时回放会把正在进行的对话冲掉，宁可不回放。
    if !app.st.lines.is_empty() {
        return;
    }
    // CLI `--conversation-id` 显式指定（启动时 transcript 必为空）→ 回放它。
    if let Some(existing) = app.st.conversation_id.clone() {
        replay_into(app, existing, None);
        return;
    }
    let Some(active_id) = app.st.active_session_id.clone() else {
        return;
    };
    let Some(pos) = app.st.sessions.iter().position(|r| r.id == active_id) else {
        return;
    };
    app.st.selected = pos;
    let Some(resume) = app.st.selected_resume() else {
        return;
    };
    let title = app.st.sessions[pos].title.trim().to_string();
    let title = (!title.is_empty()).then_some(title);
    replay_into(app, resume, title.as_deref());
}

/// 响应 → (行类型, 文本, 工具成败)。抽成纯函数便于单测：`apply_page` 剩下的只是作用到 `UiState`。
/// 第三项只对工具行有值，其余恒 `None`。
///
/// 助手正文先过 [`balance_unclosed_fence`]：回放是**终态**，一条消息里多出的孤儿开符
/// （模型写到一半的 ``` / 服务端展示合并留下的残片）会让 TUI 把余下正文全部涂成代码块
/// （`md::assistant_styled_text` 的 `in_fence` 进去就不出来）。流式期间不能这么做 ——
/// 那时停在未闭合围栏是正常状态，提前「修正」会让代码块在闭合瞬间跳变。
fn rows_to_lines(resp: &ConversationMessagesResponse) -> Vec<(LineKind, String, Option<bool>)> {
    resp.replay_rows()
        .into_iter()
        .map(|row| match row {
            ReplayRow::User(t) => (LineKind::User, t, None),
            ReplayRow::Thinking(t) => (LineKind::Thinking, t, None),
            ReplayRow::Assistant(t) => (LineKind::Assistant, balance_unclosed_fence(&t), None),
            ReplayRow::Tool { name, body } => {
                let (text, ok) = tool_history_row(&name, &body);
                (LineKind::Tool, text, ok)
            }
        })
        .collect()
}

/// 历史工具行：`(文案, 成败)`。
///
/// `role=tool` 落盘的正文有两种形态：纯文本结果，或 `{"crabmate_tool":{…}}` 信封。
/// 后者必须解包——实时路径的 `note` 取的是 serve 的 `metadata.summary`，整段 JSON
/// 直接印出来会让回放和现场差一整条街。信封里的 `ok` 同时决定文案的 `✓`/`✗`
/// **和正文颜色**，所以要一起返回；纯文本结果没有信号 → `None`（不猜成败）。
fn tool_history_row(name: &str, body: &str) -> (String, Option<bool>) {
    if let Some(input) = parse_tool_envelope(body, Some(name)) {
        let note = tool_note(&input.name, input.summary.as_deref(), &input.output);
        return (
            tool_end_text(&input.name, input.ok, note.as_deref()),
            input.ok,
        );
    }
    let note = tool_note(name, None, body);
    (tool_end_text(name, None, note.as_deref()), None)
}

/// 摘要：`summary` 优先，其次结果正文首行；先压成单行，再按显示宽截断。
///
/// 压平必须先做：`summary` 可能是「工具名 \n 工具名: 详情」，只取首行会留下一句纯
/// 工具名（信息全丢），不压平则整段挤进一行、名字出现两次。
fn tool_note(name: &str, summary: Option<&str>, body: &str) -> Option<String> {
    // 占位摘要（Server 摘要推导失败时的 `tool: <name>`）没有信息量，还会把工具行
    // 退回「只有工具名」：丢掉它，宁可回退到结果正文首行。
    let raw = summary
        .filter(|s| !s.trim().is_empty() && !is_placeholder_summary(name, s))
        .or_else(|| body.lines().next())?;
    let flat = flatten_note(name, raw)?;
    Some(head_excerpt(&flat))
}

/// 首行按**显示列宽**截断（宽字符按 2 列算）。
fn head_excerpt(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in line.trim().chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > TOOL_HEAD_WIDTH {
            break;
        }
        width += w;
        out.push(ch);
    }
    out
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
