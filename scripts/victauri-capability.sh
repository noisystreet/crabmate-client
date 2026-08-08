#!/usr/bin/env bash
# 确保 capabilities 含 victauri:default（仅 --features victauri 构建时合法）。
# 无 feature 的 release/check 不得长期保留该权限。
# 用法：source 后 ensure_victauri_capability；EXIT trap 调 restore_victauri_capability。
set -euo pipefail

_VICTAURI_CAP_FILE="${_VICTAURI_CAP_FILE:-}"
_VICTAURI_CAP_BACKUP=""
_VICTAURI_CAP_PATCHED=0

ensure_victauri_capability() {
  local tauri_dir="${1:?tauri dir}"
  _VICTAURI_CAP_FILE="${tauri_dir}/capabilities/default.json"
  if [[ ! -f "${_VICTAURI_CAP_FILE}" ]]; then
    echo "error: missing ${_VICTAURI_CAP_FILE}" >&2
    return 1
  fi
  if grep -q '"victauri:default"' "${_VICTAURI_CAP_FILE}"; then
    _VICTAURI_CAP_PATCHED=0
    echo "   capability already has victauri:default"
    return 0
  fi
  _VICTAURI_CAP_BACKUP="$(mktemp)"
  cp "${_VICTAURI_CAP_FILE}" "${_VICTAURI_CAP_BACKUP}"
  python3 - "${_VICTAURI_CAP_FILE}" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    data = json.load(f)
perms = data.setdefault("permissions", [])
if "victauri:default" not in perms:
    perms.append("victauri:default")
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, ensure_ascii=False, indent=2)
    f.write("\n")
PY
  _VICTAURI_CAP_PATCHED=1
  echo "   patched ${_VICTAURI_CAP_FILE} (+victauri:default; will restore on exit)"
}

restore_victauri_capability() {
  if [[ "${_VICTAURI_CAP_PATCHED}" != "1" ]]; then
    return 0
  fi
  if [[ -n "${_VICTAURI_CAP_BACKUP}" && -f "${_VICTAURI_CAP_BACKUP}" && -n "${_VICTAURI_CAP_FILE}" ]]; then
    cp "${_VICTAURI_CAP_BACKUP}" "${_VICTAURI_CAP_FILE}"
    rm -f "${_VICTAURI_CAP_BACKUP}"
    echo "   restored ${_VICTAURI_CAP_FILE}"
  fi
  _VICTAURI_CAP_PATCHED=0
}
