#!/usr/bin/env bash
# ACTIVE 作用域门禁（#558 项 5/6；扫描范围 = frontend/src/**/*.rs）：
#   规则 A · 读 `ACTIVE`（前台那份）的零参口只允许出现在 scripts/active_scope_allowlist.txt
#            里，每条须写理由；条目在代码里消失（陈旧）也失败 —— 这是 #558 项 5 缺的
#            「约束机制（谁能读它）」。
#   规则 B · 零参包装（api_base_url / api_url / prepare_api_auth / auth_headers /
#            web_api_bearer_token）的每文件调用次数必须等于 scripts/active_scope_budget.txt，
#            增一处失败（跨 lane 路径要用 *_on(holder)），少一处也要下调预算（ratchet）。
#
# 门禁（与 scripts/active_scope_check.py 一致）：规则 A 未登记/陈旧、规则 B 增减/遗漏/陈旧
# 全部必须为 0。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/active_scope_check.py" "$@"
