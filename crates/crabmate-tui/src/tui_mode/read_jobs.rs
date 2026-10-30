//! 读请求（只读 GET）的发射路径：直接丢给主程序的 tokio runtime，不经 worker 队列。
//!
//! 抽成独立模块有两个理由：一是把「哪些请求必须排队」和「哪些可以并发」分开写清楚，
//! 二是压住 `mod.rs` 的行数（fn-nloc 门禁 ≤920）。

use super::LaneApp;
use super::state::LineKind;
use super::worker::{UiEvent, WorkerJob};
use crate::serve::fetch_workspace_dir;

impl LaneApp<'_> {
    /// 拉取子目录列表。
    ///
    /// 有 tokio runtime 时直接 `spawn`（生产路径）：目录展开是纯读，没有串行需求，排进
    /// worker 只会让它在长任务后面干等 —— 回合的 SSE 能跑几分钟，期间队里的读请求一条都
    /// 发不出去，右栏就一直停在 `⋯`。
    ///
    /// 没有 runtime 时（单测）退回 worker 队列：那里没有并发回合，串行不会有可观察差异。
    ///
    /// 两条路径都带上发起时的 `ws_generation`：并发之后回包顺序与发起顺序无关，中途切走
    /// 工作区的那条结果必须被丢弃，否则会把旧工作区的目录写进新工作区的缓存。
    pub(super) fn fetch_ws_dir(&mut self, rel: String) -> bool {
        let generation = self.st.ws_generation;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // `into_owned`：spawn 出来的任务要求 `'static`，借来的那份活不到那时
            // （`ServeClient` 只含 `reqwest::Client` + 配置，克隆很轻）。
            let client = self.client.clone().into_owned();
            let tx = self.tx.clone();
            handle.spawn(async move {
                let result = fetch_workspace_dir(&client, Some(&rel))
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx.send(UiEvent::WorkspaceDir {
                    rel,
                    generation,
                    result,
                });
            });
            return true;
        }
        if self
            .job_tx
            .send(WorkerJob::WorkspaceDir {
                rel: rel.clone(),
                generation,
            })
            .is_err()
        {
            self.st
                .push_line(LineKind::System, "拉取目录失败：worker 已退出");
            self.st.ws_dir_failed(&rel);
            return false;
        }
        true
    }
}
