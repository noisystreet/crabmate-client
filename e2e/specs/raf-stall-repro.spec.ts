import { expect, test } from "@playwright/test";
import {
  installDelayedMockSse,
  seedSession,
  sendMessage,
} from "../fixtures/helpers";

// 回归测试（失焦自愈已落地，见 frontend/src/app/chat/tui_stream_dom_sync.rs）：
// 桌面壳失焦时 WebKitGTK 不派发 requestAnimationFrame，合帧 Effect 的
// sync_scheduled 标志原本只在 rAF 回调里复位 → 排队的 DOM 同步永不 flush，
// 表现为「消息气泡很久不出现」。修复后同步路径安排自愈 Timeout（setTimeout
// 失焦仍触发）：到期若 rAF 未执行则复位标志并直接兜底同步。此处用可挂起的
// rAF 替身模拟失焦窗口，断言停摆期间气泡由自愈兜底正常出现。

const SID = "e2e-raf-stall-repro";

interface RafTestHook {
  suspend(): void;
  resume(): void;
  pendingCount(): number;
}

test("rAF 停摆期间气泡由失焦自愈兜底出现，恢复 rAF 后一致", async ({
  page,
}) => {
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

  // 5. 停摆窗口：SSE 早已收完（4×120ms≈0.5s），自愈 Timeout（200ms）应已兜底
  // flush——气泡在停摆期间就必须出现（回归断言：修复前会卡到 resume 才出现）
  await page.waitForTimeout(3000);
  const transcriptStalled = page.getByTestId("chat-tui-transcript");
  await expect(
    transcriptStalled,
    "rAF 停摆 3 秒后助手气泡应已由失焦自愈兜底出现",
  ).toContainText("第一段");

  // 机制证据：合帧 rAF 回调确实曾滞留队列（说明停摆窗口真实存在，
  // 气泡出现靠的是自愈 Timeout 而非 rAF 恢复）
  const queued = await page.evaluate(() =>
    (
      window as unknown as { __cmRafTest: RafTestHook }
    ).__cmRafTest.pendingCount(),
  );
  expect(queued, "停摆期间应有合帧回调滞留在 rAF 队列").toBeGreaterThan(0);

  // 6. 恢复 rAF（模拟重新聚焦）：积压同步幂等补跑，finalize 粗体保持生效
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
