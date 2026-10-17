/**
 * 真实 LLM 端到端复现：长工具执行中点「停止」后再「继续」
 *
 * 与 `real-llm-stop-then-continue.spec.ts`（纯文本流式中断）互补：本用例让模型
 * 同步执行一个长命令（`sleep`），在**工具卡处于运行态（⏳）**时点「停止」，
 * 然后立刻发「继续」，断言不得落为零回复。
 *
 * 病灶来源（`chat_export_20261017_091401.md`）：
 *   某回合助手输出到一半被「[已停止]」→ 随后用户重复同一问题 2 次 +「继续」3 次，
 *   连续 5 个回合全部落为
 *     `(无回复：未进入正文阶段)` + `诊断：stream_ended=完成, answer_phase=false, delta_chars=0`
 *
 * 本用例只做「复现」，不改产品代码。
 * 实测（2026-10-17，deepseek-v4-flash）：**绿**。工具运行中点停止后服务端日志出现
 * `subprocess session kill ... reason=cancel`（`sleep` 子进程被终止），随后「继续」
 * 能落出新的助手正文，未出现「未进入正文阶段」诊断。若将来变红即为回归。
 *
 * 前置条件：
 *   1. `crabmate serve` 可用（`scripts/e2e-playwright.sh` 会自动拉起）
 *   2. 模型密钥：环境变量 `API_KEY` / 本地 TOML，或本机钥匙串 / E2E 注入已有 `client_llm` 密钥
 *
 * 运行方式（本机 8080 常被真实 serve 占用，改用 8099/4199）：
 *   CRABMATE_PORT=8099 CRABMATE_WEB_PORT=4199 \
 *     ./scripts/e2e-playwright.sh specs/real-llm-stop-during-long-tool.spec.ts
 *
 * `real-llm-*` 不在 CI（需真实密钥）；如需直跑：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-stop-during-long-tool.spec.ts
 */

import { test, expect, type Page } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import {
  apiUrl,
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
  waitForReadyWhileApproving,
} from "../fixtures/helpers";

const API_KEY = resolveOptionalApiKeyFromEnvOrToml();

const SCROLLER = '[data-testid="chat-messages-scroller"]';
const STATUS_BAR = '[data-testid="status-bar"]';
const STOP_BUTTON = '[data-testid="chat-stop-button"]';
const TOOL_PROCESS = '[data-testid="chat-tui-tool-process"]';
const TOOL_STATUS = `${TOOL_PROCESS} .chat-tui-tool-status`;
const TOOL_NAME = `${TOOL_PROCESS} .chat-tui-tool-name`;
const APPROVAL_MODAL = '[data-testid="approval-modal"]';

/** 强约束：同步执行长命令，确保工具卡有稳定的运行态窗口可中断。 */
const LONG_TOOL_PROMPT =
  "请调用 run_command 工具执行命令 sleep 120。" +
  '参数示例：{"command":"sleep","args":["120"]}。' +
  "必须同步执行，绝对不要设置 async=true，也不要用后台任务。" +
  "等命令结束后告诉我它何时结束。";

/** 会话内可见文本快照（失败时打印，便于定位）。 */
async function scrollerText(page: Page): Promise<string> {
  return (
    (await page
      .locator(SCROLLER)
      .innerText()
      .catch(() => "")) ?? ""
  );
}

/**
 * `run_command` 未设置工作区会被直接拒绝（`workspace_not_set`）。
 * 与 `real-llm-tool-call.spec.ts` 一致：自建临时目录并绑定为当前工作区。
 */
