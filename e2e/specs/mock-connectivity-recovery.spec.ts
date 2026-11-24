/**
 * issue #23：网络 / 前台恢复后**只补失败项**。
 *
 * 覆盖三条验收：
 * 1. `/status` 首次失败 → 底栏出现重试入口（手动路径仍在，是这里的基线）
 * 2. `online` → 自动重试成功 → 重试入口消失
 * 3. 连续多个恢复事件 → 冷却只放行一次（不并发发起重复请求）
 *
 * 用 `page.route` 控制「先失败后放行」，**不构造** `/status` 的响应体 —— 那形状随契约变，
 * mock 一份就多一处漂移。放行时 `route.continue()` 打真实 serve（E2E 环境本就要求它）。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

/** 底栏 `/status` 失败时的重试按钮（`status_bar::StatusFetchErrorPanel`）。 */
const RETRY = '[data-testid="status-fetch-retry"]';

type StatusGate = {
  /** 累计被拦截的 `/status` 请求数。 */
  calls: number;
  /** `true` 时一律 503，`false` 时放行到真实 serve。 */
  failing: boolean;
};

/** 装一个可切换的 `/status` 拦截器（须在 `seedSession` 之前调用，首屏请求才能被拦）。 */
async function installStatusGate(page: Page): Promise<StatusGate> {
  const gate: StatusGate = { calls: 0, failing: true };
  await page.route("**/status", async (route) => {
    gate.calls += 1;
    if (gate.failing) {
      await route.fulfill({
        status: 503,
        contentType: "application/json",
        body: "{}",
      });
      return;
    }
    await route.continue();
  });
  return gate;
}

/** 模拟浏览器/WebView 报「网络恢复」。 */
async function fireOnline(page: Page) {
  await page.evaluate(() => {
    window.dispatchEvent(new Event("online"));
  });
}

test("online 恢复后自动重试 /status 并收起错误面板", async ({ page }) => {
  const gate = await installStatusGate(page);
  await seedSession(page, "s_e2e_conn_recover");

  // 首屏失败：重试入口可见（这条也保证手动路径没被削弱）。
  await expect(page.locator(RETRY)).toBeVisible();
  const before = gate.calls;

  gate.failing = false;
  await fireOnline(page);

  // 自动重试成功：错误面板收起。
  await expect(page.locator(RETRY)).toBeHidden();
  expect(gate.calls).toBeGreaterThan(before);
});

test("多个恢复事件不会并发发起重复请求", async ({ page }) => {
  // 一直失败：这样每次事件都还有「可补」的目标，冷却才是唯一的约束。
  const gate = await installStatusGate(page);
  await seedSession(page, "s_e2e_conn_dedupe");
  await expect(page.locator(RETRY)).toBeVisible();

  const before = gate.calls;
  // 一次派发 5 个事件，模拟 Android 切网时 online / visible 抖动。
  await page.evaluate(() => {
    for (let i = 0; i < 5; i += 1) {
      window.dispatchEvent(new Event("online"));
    }
  });
  // 冷却窗（1.5s）内**恰好**一次：`<= 1` 会让「一次都没发」也通过，
  // 那正是「恢复根本没生效」的形态。
  await page.waitForTimeout(600);
  expect(gate.calls - before).toBe(1);
});

test("手动重试入口仍然有效", async ({ page }) => {
  const gate = await installStatusGate(page);
  await seedSession(page, "s_e2e_conn_manual");
  await expect(page.locator(RETRY)).toBeVisible();

  gate.failing = false;
  const before = gate.calls;
  await page.locator(RETRY).click();

  await expect(page.locator(RETRY)).toBeHidden();
  expect(gate.calls).toBeGreaterThan(before);
});
