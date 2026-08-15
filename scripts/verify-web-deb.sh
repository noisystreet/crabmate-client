#!/usr/bin/env bash
# 校验 crabmate-web .deb：包名、二进制、菜单图标、静态根；不得含 serve sidecar。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEB="${1:-}"
if [[ -z "${DEB}" ]]; then
  DEB="$(ls "${ROOT}/crates/crabmate-web-host/target/debian/crabmate-web_"*.deb | head -1)"
fi
test -f "${DEB}"
ls -lh "${DEB}"

dpkg-deb -I "${DEB}" | grep -E '^ Package: crabmate-web$'
contents="$(dpkg-deb -c "${DEB}")"
echo "${contents}" | grep -E 'usr/bin/crabmate-web$'
echo "${contents}" | grep -E 'usr/share/applications/crabmate-web\.desktop$'
echo "${contents}" | grep -E 'usr/share/crabmate-web/dist/index.html$'
echo "${contents}" | grep -E 'usr/share/icons/hicolor/128x128/apps/crabmate-web\.png$'
echo "${contents}" | grep -E 'usr/share/pixmaps/crabmate-web\.png$'

desktop="$(dpkg-deb --fsys-tarfile "${DEB}" | tar -xO ./usr/share/applications/crabmate-web.desktop)"
echo "${desktop}" | grep -E '^Icon=crabmate-web$'
echo "${desktop}" | grep -E '^Exec=crabmate-web$'

if echo "${contents}" | grep -E 'etc/crabmate/'; then
  echo "error: web deb must not ship /etc/crabmate (owned by server package)" >&2
  exit 1
fi
if echo "${contents}" | grep -E 'usr/bin/crabmate$'; then
  echo "error: deb must not contain crabmate serve sidecar" >&2
  exit 1
fi
echo "web deb package OK: ${DEB}"
