#!/usr/bin/env bash
# 把 crabmate-tui 二进制打成 Debian 包（Package: crabmate-tui）。
# 仅 /usr/bin/crabmate-tui；无菜单图标、无配置文件、不内嵌 crabmate serve。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUI_DIR="${ROOT}/crates/crabmate-tui"
BIN="${TUI_DIR}/target/release/crabmate-tui"

command -v dpkg-deb >/dev/null 2>&1 || {
  echo "error: dpkg-deb not found (Debian/Ubuntu: apt install dpkg)" >&2
  exit 1
}

if [[ ! -x "${BIN}" ]]; then
  echo "error: missing ${BIN}; build with: cargo build --release --manifest-path ${TUI_DIR}/Cargo.toml --bin crabmate-tui" >&2
  exit 1
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${TUI_DIR}/Cargo.toml" | head -1)"
arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
pkg="crabmate-tui_${version}_${arch}.deb"
stage="$(mktemp -d "${TMPDIR:-/tmp}/crabmate-tui-deb.XXXXXX")"
cleanup() { rm -rf "${stage}"; }
trap cleanup EXIT

mkdir -p "${stage}/DEBIAN" "${stage}/usr/bin"
install -m 0755 "${BIN}" "${stage}/usr/bin/crabmate-tui"

size_kb="$(du -sk "${stage}" | awk '{print $1}')"
cat >"${stage}/DEBIAN/control" <<EOF
Package: crabmate-tui
Version: ${version}
Section: utils
Priority: optional
Architecture: ${arch}
Installed-Size: ${size_kb}
Maintainer: CrabMate <noreply@crabmate.local>
Homepage: https://github.com/noisystreet/crabmate-client
Description: CrabMate remote terminal client
 Connects to a running crabmate serve over HTTP/SSE (chat / repl).
 Does not include crabmate serve, desktop icons, or config files.
EOF

out_dir="${TUI_DIR}/target/debian"
mkdir -p "${out_dir}"
out="${out_dir}/${pkg}"
dpkg-deb --root-owner-group --build "${stage}" "${out}"
echo "wrote ${out}"
ls -lh "${out}"
