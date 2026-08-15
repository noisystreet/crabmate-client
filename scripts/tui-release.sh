#!/usr/bin/env bash
# make tui-release：crabmate-tui 二进制 + .deb（仅 /usr/bin；无图标、无配置文件）
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUI_DIR="${ROOT}/crates/crabmate-tui"

echo "building crabmate-tui…" >&2
cargo build --release --manifest-path "${TUI_DIR}/Cargo.toml" --bin crabmate-tui

bash "${ROOT}/scripts/pack-tui-deb.sh"
