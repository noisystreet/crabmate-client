/**
 * 发送消息后的自动滚底与「跟读」语义。
 *
 * 由 `victauri_scroll_send.rs`(4) 迁入（#152 Wave 3）。
 *
 * 原用例用 V1 帧造流；这里改用 V2。流式增长的用例走 `installDelayedMockSse`
 * （按 delay 逐帧喂），才能真的观察「流中是否保持贴底」。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  seedSession,
  openSessionInRail,
  sendMessage,
  installMockSse,
  installDelayedMockSse,
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

/** 播种 N 条消息，撑出可滚动区域。 */
async function seedScrollable(page: Page, sid: string, count: number) {
  const messages = Array.from({ length: count }, (_, i) => ({
    id: `m_${i}`,
    role: "user",
    text: `scroll-test-line-${i}`,
  }));
  await seedSession(page, sid, { session: { messages } });
  await openSessionInRail(page, sid);
}

/** 多段 delta 的帧序列（每帧一条，交给 delay 版 mock 逐帧喂）。 */
function multiChunkFrames(count: number): string[] {
  const frames = [
    `data: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
  ];
  for (let i = 0; i < count; i += 1) {
    const n = String(i).padStart(2, "0");
    // 每帧都写得足够长：内容撑不满视口时根本没有可滚动距离，
    // 「是否贴底」「是否滚离」都失去意义（那会让断言假绿/假红）。
    const filler = "padding for scroll follow growth ".repeat(6);
    frames.push(`data: STREAM-E2E-LINE-${n} ${filler}\n\n`);
  }
  frames.push(`data: {"type":"RUN_FINISHED"}\n\n`);
  return frames;
}

test("已有消息的会话里发送新消息会滚到底", async ({ page }) => {
  await seedScrollable(page, "s_e2e_scroll_send", 60);

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: Hello from E2E scroll test.\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  // 先滚离底部。**必须用真实滚轮**：产品刻意不用「scrollTop 下降」判上滑
  // （见 `scroll_shell::on_messages_stick_scroll_event` 的注释 —— DOM 重写会误关跟底），
  // 解除贴底只有「向上滚轮」与「指针拖离」两条路。所以程序化 `scrollTo(0, 0)` 并不解除
  // 贴底：水合稍慢时下一次 rAF 就把视图 snap 回底部，断言 `scrollTop === 0` 会一直读到底部
  // 位置（CI 曾因此偶发红，15s 轮询恒为非零）。
  const box = await page.locator(SCROLLER).boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.wheel(0, -600);
  // 断言「已脱离底部」而不是「滚到顶」：滚轮的滚动量取决于内容高度，钉死 scrollTop
  // 等于把视口高度也钉死，换个字体/行高就假红。语义要的正是「离底超过阈值」。
  await expect.poll(() => scrollGap(page)).toBeGreaterThan(FOLLOW_GAP_MAX_PX);

  await sendMessage(page, "e2e scroll send test");

  await expect(
    page.getByText("Hello from E2E scroll test.").first(),
  ).toBeVisible();
  await expect.poll(() => atBottom(page)).toBe(true);
});

test("空会话发送首条消息也会滚到底", async ({ page }) => {
  await seedSession(page, "s_e2e_empty");

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: First message reply.\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  await sendMessage(page, "first message");

  await expect(page.getByText("First message reply.").first()).toBeVisible();
  await expect.poll(() => atBottom(page)).toBe(true);
});

test("流式增长时始终保持在跟读阈值内", async ({ page }) => {
  await seedSession(page, "s_e2e_stream_follow");

  await installDelayedMockSse(page, multiChunkFrames(28), 35);

  await sendMessage(page, "stream follow e2e");

  await expect(page.getByText(/STREAM-E2E-LINE-00/).first()).toBeVisible();

  // 流式过程中反复采样：距底不得超过跟读阈值
  let sawLast = false;
  for (let i = 0; i < 120 && !sawLast; i += 1) {
    const gap = await scrollGap(page);
    expect(gap, `流式第 ${i} 次采样距底超阈值`).toBeLessThanOrEqual(
      FOLLOW_GAP_MAX_PX,
    );
    sawLast = await page.evaluate(() =>
      document.body.innerText.includes("STREAM-E2E-LINE-27"),
    );
    if (!sawLast) await page.waitForTimeout(80);
  }
  expect(sawLast, "应等到最后一段 delta").toBe(true);

  await expect.poll(() => atBottom(page)).toBe(true);
});

test("流式中上滚后不再强制跟底", async ({ page }) => {
  await seedSession(page, "s_e2e_stream_wheel");

  // 放慢每帧，留出「上滚 → 后续 delta 继续来」的时间窗
  await installDelayedMockSse(page, multiChunkFrames(28), 120);

  await sendMessage(page, "wheel disengage e2e");

  await expect(page.getByText(/STREAM-E2E-LINE-05/).first()).toBeVisible();

  // 流式中上滚
  await page.evaluate((sel) => {
    (document.querySelector(sel) as HTMLElement | null)?.dispatchEvent(
      new WheelEvent("wheel", {
        deltaY: -160,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, SCROLLER);
  await page.waitForTimeout(300);

  await expect(page.getByText(/STREAM-E2E-LINE-20/).first()).toBeVisible();

  const gap = await scrollGap(page);
  expect(gap, "上滚后应与 live edge 脱开（不再强制跟底）").toBeGreaterThan(
    FOLLOW_GAP_MAX_PX,
  );
});
