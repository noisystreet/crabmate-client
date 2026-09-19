import { expect, test } from "@playwright/test";
import {
  installDelayedMockSse,
  seedSession,
  sendMessage,
} from "../fixtures/helpers";

// 复现探针（PR #139 rAF 合帧 + WebKitGTK 失焦假设）：
// 桌面壳失焦时 WebKitGTK 不派发 requestAnimationFrame，合帧 Effect 的
// sync_scheduled 标志在 rAF 回调里才复位 → 排队的 DOM 同步永不 flush，
// 表现为「消息气泡很久不出现」。此处用可挂起的 rAF 替身模拟失焦窗口。
//
// 注意：本用例固化的是「修复前」行为——失焦自愈修复落地后，停摆窗口内
// 应由兜底 Timeout flush，第 5 步断言需反转为回归测试（停摆期间气泡也出现）。

const SID = "e2e-raf-stall-repro";

interface RafTestHook {
  suspend(): void;
  resume(): void;
  pendingCount(): number;
}

test("rAF 停摆期间气泡不出现，恢复 rAF 后一次性追上", async ({ page }) => {
  // 1. 注入可挂起的 rAF 替身（页面加载前生效；初始放行，应用正常启动）
  await page.addInitScript(() => {
    const original = window.requestAnimationFrame.bind(window);
    const originalCancel = window.cancelAnimationFrame.bind(window);
    const pending: { id: number; cb: FrameRequestCallback }[] = [];
    let suspended = false;
    let nextId = 1;
    const w = window as unknown as {
      __cmRafTest: {
        suspend(): void;
        resume(): void;
        pendingCount(): number;
      };
    };
    w.__cmRafTest = {
      suspend() {
        suspended = true;
      },
      // 模拟重新聚焦：积压回调在下一真实帧按序补发，此后放行
      resume() {
        suspended = false;
        const queued = pending.splice(0, pending.length);
        for (const { cb } of queued) original(cb);
      },
      pendingCount() {
        return pending.length;
      },
    };
    window.requestAnimationFrame = ((cb: FrameRequestCallback): number => {
      if (suspended) {
        const id = nextId++;
        pending.push({ id, cb });
        return id;
      }
      return original(cb);
    }) as typeof window.requestAnimationFrame;
    window.cancelAnimationFrame = ((handle: number): void => {
      const idx = pending.findIndex((e) => e.id === handle);
      if (idx >= 0) {
        pending.splice(idx, 1);
        return;
      }
      originalCancel(handle);
    }) as typeof window.cancelAnimationFrame;
  });

  // 2. 正常启动 + seed
  await seedSession(page, SID);
  await expect(page.getByTestId("chat-tui-stream-view")).toBeVisible();

  // 3. mock SSE：4 个事件各延迟 120ms，数据会在停摆窗口内全部到达
  await installDelayedMockSse(
    page,
    [
      'id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n',
      `id: 2\ndata: ${JSON.stringify({ type: "TEXT_MESSAGE_CONTENT", delta: "**第一段" })}\n\n`,
      `id: 3\ndata: ${JSON.stringify({ type: "TEXT_MESSAGE_CONTENT", delta: "，第二段**" })}\n\n`,
      'id: 4\ndata: {"type":"RUN_FINISHED"}\n\n',
    ],
    120,
  );

  // 4. 挂起 rAF（模拟失焦），发送消息
  await page.evaluate(() =>
    (window as unknown as { __cmRafTest: RafTestHook }).__cmRafTest.suspend(),
  );
  await sendMessage(page, "raf 停摆复现");

  // 5. 停摆窗口：SSE 早已收完（4×120ms≈0.5s），等 3 秒后气泡仍应缺席
  await page.waitForTimeout(3000);
  const bodyText = await page.evaluate(() => document.body.innerText);
  expect(
    bodyText,
    "rAF 停摆 3 秒后助手气泡仍未出现（复现「消息气泡很久不出现」）",
  ).not.toContain("第一段");

  // 机制证据：合帧 rAF 回调确实被卡在队列里（sync_scheduled 未复位）
  const queued = await page.evaluate(() =>
    (
      window as unknown as { __cmRafTest: RafTestHook }
    ).__cmRafTest.pendingCount(),
  );
  expect(queued, "停摆期间应有合帧回调滞留在 rAF 队列").toBeGreaterThan(0);

  // 6. 恢复 rAF（模拟重新聚焦）：积压同步一次性 flush，finalize 粗体生效
  await page.evaluate(() =>
    (window as unknown as { __cmRafTest: RafTestHook }).__cmRafTest.resume(),
  );
  const transcript = page.getByTestId("chat-tui-transcript");
  await expect(transcript).toContainText("第一段", { timeout: 5000 });
  await expect(transcript).toContainText("raf 停摆复现");
  await expect(transcript.locator("strong")).toHaveText("第一段，第二段", {
    timeout: 10_000,
  });
});
