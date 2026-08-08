/**
 * 真实 LLM E2E：连续三轮上下文对话，并验证每轮完成后保持在最新消息末尾。
 *
 * 运行：
 *   REAL_LLM_E2E=1 no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-three-turn-scroll.spec.ts
 *
 * 模型密钥：`API_KEY` / TOML，或服务端钥匙串 `client_llm`。
 * Web Bearer：启用时设 `CM_WEB_API_BEARER_TOKEN`。
 * 未设 `REAL_LLM_E2E=1` 时跳过，避免 CI 意外产生真实调用费用。
 */

import { expect, Page, test } from "@playwright/test";
import {
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
} from "../fixtures/helpers";

const PROMPTS = ["你好", "你有哪些技能", "还有吗？"] as const;
const SID = `s_e2e_real_three_turn_${Date.now()}`;
const API_KEY = resolveOptionalApiKeyFromEnvOrToml();
const ENABLED = process.env.REAL_LLM_E2E === "1";

async function scrollGapPx(page: Page): Promise<number> {
  return page.evaluate(() => {
    const scroller = document.querySelector(
      '[data-testid="chat-messages-scroller"]',
    );
    if (!scroller) return -1;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
  });
}

async function waitForTurnComplete(
  page: Page,
  assistantCountBefore: number,
  timeoutMs = 180_000,
) {
  const deadline = Date.now() + timeoutMs;
  const statusBar = page.locator('[data-testid="status-bar"]');
  const approvalModal = page.locator('[data-testid="approval-modal"]');
  while (Date.now() < deadline) {
    if (await approvalModal.isVisible()) {
      await page.locator('[data-testid="approval-allow-always"]').click();
      continue;
    }
    const ready = (await statusBar.textContent())?.includes("就绪");
    const assistantCount = await page
      .locator("section.chat-tui-turn--assistant")
      .count();
    if (ready && assistantCount > assistantCountBefore) return;
    await page.waitForTimeout(250);
  }
  throw new Error(`真实 LLM 回合在 ${timeoutMs}ms 内未完成`);
}

test.describe("真实 LLM：三轮对话滚动跟随", () => {
  const runTest = ENABLED ? test : test.skip;

  runTest(
    "你好 → 技能 → 追问，每轮完成后均停在最新消息末尾",
    async ({ page }) => {
      test.setTimeout(600_000);
      await setupRealLLMSessionPreferringKeyring(page, SID, API_KEY);
      if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
        test.skip(
          true,
          "未设置 API_KEY 且服务端钥匙串无 client_llm，跳过真实 LLM 用例",
        );
        return;
      }

      for (const prompt of PROMPTS) {
        const assistantCountBefore = await page
          .locator("section.chat-tui-turn--assistant")
          .count();
        await sendMessage(page, prompt);
        await waitForTurnComplete(page, assistantCountBefore);
        await expect
          .poll(() => scrollGapPx(page), { timeout: 10_000 })
          .toBeLessThanOrEqual(4);
      }

      await expect(page.locator("section.chat-tui-turn--user")).toHaveCount(
        PROMPTS.length,
      );
      expect(
        await page.locator("section.chat-tui-turn--assistant").count(),
      ).toBeGreaterThanOrEqual(PROMPTS.length);
      for (const prompt of PROMPTS) {
        await expect(
          page.locator('[data-testid="chat-messages-scroller"]'),
        ).toContainText(prompt);
      }
      expect(await page.locator('[data-testid="error-toast"]').count()).toBe(0);
    },
  );
});
