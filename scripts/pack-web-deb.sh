#!/usr/bin/env bash
# 把 crabmate-web 二进制 + frontend/dist 打成 Debian 包（Package: crabmate-web）。
# 不是 Tauri，也不内嵌 crabmate serve。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_DIR="${ROOT}/crates/crabmate-web-host"
FRONTEND_DIST="${CRABMATE_FRONTEND_DIST:-${ROOT}/frontend/dist}"
BIN="${HOST_DIR}/target/release/crabmate-web"
MAX_WASM_BYTES=$((40 * 1024 * 1024))

command -v dpkg-deb >/dev/null 2>&1 || {
  echo "error: dpkg-deb not found (Debian/Ubuntu: apt install dpkg)" >&2
  exit 1
}

if [[ ! -x "${BIN}" ]]; then
  echo "error: missing ${BIN}; build with: cargo build --release --manifest-path ${HOST_DIR}/Cargo.toml" >&2
  exit 1
fi
if [[ ! -f "${FRONTEND_DIST}/index.html" ]]; then
  echo "error: missing ${FRONTEND_DIST}/index.html; run make frontend-release" >&2
  exit 1
fi

mapfile -t wasm_found < <(find "${FRONTEND_DIST}" -type f -name '*.wasm' 2>/dev/null || true)
if [[ "${#wasm_found[@]}" -eq 0 ]]; then
  echo "error: no .wasm under ${FRONTEND_DIST}" >&2
  exit 1
fi
for w in "${wasm_found[@]}"; do
  sz=$(wc -c <"${w}" | tr -d ' ')
  if [[ "${sz}" -eq 0 ]]; then
    echo "error: empty WASM ${w} (wasm-opt missing?)" >&2
    exit 1
  fi
  if [[ "${sz}" -gt "${MAX_WASM_BYTES}" ]]; then
    echo "error: ${w} is ${sz} bytes — looks like a debug trunk build" >&2
    echo "  run: make frontend-release" >&2
    exit 1
  fi
done

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${HOST_DIR}/Cargo.toml" | head -1)"
arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
pkg="crabmate-web_${version}_${arch}.deb"
stage="$(mktemp -d "${TMPDIR:-/tmp}/crabmate-web-deb.XXXXXX")"
cleanup() { rm -rf "${stage}"; }
trap cleanup EXIT

ICON_SRC="${ROOT}/desktop-tauri/src-tauri/icons"
install_hicolor_icon() {
  local size="$1" src="$2"
  if [[ ! -f "${src}" ]]; then
    echo "error: missing desktop icon ${src}" >&2
    exit 1
  fi
  local dest_dir="${stage}/usr/share/icons/hicolor/${size}/apps"
  mkdir -p "${dest_dir}"
  install -m 0644 "${src}" "${dest_dir}/crabmate-web.png"
}

mkdir -p \
  "${stage}/DEBIAN" \
  "${stage}/usr/bin" \
  "${stage}/usr/share/applications" \
  "${stage}/usr/share/crabmate-web" \
  "${stage}/usr/share/pixmaps"

install -m 0755 "${BIN}" "${stage}/usr/bin/crabmate-web"
cp -a "${FRONTEND_DIST}" "${stage}/usr/share/crabmate-web/dist"
install -m 0644 "${HOST_DIR}/packaging/crabmate-web.desktop" \
  "${stage}/usr/share/applications/crabmate-web.desktop"

# Same brand icons as crabmate-desktop (Freedesktop hicolor + pixmap fallback).
install_hicolor_icon 32x32 "${ICON_SRC}/32x32.png"
install_hicolor_icon 64x64 "${ICON_SRC}/64x64.png"
install_hicolor_icon 128x128 "${ICON_SRC}/128x128.png"
install_hicolor_icon 256x256 "${ICON_SRC}/128x128@2x.png"
install_hicolor_icon 512x512 "${ICON_SRC}/icon.png"
install -m 0644 "${ICON_SRC}/128x128.png" "${stage}/usr/share/pixmaps/crabmate-web.png"

size_kb="$(du -sk "${stage}" | awk '{print $1}')"
cat >"${stage}/DEBIAN/control" <<EOF
Package: crabmate-web
Version: ${version}
Section: utils
Priority: optional
Architecture: ${arch}
Installed-Size: ${size_kb}
Depends: xdg-utils, hicolor-icon-theme
Maintainer: CrabMate <noreply@crabmate.local>
Homepage: https://github.com/noisystreet/crabmate-client
Description: CrabMate web UI host (loopback static server)
 Hosts the packaged frontend on 127.0.0.1 and opens the system browser.
 Does not include crabmate serve; point --api-base at a running API.
EOF

out_dir="${HOST_DIR}/target/debian"
mkdir -p "${out_dir}"
out="${out_dir}/${pkg}"
dpkg-deb --root-owner-group --build "${stage}" "${out}"
echo "wrote ${out}"
ls -lh "${out}"
