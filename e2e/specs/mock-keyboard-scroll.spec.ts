/**
 * 键盘：消息区 Home / End 滚动 + Enter 发送回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_keyboard.rs` 迁入（#152 Wave 2）。
 * 原用例靠 `eval_js` 轮询 `scrollTop` 与注入 SSE 存根，都是页面行为。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  seedSession,
  openSessionInRail,
  sendMessage,
  installMockSse,
} from "../fixtures/helpers";

const SCROLLER = '[data-testid="chat-messages-scroller"]';

/** 播种 N 条 user 消息（足够撑出滚动区域）。 */
async function seedScrollSession(page: Page, sid: string, prefix: string) {
  const messages = Array.from({ length: 40 }, (_, i) => ({
    id: `m_${prefix}_${i}`,
    role: "user",
    text: `${prefix}-line-${i}`,
  }));
  await seedSession(page, sid, { session: { messages } });
  await openSessionInRail(page, sid);
  // 首条消息渲染出来即说明列表已就位
  await expect(page.getByText(`${prefix}-line-0`).first()).toBeAttached();
}

function scrollTop(page: Page) {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel);
    return el ? el.scrollTop : -1;
  }, SCROLLER);
}

function atBottom(page: Page) {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement | null;
    if (!el) return false;
    const max = el.scrollHeight - el.clientHeight;
    return max > 0 && el.scrollTop >= max - 4;
  }, SCROLLER);
}

test("End 键把消息区滚到底", async ({ page }) => {
  await seedScrollSession(page, "s_e2e_keys", "e2e-scroll");

  await page.getByTestId("chat-composer-input").focus();

  // 先滚到顶，确保 End 有距离可走
  await page.keyboard.press("Home");
  await expect.poll(() => scrollTop(page)).toBe(0);

  await page.keyboard.press("End");
  await expect.poll(() => atBottom(page)).toBe(true);
});

test("Home 键把消息区滚到顶", async ({ page }) => {
  await seedScrollSession(page, "s_e2e_keys_home", "e2e-home");

  // 先手动贴底
  await page.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement | null;
    if (el) el.scrollTop = el.scrollHeight;
  }, SCROLLER);
  expect(await scrollTop(page)).toBeGreaterThan(0);

  await page.getByTestId("chat-composer-input").focus();
  await page.keyboard.press("Home");

  await expect.poll(() => scrollTop(page)).toBe(0);
});

test("Enter 发送消息并渲染助手回复", async ({ page }) => {
  await seedScrollSession(page, "s_e2e_keys_enter", "enter-test");

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: Hello from E2E stub.\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  await sendMessage(page, "e2e enter send");

  await expect(page.getByText("Hello from E2E stub.").first()).toBeVisible();
});
