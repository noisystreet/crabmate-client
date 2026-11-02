#!/usr/bin/env bash
# CHANGELOG 条目长度门禁：只校验未发布段（`## [Unreleased]`），已发布段冻结。
#
# 为什么只校验 Unreleased：0.8.0-alpha.0 及更早的段是**已发布**的发布说明
# （`.github/workflows/release.yml` 的 "Extract CHANGELOG section for this tag" 按段抽取），
# 改写它们会让文件与 GitHub 上已固化的 Release notes 永久不一致。所以历史段一律不动，
# 门禁只拦「新写进去的条目」——从下一个版本切段起，条目自然都是短的。
#
# 为什么是 800：历史条目平均 2900 字符、96 条超过 2000，文件已到 51 万字符无法通读。
# 800 字符能放下「改了什么 + 为什么 + 用户可见影响」，而机制、备选方案、验证清单都该进
# PR 描述；能长期留存的理由进 docs/design/。上限比目标（约 600）宽一点，避免边界摩擦。
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT/CHANGELOG.md" <<'PY'
import pathlib
import sys

MAX_CHARS = 800
path = pathlib.Path(sys.argv[1])
if not path.is_file():
    print(f"check-changelog-style: 找不到 {path}", file=sys.stderr)
    sys.exit(2)

lines = path.read_text(encoding="utf-8").splitlines()
in_unreleased = False
seen_unreleased = False
too_long = []

for lineno, line in enumerate(lines, 1):
    if line.startswith("## "):
        in_unreleased = line.startswith("## [Unreleased]")
        seen_unreleased = seen_unreleased or in_unreleased
        continue
    if not in_unreleased or not line.startswith("- "):
        continue
    chars = len(line)
    if chars > MAX_CHARS:
        too_long.append((lineno, chars, line[:70]))

if not seen_unreleased:
    print(
        "check-changelog-style: CHANGELOG.md 里没有 `## [Unreleased]` 段"
        "（发布时整段会被移进版本段；若确实没有未发布改动，保留空段即可）",
        file=sys.stderr,
    )
    sys.exit(2)

if too_long:
    print(
        f"check-changelog-style: [Unreleased] 有 {len(too_long)} 条超过 {MAX_CHARS} 字符",
        file=sys.stderr,
    )
    for lineno, chars, head in too_long:
        print(f"  CHANGELOG.md:{lineno}  {chars} 字符：{head}…", file=sys.stderr)
    print(
        "  条目只写「改了什么 + 为什么 + 用户可见影响」；机制 / 备选方案 / 验证清单进 PR 描述，"
        "长期有效的设计理由进 docs/design/。",
        file=sys.stderr,
    )
    sys.exit(1)

print(f"check-changelog-style ok（[Unreleased] 条目 ≤ {MAX_CHARS} 字符）")
PY
