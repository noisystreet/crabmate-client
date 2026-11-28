#!/usr/bin/env bash
# 目录聚合行数预算 + 根级平铺冻结（#597；扫描范围 = scripts/layout_budget_check.py 里的扫描根）。
#   920 行单文件硬帽（fn-nloc）只防巨型文件，却成了事实上的模块划分依据：拆成 8 个 900 行文件
#   是免费通行证。本门禁补聚合维度 —— 一个域的总行数也要有预算，新增文件要有唯一落点（域目录）。
#
#   Rule A  目录预算：scripts/layout_budget.txt 每条的实际 .rs 行数必须**等于**预算（写多写少都 fail）；
#   Rule A2 不许嵌套登记（recursive 条目不得是另一条目的祖先，否则重复计数）；
#   Rule B  根级冻结：scripts/layout_root_flat.txt 逐文件冻结 frontend/src/app/*.rs（双向 stale 检查）；
#   Rule C  归属守恒：frontend/src/app 递归总行数 == 根级 direct + 各直接子目录 recursive 之和。
#
# 判定细节与「为什么覆盖规则只在 app/ 试点」见 scripts/layout_budget_check.py 头注释。
# 基线必须由 `python3 scripts/layout_budget_check.py --print-baseline` 生成（数字永远实测）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/layout_budget_check.py" "$@"
