//! `tui` 子命令的**连接面**参数：额外 lane、控制面 socket、Web Bearer。
//!
//! 收成一个结构有两个原因：`run_tui_cmd` 已经到 clippy 的 7 参上限；这三项也确实
//! 是一组（都描述「连谁 / 从哪控制」），分开传入只会把调用点撑得更长。
//!
//! 独立成文件是为了 `main.rs` 的 920 行门禁。

/// `tui` 的额外连接与本地控制面。
pub(crate) struct TuiWiring<'a> {
    /// `--connect name=url` 追加的额外 lane（主连接恒为第一条）。
    pub(crate) connect: &'a [String],
    /// `crabmatectl` 控制面 socket；`None` = 未启用。
    ///
    /// 没有主连接时它是**唯一**能连上去的通道，所以 `run_tui_cmd` 会先把它解析成
    /// 可选路径，再据此决定「进未连接界面」还是「直接报错」。
    pub(crate) ctl_socket: Option<&'a str>,
}
