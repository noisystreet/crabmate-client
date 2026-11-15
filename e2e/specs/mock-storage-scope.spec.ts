/**
 * 连接分域的 `localStorage`（#457 W0-2/W0-3）。
 *
 * `localStorage` 属于 origin、不属于 WebView 实例，所以 Desktop 的多个连接窗口共用
 * 一份命名空间：键名必须带连接标识（`<base>#<conn>`），否则两个窗口互相覆盖。
 *
 * 浏览器里没有 Tauri 壳，但 `page.addInitScript` 与壳侧
 * `WebviewWindowBuilder::initialization_script` 同效 —— 都先于页面脚本执行 ——
 * 因此这条用例覆盖的正是壳内真实经历的那条读写路径（纯函数单测覆盖不到它）。
 *
 * 三条要守住的性质：
 *   1. 有标识 → 键名带后缀，且旧键值**迁移**过来；
 *   2. 分域键已存在 → 旧键**不再**参与（迁移只发生一次，不覆盖新值）；
 *   3. 没有标识（纯浏览器 / Android / 主窗口）→ 沿用旧键名，一个字节都不动。
 */
import { test, expect, type Page } from "@playwright/test";

import { homeUrlWithOptionalWebBearer } from "../fixtures/helpers";

/** 思考模式：本机两态、且按连接分域（换成另一台 serve 时模型配置本就不同）。 */
const THINKING_MODE_KEY = "crabmate-llm-thinking-mode";

/** 壳注入的标识所在的 JS 全局名（与 `api::storage_scope::INJECTED_GLOBAL` 同源）。 */
const INJECTED_GLOBAL = "__CM_CONN_KEY__";

/**
 * 打开业务 UI，并（可选）在页面脚本之前预置存储状态。
 *
 * 预置必须走 `addInitScript`：它先于前端模块执行，等价于壳的
 * `initialization_script`；用 `page.evaluate` 在导航后写就太晚了，水合已经读过。
 */
async function openWithStorage(
  page: Page,
  seed: { connKey?: string; legacy?: string; scoped?: string },
): Promise<void> {
  await page.addInitScript(
    (args: {
      connKey?: string;
      legacy?: string;
      scoped?: string;
      keys: string[];
    }) => {
      const [baseKey, globalName] = args.keys;
      try {
        if (typeof args.connKey === "string") {
          (window as unknown as Record<string, unknown>)[globalName] =
            args.connKey;
        }
        if (typeof args.legacy === "string") {
          localStorage.setItem(baseKey, args.legacy);
        }
        if (
          typeof args.scoped === "string" &&
          typeof args.connKey === "string"
        ) {
          localStorage.setItem(`${baseKey}#${args.connKey}`, args.scoped);
        }
      } catch {
        /* ignore */
      }
    },
    { ...seed, keys: [THINKING_MODE_KEY, INJECTED_GLOBAL] },
  );

  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "domcontentloaded",
    timeout: 20_000,
  });
  // 等前端挂载完成：思考模式在这一刻已经从 `localStorage` 读过一次。
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 20_000,
  });
}

/** 读一个键：返回 `null`（键不存在）或它的值。 */
async function readKey(page: Page, key: string): Promise<string | null> {
  return page.evaluate((k: string) => localStorage.getItem(k), key);
}

test("有连接标识时键名分域，旧值迁移过来", async ({ page }) => {
  await openWithStorage(page, { connKey: "lane-a", legacy: "on" });

  expect(await readKey(page, `${THINKING_MODE_KEY}#lane-a`)).toBe("on");
  // 旧键不删：回滚到旧版本时还能读到同一份值（明文 token 才需要删，它是另一条路径）。
  expect(await readKey(page, THINKING_MODE_KEY)).toBe("on");
});

test("分域键已存在时旧键不参与，迁移只发生一次", async ({ page }) => {
  await openWithStorage(page, {
    connKey: "lane-a",
    legacy: "on",
    scoped: "off",
  });

  // 分域键是权威：旧键里的 `on` 不能把已经存在的 `off` 覆盖回去。
  expect(await readKey(page, `${THINKING_MODE_KEY}#lane-a`)).toBe("off");
});

test("没有连接标识时沿用旧键名，不产生分域后缀", async ({ page }) => {
  await openWithStorage(page, { legacy: "on" });

  expect(await readKey(page, THINKING_MODE_KEY)).toBe("on");
  // 只认本键的分域形态：其它 localStorage 键（主题、locale、草稿）不属于本用例的范围。
  const scopedKeys = await page.evaluate(
    (base: string) =>
      Object.keys(localStorage).filter((k) => k.startsWith(`${base}#`)),
    THINKING_MODE_KEY,
  );
  expect(scopedKeys, "无标识的页面不该出现分域键").toEqual([]);
});
