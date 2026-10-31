/**
 * Mock 端到端测试：浅色主题首帧不再深闪（#322）
 *
 * 目的：`frontend/index.html` 的 `<head>` 预绘脚本应当在**首绘前**就把主题写进
 * `data-theme`，`data-theme$='-light'` 覆层据此把启动页刷成浅色——不再等 WASM 水合。
 *
 * 做法：阻断 `*.wasm` 让页面**无法水合**，于是断言的是「预绘 + 内联 CSS」的产物，
 * 与 App 无关；`addInitScript` 在页面脚本之前写镜像键（等同上一次会话留下的快照）。
 *
 * 三个用例恰好覆盖三条设计约束：
 *   1. 镜像为浅色偏好 → 首绘即浅色（修复目标）；
 *   2. 无镜像（首次安装 / 清过存储）→ 深色基线，不预置默认主题 `shadcn-light`
 *      （否则深色用户首访会闪浅色）；
 *   3. 镜像为未知 slug → 属性照写但不匹配任何主题规则，自然回落深色
 *      （脚本不抄主题白名单，靠 CSS 回落）。
 *
 * 运行方式（前置：`make web-release` 后 `crabmate-web` 在 127.0.0.1:4173 托管 `frontend/dist`；
 * 无需 serve，因为断言全部发生在水合之前）：
 *   cd e2e && npx playwright test specs/mock-first-paint-theme.spec.ts
 */

import { test, expect, type Page } from "@playwright/test";

/** 与 `frontend/src/app/shell_prefs_storage.rs::THEME_PREF_MIRROR_KEY` 同键。 */
const THEME_MIRROR_KEY = "crabmate.theme.pref";
/** 深色基线 `--bg`（#07090e）的计算值；浅色主题下背景必须不等于它。 */
const DARK_BG_RGB = "rgb(7, 9, 14)";
const SPLASH_ID = "cm-boot-splash";

/** 阻断 WASM：页面停在预绘阶段，任何后续翻转都不会发生。 */
async function blockWasm(page: Page): Promise<void> {
  await page.route("**/*.wasm", (route) => route.abort());
}

async function seedThemeMirror(page: Page, value: string): Promise<void> {
  await page.addInitScript(
    (args: { key: string; value: string }) => {
      window.localStorage.setItem(args.key, args.value);
    },
    { key: THEME_MIRROR_KEY, value },
  );
}

/** 用 `removeItem` 而不是「不写」：同一浏览器上下文可能被前面用例写过。 */
async function clearThemeMirror(page: Page): Promise<void> {
  await page.addInitScript((key: string) => {
    window.localStorage.removeItem(key);
  }, THEME_MIRROR_KEY);
}

async function readDataTheme(page: Page): Promise<string | null> {
  return page.evaluate(() =>
    document.documentElement.getAttribute("data-theme"),
  );
}

async function readBackgroundColor(page: Page, id?: string): Promise<string> {
  return page.evaluate((elementId?: string) => {
    const el = elementId
      ? document.getElementById(elementId)
      : document.documentElement;
    if (!el) return "";
    return getComputedStyle(el).backgroundColor;
  }, id);
}

test.describe("首帧主题（预绘脚本）", () => {
  test("镜像为浅色偏好时首绘即浅色，启动页同步变浅", async ({ page }) => {
    await seedThemeMirror(page, "crabmate-light");
    await blockWasm(page);
    await page.goto("/");

    await expect.poll(() => readDataTheme(page)).toBe("crabmate-light");
    expect(await readBackgroundColor(page)).not.toBe(DARK_BG_RGB);
    // 启动页是 id 选择器的深色规则，浅色覆层必须一并压住它，否则仍会闪一帧深色。
    expect(await readBackgroundColor(page, SPLASH_ID)).not.toBe(DARK_BG_RGB);
  });

  test("无镜像时保持深色基线（不预置默认主题）", async ({ page }) => {
    await clearThemeMirror(page);
    await blockWasm(page);
    await page.goto("/");

    expect(await readDataTheme(page)).toBeNull();
    expect(await readBackgroundColor(page)).toBe(DARK_BG_RGB);
    expect(await readBackgroundColor(page, SPLASH_ID)).toBe(DARK_BG_RGB);
  });

  test("镜像为未知 slug 时照写属性但回落深色", async ({ page }) => {
    await seedThemeMirror(page, "unknown-theme");
    await blockWasm(page);
    await page.goto("/");

    expect(await readDataTheme(page)).toBe("unknown-theme");
    expect(await readBackgroundColor(page)).toBe(DARK_BG_RGB);
  });
});
