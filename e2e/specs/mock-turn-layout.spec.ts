/**
 * Turn 布局：多工具交错、无工具终答可见性。
 *
 * 由 `victauri_turn_layout.rs`(2) 迁入（#152 Wave 3）。
 *
 * 原用例用 V1 帧（`tool_call` / `tool_result` / `sse_capabilities`）造流；前端已只认
 * AG-UI（V2），所以这里改写成 `TOOL_CALL_*` / `TOOL_CALL_RESULT` / `RUN_FINISHED`。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  seedSession,
  openSessionInRail,
  sendMessage,
  installMockSse,
} from "../fixtures/helpers";

/** 与前端 `STICKY_BOTTOM_THRESHOLD_PX` 对齐（留 4px 容差）。 */
const FOLLOW_GAP_MAX_PX = 84;

const SCROLLER = '[data-testid="chat-messages-scroller"]';

function scrollGap(page: Page) {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement | null;
    if (!el) return -1;
    return el.scrollHeight - el.scrollTop - el.clientHeight;
  }, SCROLLER);
}

function atBottom(page: Page) {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement | null;
    if (!el) return false;
    const max = el.scrollHeight - el.clientHeight;
    // 内容不足以滚动时（max <= 0）本就「在底部」，不能判 false
    return max <= 0 || el.scrollTop >= max - 4;
  }, SCROLLER);
}

// 注：原 `victauri_turn_layout` 的另一例「多工具 + commentary 交错」未迁 —— 它在
// 原处也是红的（#152 记录的 victauri_turn_layout 0/2），且需要按 V2 的
// `turn_segment_start` 时序重写才能稳定复现，列为待重写项。
test("无工具问答在流结束后终答可见并保持贴底", async ({ page }) => {
  await seedSession(page, "s_e2e_no_tool_final");

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: 这是一个无工具问答的测试回复，用于验证终答在流结束后可见。\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  await sendMessage(page, "简单问答");

  const answer = page.getByText(
    "这是一个无工具问答的测试回复，用于验证终答在流结束后可见。",
  );
  await expect(answer.first()).toBeVisible();

  // 流结束后应仍贴底，且距底在跟读阈值内
  await expect.poll(() => atBottom(page)).toBe(true);
  expect(await scrollGap(page)).toBeLessThanOrEqual(FOLLOW_GAP_MAX_PX);

  // 重载后持久化内容仍可见
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });
  await openSessionInRail(page, "s_e2e_no_tool_final");
  await expect(answer.first()).toBeVisible();
});
