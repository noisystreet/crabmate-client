#!/usr/bin/env bash
# 校验 crabmate-tui .deb：仅二进制；无图标、无配置、无 serve sidecar。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEB="${1:-}"
if [[ -z "${DEB}" ]]; then
  DEB="$(ls "${ROOT}/crates/crabmate-tui/target/debian/crabmate-tui_"*.deb | head -1)"
fi
test -f "${DEB}"
ls -lh "${DEB}"

dpkg-deb -I "${DEB}" | grep -E '^ Package: crabmate-tui$'
contents="$(dpkg-deb -c "${DEB}")"
echo "${contents}" | grep -E 'usr/bin/crabmate-tui$'

if echo "${contents}" | grep -E 'usr/share/applications/'; then
  echo "error: tui deb must not ship a desktop menu entry" >&2
  exit 1
fi
if echo "${contents}" | grep -E 'usr/share/icons/|usr/share/pixmaps/'; then
  echo "error: tui deb must not ship icons" >&2
  exit 1
fi
if echo "${contents}" | grep -E 'etc/crabmate/'; then
  echo "error: tui deb must not ship /etc/crabmate (owned by server package)" >&2
  exit 1
fi
if echo "${contents}" | grep -E 'usr/bin/crabmate$'; then
  echo "error: deb must not contain crabmate serve sidecar" >&2
  exit 1
fi
echo "tui deb package OK: ${DEB}"
