/**
 * 真实 LLM 端到端测试：无工具问答场景
 *
 * 覆盖 PR #678 修复的终答气泡（FINAL_ANSWER_ROW）可见性：
 *   - 流完成后终答正文在 UI 中可见
 *   - 会话消息持久化包含 assistant 终答
 *
 * 前置条件：
 *   1. `crabmate serve` 在 127.0.0.1:8080 运行
 *   2. 模型密钥：环境变量 `API_KEY` / 本地 TOML，**或**本机钥匙串/E2E 注入已有 `client_llm` 密钥
 *   3. 若服务端启用了 Web Bearer：`CM_WEB_API_BEARER_TOKEN`（与浏览器设置同源）
 *
 * 运行方式：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-zero-tool.spec.ts
 */

import { test, expect } from "@playwright/test";
import {
  apiUrl,
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
  waitForReadyWhileApproving,
} from "../fixtures/helpers";

const API_KEY = resolveOptionalApiKeyFromEnvOrToml();
const SID = "s_e2e_real_zero_tool";

test.describe("真实 LLM：无工具终答场景", () => {
  test("流完成后终答正文在 UI 中可见", async ({ page }) => {
    await setupRealLLMSessionPreferringKeyring(page, SID, API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    await sendMessage(page, "你有哪些核心功能？");

    await waitForReadyWhileApproving(page, 180_000);

    await expect(page.locator('[data-testid="status-bar"]')).toContainText(
      "就绪",
      { timeout: 5_000 },
    );

    await expect(
      page.locator('[data-testid="chat-messages-scroller"]'),
    ).not.toBeEmpty({ timeout: 5_000 });

    const errorToasts = await page
      .locator('[data-testid="error-toast"]')
      .count();
    expect(errorToasts).toBe(0);
  });

  test("会话消息持久化包含 assistant 终答", async ({ page }) => {
    await setupRealLLMSessionPreferringKeyring(page, SID + "_persist", API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    await sendMessage(page, "列举三个你可以做的事情");

    await waitForReadyWhileApproving(page, 180_000);

    let messages: unknown[] = [];
    const pollTimeout = 30_000;
    const pollInterval = 500;
    for (let elapsed = 0; elapsed < pollTimeout; elapsed += pollInterval) {
      const fetched: unknown[] = await page.evaluate(
        ({ url, sid }: { url: string; sid: string }) =>
          fetch(url)
            .then((r) => r.json())
            .then((d) => {
              const list = d.current?.sessions || d.sessions || [];
              const s = Array.isArray(list)
                ? list.find((x: { id: string }) => x.id === sid)
                : null;
              return s ? s.messages || [] : [];
            }),
        {
          url: apiUrl("/user-data/workspaces/current/sessions"),
          sid: SID + "_persist",
        },
      );
      const hasAssistantText = (
        fetched as Array<{ role: string; text: string }>
      ).some((m) => m.role === "assistant" && (m.text || "").trim().length > 0);
      if (hasAssistantText) {
        messages = fetched;
        break;
      }
      await new Promise((r) => setTimeout(r, pollInterval));
    }

    const assistantMessages = (
      messages as Array<{ role: string; is_tool: boolean; text: string }>
    ).filter(
      (m) =>
        m.role === "assistant" &&
        !m.is_tool &&
        (m.text || "").trim().length > 0,
    );
    expect(assistantMessages.length).toBeGreaterThanOrEqual(1);

    const finalText = assistantMessages.map((m) => m.text).join("");
    expect(finalText.length).toBeGreaterThan(10);
  });
});