async function ensureTempWorkspace(page: Page): Promise<string> {
  const wsDir = path.resolve(
    process.cwd(),
    "..",
    `.e2e_tmp_stop_long_tool_${Date.now()}`,
  );
  fs.mkdirSync(wsDir, { recursive: true });

  const result = await page.evaluate(
    async ({ url, dir }: { url: string; dir: string }) => {
      const response = await fetch(url, {
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
    },
    { url: apiUrl("/workspace"), dir: wsDir },
  );

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

/** 弹审批时点「始终允许」，与 `waitForReadyWhileApproving` 的选择一致。 */
async function approveIfVisible(page: Page): Promise<boolean> {
  const modal = page.locator(APPROVAL_MODAL);
  if (!(await modal.isVisible().catch(() => false))) return false;
  const allowAlways = page.locator('[data-testid="approval-allow-always"]');
  if (await allowAlways.isVisible().catch(() => false)) {
    await allowAlways.click();
  } else {
    await page.locator('[data-testid="approval-allow-once"]').click();
  }
  await expect(modal).not.toBeVisible({ timeout: 10_000 });
  return true;
}

/**
 * 等待某张 `run_command` 工具卡进入运行态（⏳），期间自动放行审批。
 *
 * 必须同时满足「审批弹窗不可见」，否则停在审批阶段的卡也会显示 ⏳，
 * 点停止就退化成「审批中断」而不是「工具执行中断」。
 */
async function waitForRunCommandRunning(
  page: Page,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastSnapshot = "";
  while (Date.now() < deadline) {
    if (await approveIfVisible(page)) continue;

    const statuses = page.locator(TOOL_STATUS);
    const names = page.locator(TOOL_NAME);
    const count = Math.min(await statuses.count(), await names.count());
    const parts: string[] = [];
    for (let i = 0; i < count; i += 1) {
      const status = ((await statuses.nth(i).innerText()) ?? "").trim();
      const aria =
        (await statuses
          .nth(i)
          .getAttribute("aria-label")
          .catch(() => "")) ?? "";
      // `.chat-tui-tool-name` 的可见文本是本地化人类名（「命令执行」），
      // 工具 id（`run_command`）在 `title` 属性上。
      const name = ((await names.nth(i).innerText()) ?? "").trim();
      const toolId =
        (await names
          .nth(i)
          .getAttribute("title")
          .catch(() => "")) ?? "";
      parts.push(`${toolId || name}:${status}${aria ? `(${aria})` : ""}`);
      if (
        (toolId.includes("run_command") || name.includes("run_command")) &&
        (status.includes("⏳") || aria.includes("执行中"))
      ) {
        return;
      }
    }
    lastSnapshot = parts.join(" | ");
    await page.waitForTimeout(250);
  }
  const transcript = await scrollerText(page);
  throw new Error(
    `${timeoutMs}ms 内未观察到 run_command 工具卡运行态（⏳）。` +
      `最后工具状态：${lastSnapshot || "(无)"}\n当前会话文本：\n${transcript}`,
  );
}

test.describe("真实 LLM：长工具执行中停止后继续必须能拿到回复", () => {
  test("工具运行中停止一个回合后，下一回合仍应产出非空正文", async ({
    page,
  }) => {
    // 第 1 回合等模型决定 + 审批；第 2 回合可能再次执行长命令（受服务端默认
    // 30s 命令超时约束）后给出终答。留足余量。
    test.setTimeout(420_000);

    const sid = `s_e2e_stop_long_tool_${Date.now()}`;
    await setupRealLLMSessionPreferringKeyring(page, sid, API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    // --- 第 1 回合：长工具执行中 → 点「停止」 ---
    const wsDir = await ensureTempWorkspace(page);
    try {
      await sendMessage(page, LONG_TOOL_PROMPT);

      await waitForRunCommandRunning(page, 120_000);

      const stopButton = page.locator(STOP_BUTTON);
      await expect(stopButton, "工具运行中应可点击停止按钮").toBeVisible({
        timeout: 30_000,
      });
      await stopButton.click();
      await expect(stopButton).toHaveCount(0, { timeout: 60_000 });

      const afterStop = await scrollerText(page);
      expect(
        afterStop.includes("已停止"),
        `工具执行中点停止后应出现中断收口文案。当前会话文本：\n${afterStop}`,
      ).toBe(true);

      // --- 第 2 回合：继续 —— 本轮必须拿到非空正文 ---
      const assistantTurns = page.locator("section.chat-tui-turn--assistant");
      const turnsBeforeContinue = await assistantTurns.count();
      const lastTurnBeforeContinue = (
        await assistantTurns
          .last()
          .innerText()
          .catch(() => "")
      ).trim();

      await sendMessage(page, "继续");

      // 先确认忙碌真的出现，否则 waitForReadyWhileApproving 会因状态栏仍显示
      // 上一次的「就绪」而立即返回，导致断言在流开始前就通过（假绿）。
      await expect(stopButton).toBeVisible({ timeout: 60_000 });
      await waitForReadyWhileApproving(page, 300_000);

      await expect(page.locator(STATUS_BAR)).toContainText("就绪", {
        timeout: 5_000,
      });

      const transcript = await scrollerText(page);
      // 只匹配诊断文案本身：助手正文可能引用提交信息里的「无回复」字样（本轮实测
      // 模型会去 `git log`），用宽泛的 `includes("无回复")` 会误报。
      expect(
        transcript.includes("未进入正文阶段"),
        `工具运行中停止后继续的回合不应落为「无回复：未进入正文阶段」诊断。` +
          `当前会话文本：\n${transcript}`,
      ).toBe(false);

      const turnsAfterContinue = await assistantTurns.count();
      const lastTurnText = (
        await assistantTurns
          .last()
          .innerText()
          .catch(() => "")
      ).trim();
      expect(
        turnsAfterContinue > turnsBeforeContinue ||
          lastTurnText !== lastTurnBeforeContinue,
        `「继续」必须产出新的助手回合（不能只是沿用中断回合的旧文本）。` +
          `继续前末条：\n${lastTurnBeforeContinue}\n继续后末条：\n${lastTurnText}`,
      ).toBe(true);
      expect(
        lastTurnText.length,
        `「继续」回合的助手正文不应为空。末条回合文本：\n${lastTurnText}`,
      ).toBeGreaterThan(5);
      expect(
        lastTurnText.includes("已停止"),
        `「继续」回合不应仍是中断收口文案。末条回合文本：\n${lastTurnText}`,
      ).toBe(false);

      const errorToasts = await page
        .locator('[data-testid="error-toast"]')
        .count();
      expect(errorToasts).toBe(0);
    } finally {
      fs.rmSync(wsDir, { recursive: true, force: true });
    }
  });
});
