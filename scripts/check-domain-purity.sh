#!/usr/bin/env bash
# domain 纯规则层门禁（#598；扫描范围 = frontend/src/domain/**/*.rs）：
#   这一层存在的理由是「能在 host target 上 cargo test」（make test-frontend）。
#   一旦有人在这里引 web_sys / leptos 信号 / crate::api / 时钟 / 随机源，
#   wasm 侧照样编得过，host 单测却静默失效 —— 规则退回「只能靠整页 e2e 兜」，
#   正是 #598 要消掉的缺口。AGENTS.md 的 Frontend domain layer 条目给形状，
#   本门禁把它变成机械约束（issue 第 5 步：先立形状，再上门禁）。
#
# 门禁：命中即失败；就地豁免写 `domain-gate: allow <理由>`（理由为空不算豁免）。
# 判定细节（剥注释 / 不剥字符串 / 目录改名即失败）见 scripts/domain_purity_check.py。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/domain_purity_check.py" "$@"
