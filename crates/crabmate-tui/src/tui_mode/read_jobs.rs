//! 读请求（只读 GET）的发射路径：直接丢给主程序的 tokio runtime，不经 worker 队列。
//!
//! 抽成独立模块有两个理由：一是把「哪些请求必须排队」和「哪些可以并发」分开写清楚，
//! 二是压住 `mod.rs` 的行数（fn-nloc 门禁 ≤920）。

use super::LaneApp;
use super::state::LineKind;
use super::worker::{LaneTx, UiEvent, WorkerJob};
use crate::serve::{fetch_workspace_dir, fetch_workspace_file};

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
            let out = LaneTx::new(self.id, self.tx.clone());
            handle.spawn(async move {
                let result = fetch_workspace_dir(&client, Some(&rel))
                    .await
                    .map_err(|e| e.to_string());
                let _ = out.send(UiEvent::WorkspaceDir {
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

    /// 拉取文件内容（只读 GET，与目录展开同一条直发路径）。
    ///
    /// 返回 `false` = 没能发起：当前不在 tokio runtime 里（只有单测会出现，生产路径
    /// `run_tui` 恒在 runtime 内）。此时视图状态保持调用方设的 `loading`，测试可以据此
    /// 断言「发起了、还没回包」，而不是看到一个假成功。
    ///
    /// 回包带发起时的 `generation`：中途切了工作区就把旧内容丢掉（与目录缓存同一口径，
    /// 否则会把 A 工作区的文件画在 B 的标题下）。
    pub(super) fn fetch_ws_file(&mut self, rel: String) -> bool {
        let generation = self.st.ws_generation;
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let client = self.client.clone().into_owned();
        let out = LaneTx::new(self.id, self.tx.clone());
        handle.spawn(async move {
            let result = fetch_workspace_file(&client, &rel)
                .await
                .map_err(|e| e.to_string());
            let _ = out.send(UiEvent::WorkspaceFile {
                rel,
                generation,
                result,
            });
        });
        true
    }
}
