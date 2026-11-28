//! 逐域**纯规则层**（#598）。
//!
//! 只依赖数据类型本身（`crate::storage` 的 `ChatSession` / `StoredMessage` 等），
//! **不碰** leptos 信号 / `web_sys` / `js_sys` / IO。因此可以直接在 host target
//! `cargo test` —— `make test-frontend` 跑的就是 `cd frontend && cargo test --lib`。
//!
//! # 为什么要这一层
//!
//! 这些规则原先写在 `crate::session_ops` 里，与 `window.alert` / 下载 / 剪贴板等 DOM
//! 副作用同处一个 848 行的文件，于是规则本身只能靠整页 e2e（63 文件 / 13.6k 行）兜。
//! 搬到本层后每条规则都有了便宜的 host 单测。
//!
//! # 边界（勿破）
//!
//! - **副作用留在调用侧**：DOM、日志、剪贴板、下载一律不进本层；
//! - **异步时序不进本层**：水合 nonce 门闩、`spawn_local` 快照属于接线职责；
//! - **时间 / 随机源由调用方注入**：本层不读时钟、不生成 id —— 否则又变成不可测；
//! - 各端调用点**不用改**：`crate::session_ops` 对**签名未变**的规则用 `pub use` 转发；对改为
//!   注入时间 / id 的规则（`truncate_at_user_message_and_prepare_regenerate` 等）保留**同名薄包装**
//!   并在那层注入 Web 的时钟与 id 源。两条路都不动调用点。
//!
//! # 迁移方式
//!
//! 一个域试点成功后再复制下一个，**不整块重写** `app_signals` / `Effect` 接线
//! （Leptos 的 owner 与 `Rc` 捕获约束会惩罚大爆改，见 `crate::app` 的实测记录）。

pub mod session;
