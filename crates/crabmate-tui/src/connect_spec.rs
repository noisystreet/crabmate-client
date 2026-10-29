//! `--connect name=url` 的解析。
//!
//! 独立成文件：`main.rs` 贴着 `fn-nloc-ratchet` 的 920 行/文件门禁，而这段纯解析
//! 与 CLI 其余部分没有耦合（用例见 `connect_spec_tests.rs`）。

use anyhow::{Context, Result};

/// 解析 `--connect name=url`；`name` 留空时直接拿 URL 当名字（好认过没有）。
pub(crate) fn parse_connect_spec(spec: &str) -> Result<(String, String)> {
    let (name, url) = spec
        .split_once('=')
        .with_context(|| format!("--connect 需要 <name>=<url>，收到：{spec}"))?;
    let url = url.trim().to_string();
    if url.is_empty() {
        anyhow::bail!("--connect 的 url 不能为空：{spec}");
    }
    let name = name.trim();
    let name = if name.is_empty() {
        url.clone()
    } else {
        name.to_string()
    };
    Ok((name, url))
}
