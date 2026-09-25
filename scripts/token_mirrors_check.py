#!/usr/bin/env python3
"""非 CSS 表面 ↔ 设计令牌镜像门禁。

颜色令牌的单一来源是 frontend/styles/tokens.css 的 :root。有三类表面无法直接引用 CSS 变量，
只能把色值各抄一份，抄错既不报错、也只是「看起来有点怪」，故机械校验：

  1. **Android 原生层镜像令牌**：res/values/colors.xml 的 cm_<token>（下划线代替连字符）必须等于
     tokens.css 的对应 --<token>；该文件只允许放这些镜像令牌，多出一个裸颜色即失败。
  2. **Android 主题**：res/values*/themes.xml 不得出现颜色字面量，应用主题的四个底色必须引用
     @color/cm_bg，且必须显式声明系统栏图标用浅色（windowLightStatusBar /
     windowLightNavigationBar = false）—— 底色恒为深色令牌，而 DayNight 的浅色变体默认给深色
     图标，漏声明就会在深色状态栏上看见一片空（图标与底色同色）。
  3. **各启动页首绘底色**：frontend/index.html 的内联样式、desktop-tauri/splash.html 与
     crates/crabmate-connect/assets/connect.html 的 bg0 变量，必须等于 --bg；否则首帧会闪出
     另一种深色（未来令牌改浅色时则闪黑）。

另外两处「抄漏也静默生效」的缺口一并堵上：

  4. **主题里的颜色引用必须真实存在**：res/values*/themes.xml 出现的 @color/<name> 要在
     res/values*/colors.xml 里有定义 —— 拼错（cm_textt）只在 AAPT 构建期报错，门禁先行拦下。
  5. **手写 Kotlin 只能消费镜像令牌**：app/src/main/java/edu/crabmate 下的 Kotlin 若直接用
     R.color.<非 cm_*>，就绕开了镜像比对（值可以随便写），故一并禁止。

五条规则都是「精确相等」，没有白名单：要改值就必须同时改两侧。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOKENS_CSS = ROOT / "frontend" / "styles" / "tokens.css"
ANDROID_SRC = ROOT / "mobile-tauri" / "src-tauri" / "gen" / "android" / "app" / "src" / "main"
ANDROID_RES = ANDROID_SRC / "res"
# 手写 Kotlin 的范围与 scripts/ktlint-android.sh 一致（edu/crabmate，排除 Tauri 生成的 generated/）。
ANDROID_KOTLIN = ANDROID_SRC / "java" / "edu" / "crabmate"
COLORS_XML = ANDROID_RES / "values" / "colors.xml"
APP_THEME = "Theme.crabmate_mobile"

# 原生层实际消费的令牌：cm_<token> ↔ --<token>。新增消费点时把 token 名加进这里并补 colors.xml，
# 只留一侧会被「多出 / 缺失」检查拦下。
MIRRORED_TOKENS = ("bg", "surface", "text", "muted", "accent", "error")

# 首绘底色：下面的正则在对应文件里必须命中且仅命中一次，否则说明声明被改名 / 搬走，门禁会静默失效。
BOOT_BACKGROUNDS = (
    ("frontend/index.html", re.compile(r"background:\s*(#[0-9a-fA-F]{3,8})\s*;")),
    ("desktop-tauri/splash.html", re.compile(r"--bg0:\s*(#[0-9a-fA-F]{3,8})\s*;")),
    ("crates/crabmate-connect/assets/connect.html", re.compile(r"--bg0:\s*(#[0-9a-fA-F]{3,8})\s*;")),
)

# 应用主题里必须逐项出现的项（键 → 允许的取值）。
REQUIRED_THEME_ITEMS = {
    "android:windowBackground": "@color/cm_bg",
    "android:colorBackground": "@color/cm_bg",
    "android:statusBarColor": "@color/cm_bg",
    "android:navigationBarColor": "@color/cm_bg",
    "android:windowLightStatusBar": "false",
    "android:windowLightNavigationBar": "false",
}

CSS_COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
ROOT_SELECTOR_RE = re.compile(r":root\s*\{")
DECL_RE = re.compile(r"(--[A-Za-z0-9_-]+)\s*:\s*([^;]+);")
XML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
COLOR_LITERAL_RE = re.compile(r"#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(")
HEX6_RE = re.compile(r"^#([0-9a-fA-F]{6})$")
ANDROID_COLOR_RE = re.compile(r'<color\s+name="([^"]+)"\s*>\s*([^<]+?)\s*</color>')
COLOR_REF_RE = re.compile(r"@color/([A-Za-z0-9_]+)")
KOTLIN_COLOR_RE = re.compile(r"R\.color\.([A-Za-z0-9_]+)")
KOTLIN_BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", re.S)
KOTLIN_LINE_COMMENT_RE = re.compile(r"//[^\n]*")
STYLE_RE = re.compile(r'<style\s+name="%s"[^>]*>(.*?)</style>' % re.escape(APP_THEME), re.S)
ITEM_RE = re.compile(r'<item\s+name="([^"]+)"[^>]*>\s*([^<]*?)\s*</item>', re.S)


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT))


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def strip_xml_comments(text: str) -> str:
    """注释按等长空格抹掉，保证行号与原文一致。"""
    return XML_COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def strip_css_comments(text: str) -> str:
    return CSS_COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)


def strip_kotlin_comments(text: str) -> str:
    """块注释先抹（等长空格保行号），再抹行注释——注释掉的 R.color.* 不该算消费点。"""
    without_blocks = KOTLIN_BLOCK_COMMENT_RE.sub(lambda m: " " * len(m.group(0)), text)
    return KOTLIN_LINE_COMMENT_RE.sub(lambda m: " " * len(m.group(0)), without_blocks)


def iter_root_blocks(css: str):
    """逐个产出 :root 块的正文。按花括号配平切分——注释里的 `{}` 会骗过朴素正则。"""
    text = strip_css_comments(css)
    pos = 0
    while True:
        m = ROOT_SELECTOR_RE.search(text, pos)
        if not m:
            return
        depth = 1
        i = m.end()
        start = i
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        yield text[start : i - 1]
        pos = i


def root_token_colors(css: str) -> dict[str, str]:
    """tokens.css :root 里 6 位 hex 形式的颜色令牌（color-mix 等其余形态不入表）。"""
    out: dict[str, str] = {}
    for block in iter_root_blocks(css):
        for name, value in DECL_RE.findall(block):
            m = HEX6_RE.match(value.strip())
            if m:
                out.setdefault(name, "#" + m.group(1).lower())
    return out


def android_color(raw: str) -> str | None:
    """Android 颜色（AARRGGBB / RRGGBB）归一为 6 位小写；非不透明 alpha 视为非法。"""
    value = raw.strip()
    if not value.startswith("#"):
        return None
    body = value[1:]
    if len(body) == 8:
        if body[:2].lower() != "ff":
            return None
        body = body[2:]
    if len(body) != 6 or not all(c in "0123456789abcdefABCDEF" for c in body):
        return None
    return "#" + body.lower()


def declared_color_names() -> set[str]:
    """res/values*/colors.xml 里 <color name="..."> 声明的名字（night 变体一并算入）。"""
    names: set[str] = set()
    for path in sorted(ANDROID_RES.glob("values*/colors.xml")):
        names |= {name for name, _ in ANDROID_COLOR_RE.findall(read(path))}
    return names


def check_colors_xml(tokens: dict[str, str], failures: list[str]) -> None:
    if not COLORS_XML.is_file():
        failures.append(f"{rel(COLORS_XML)} 不存在")
        return
    declared = dict(ANDROID_COLOR_RE.findall(read(COLORS_XML)))
    expected = {f"cm_{token}": f"--{token}" for token in MIRRORED_TOKENS}
    for name in sorted(set(declared) - set(expected)):
        failures.append(f"{rel(COLORS_XML)} 多出 {name}；该文件只放与 tokens.css 对应的 cm_* 镜像令牌")
    for name, token in expected.items():
        if name not in declared:
            failures.append(f"{rel(COLORS_XML)} 缺少 {name}（对应 tokens.css 的 {token}）")
            continue
        want = tokens.get(token)
        got = android_color(declared[name])
        if want is None:
            failures.append(f"tokens.css 未以 6 位 hex 定义 {token}，无法与 {name} 比对")
        elif got is None:
            failures.append(f"{rel(COLORS_XML)} 的 {name} = {declared[name]!r} 不是不透明颜色字面量")
        elif got != want:
            failures.append(f"{name} = {got} 与 tokens.css 的 {token} = {want} 不一致")


def check_themes(declared: set[str], failures: list[str]) -> None:
    files = sorted(ANDROID_RES.glob("values*/themes.xml"))
    if not files:
        failures.append(f"{rel(ANDROID_RES)}/values*/themes.xml 一个都没有")
        return
    for path in files:
        where = rel(path)
        text = strip_xml_comments(read(path))
        for hit in COLOR_LITERAL_RE.findall(text):
            failures.append(f"{where} 出现颜色字面量 {hit!r}；主题必须引用 @color/ 令牌")
        for name in sorted(set(COLOR_REF_RE.findall(text))):
            if name not in declared:
                failures.append(
                    f"{where} 引用了 @color/{name}，但 res/values*/colors.xml 没有定义"
                    "（拼错只会到 AAPT 构建期才报）"
                )
        blocks = STYLE_RE.findall(text)
        if len(blocks) != 1:
            failures.append(f"{where} 里 {APP_THEME} 定义数 = {len(blocks)}，应为 1")
            continue
        items = dict(ITEM_RE.findall(blocks[0]))
        for item, allowed in REQUIRED_THEME_ITEMS.items():
            value = items.get(item)
            if value is None:
                failures.append(f"{where} 的 {APP_THEME} 缺少 {item}（应为 {allowed}）")
            elif value != allowed:
                failures.append(f"{where} 的 {item} = {value!r}，应为 {allowed!r}")


def check_boot_backgrounds(tokens: dict[str, str], failures: list[str]) -> None:
    want = tokens.get("--bg")
    if want is None:
        failures.append("tokens.css 未以 6 位 hex 定义 --bg")
        return
    for relative, pattern in BOOT_BACKGROUNDS:
        path = ROOT / relative
        if not path.is_file():
            failures.append(f"{relative} 不存在")
            continue
        hits = pattern.findall(read(path))
        if len(hits) != 1:
            failures.append(
                f"{relative} 命中首绘底色声明 {len(hits)} 处（应为 1）；声明被改名 / 搬走后门禁会静默失效"
            )
            continue
        m = HEX6_RE.match(hits[0])
        got = ("#" + m.group(1).lower()) if m else None
        if got != want:
            failures.append(f"{relative} 首绘底色 {hits[0]} 与 --bg {want} 不一致")


def check_kotlin_color_consumption(failures: list[str]) -> None:
    """手写 Kotlin 只能消费 cm_* 镜像令牌；直接用别的颜色资源即绕过整条镜像链。"""
    files = sorted(p for p in ANDROID_KOTLIN.glob("**/*.kt") if "generated" not in p.parts)
    if not files:
        failures.append(f"{rel(ANDROID_KOTLIN)} 下没有 Kotlin 文件；消费点检查会静默失效")
        return
    allowed = {f"cm_{token}" for token in MIRRORED_TOKENS}
    for path in files:
        for name in sorted(set(KOTLIN_COLOR_RE.findall(strip_kotlin_comments(read(path))))):
            if name not in allowed:
                failures.append(
                    f"{rel(path)} 消费了 R.color.{name}；手写 Kotlin 的颜色必须走 cm_* 镜像令牌"
                    f"（当前镜像集：{'、'.join(sorted(allowed))}）"
                )


def main() -> int:
    if not TOKENS_CSS.is_file():
        print(f"{rel(TOKENS_CSS)} 不存在", file=sys.stderr)
        return 1
    tokens = root_token_colors(read(TOKENS_CSS))
    failures: list[str] = []
    check_colors_xml(tokens, failures)
    check_themes(declared_color_names(), failures)
    check_boot_backgrounds(tokens, failures)
    check_kotlin_color_consumption(failures)
    if failures:
        print("设计令牌镜像门禁失败（单一来源 = frontend/styles/tokens.css 的 :root）：", file=sys.stderr)
        for item in failures:
            print(f"  - {item}", file=sys.stderr)
        return 1
    print("token mirror ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
