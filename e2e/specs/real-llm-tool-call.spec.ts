/**
 * 真实 LLM 端到端测试：工具调用场景
 *
 * 前置：
 *   1. `crabmate serve` 在 127.0.0.1:8080
 *   2. `API_KEY` / TOML，或本机钥匙串/E2E 注入已有 `client_llm` 密钥
 *   3. 启用 Web Bearer 时设 `CM_WEB_API_BEARER_TOKEN`
 *
 * 运行：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-tool-call.spec.ts
 */

import { test, expect, type Page } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import {
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
  waitForReadyWhileApproving,
} from "../fixtures/helpers";

const API_KEY = resolveOptionalApiKeyFromEnvOrToml();
const SID_BASE = "s_e2e_real_tool_call";

/** 自建临时工作区并绑定到当前服务，避免依赖前序用例留下的失效路径。 */
async function ensureTempWorkspace(page: Page): Promise<string> {
  const wsDir = path.resolve(
    process.cwd(),
    "..",
    `.e2e_tmp_tool_call_${Date.now()}`,
  );
  fs.mkdirSync(wsDir, { recursive: true });
  fs.writeFileSync(
    path.join(wsDir, "README.md"),
    "# e2e tool-call workspace\n",
  );

  const result = await page.evaluate(async (dir: string) => {
    const response = await fetch("/workspace", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: dir }),
    });
    const data = (await response.json().catch(() => ({}))) as {
      path?: unknown;
      error?: unknown;
    };
    return {
      ok: response.ok,
      path: typeof data.path === "string" ? data.path.trim() : "",
      error: typeof data.error === "string" ? data.error.trim() : "",
    };
  }, wsDir);

  if (!result.ok || result.error || !result.path) {
    throw new Error(
      `POST /workspace 失败：${result.error || `HTTP ok=${result.ok}`}`,
    );
  }

  await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });
  return wsDir;
}

test.describe("真实 LLM：工具调用场景", () => {
  const uniqueSid = `${SID_BASE}_${Date.now()}`;
  const uniqueSidPersist = `${SID_BASE}_persist_${Date.now()}`;

  test("工具卡 + 工具结果 + 终答在 UI 中可见", async ({ page }) => {
    test.setTimeout(300_000);
    await setupRealLLMSessionPreferringKeyring(page, uniqueSid, API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }
    const wsDir = await ensureTempWorkspace(page);
    try {
      const putOk = await page.evaluate(async (s: string) => {
        const body = JSON.stringify({
          sessions: [
            {
              id: s,
              title: "e2e-real-tool-call",
              draft: "",
              messages: [],
              updated_at: Date.now(),
              pinned: false,
              starred: false,
            },
          ],
          active_session_id: s,
        });
        const response = await fetch("/user-data/workspaces/current/sessions", {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body,
        });
        return response.ok;
      }, uniqueSid);
      if (!putOk) {
        throw new Error("PUT /user-data/workspaces/current/sessions 失败");
      }
      await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
      await page.waitForSelector('[data-testid="chat-composer-input"]', {
        timeout: 15_000,
      });
      await sendMessage(page, "列出当前工作区目录结构，用列表工具。");

      await waitForReadyWhileApproving(page, 180_000);

      await expect(page.locator('[data-testid="status-bar"]')).toContainText(
        "就绪",
        { timeout: 5_000 },
      );

      const toolCards = await page
        .locator("section.chat-tui-turn--tool")
        .count();
      expect(toolCards).toBeGreaterThanOrEqual(1);

      await expect(
        page.locator('[data-testid="chat-messages-scroller"]'),
      ).not.toBeEmpty({ timeout: 5_000 });

      const errorToasts = await page
        .locator('[data-testid="error-toast"]')
        .count();
      expect(errorToasts).toBe(0);
    } finally {
      fs.rmSync(wsDir, { recursive: true, force: true });
    }
  });

  test("会话消息持久化包含助手终答内容", async ({ page }) => {
    test.setTimeout(300_000);
    await setupRealLLMSessionPreferringKeyring(page, uniqueSidPersist, API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }
    await sendMessage(
      page,
      "现在几点？请用 get_current_time 工具获取当前时间。",
    );

    await waitForReadyWhileApproving(page, 180_000);

    let messages: unknown[] = [];
    const pollTimeout = 30_000;
    const pollInterval = 500;
    for (let elapsed = 0; elapsed < pollTimeout; elapsed += pollInterval) {
      const fetched: unknown[] = await page.evaluate(
        (sid: string) =>
          fetch("/user-data/workspaces/current/sessions")
            .then((r) => r.json())
            .then((d) => {
              const list = d.current?.sessions || d.sessions || [];
              const s = Array.isArray(list)
                ? list.find((x: { id: string }) => x.id === sid)
                : null;
              return s ? s.messages || [] : [];
            }),
        uniqueSidPersist,
      );
      if (fetched.length >= 2) {
        messages = fetched;
        break;
      }
      await new Promise((r) => setTimeout(r, pollInterval));
    }

    const userMessages = (messages as Array<{ role: string }>).filter(
      (m) => m.role === "user",
    );
    expect(userMessages.length).toBeGreaterThanOrEqual(1);

    const assistantMessages = (
      messages as Array<{ role: string; text: string }>
    ).filter((m) => m.role === "assistant" && (m.text || "").trim().length > 0);
    expect(assistantMessages.length).toBeGreaterThanOrEqual(1);

    const finalText = assistantMessages.map((m) => m.text).join("");
    expect(finalText.length).toBeGreaterThan(10);
  });
});
