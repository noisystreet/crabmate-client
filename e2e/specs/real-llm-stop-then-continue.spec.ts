/**
 * 真实 LLM 端到端复现：中断对话后再继续拿不到回复
 *
 * 病灶来源（`chat_export_20261017_091401.md`）：
 *   某回合助手输出到一半被「[已停止]」→ 随后用户重复同一问题 2 次 +「继续」3 次，
 *   连续 5 个回合全部落为
 *     `(无回复：未进入正文阶段)` + `诊断：stream_ended=完成, answer_phase=false, delta_chars=0`
 *
 * 本用例只做「复现」，不改产品代码：先等真实流式已开始 → 点「停止」中断 →
 * 再发一回合「继续」→ 断言新回合必须拿到非空正文。
 *
 * 预期结果：**红**（复现成功）。修好客户端流收口语义（无终结帧不得合成「完成」并放弃 resume）
 * 之后应转为绿。
 *
 * 前置条件：
 *   1. `crabmate serve` 可用（`scripts/e2e-playwright.sh` 会自动拉起）
 *   2. 模型密钥：环境变量 `API_KEY` / 本地 TOML，或本机钥匙串 / E2E 注入已有 `client_llm` 密钥
 *
 * 运行方式（本机 8080 常被真实 serve 占用，改用 8099/4199）：
 *   CRABMATE_PORT=8099 CRABMATE_WEB_PORT=4199 \
 *     ./scripts/e2e-playwright.sh specs/real-llm-stop-then-continue.spec.ts
 *
 * `real-llm-*` 不在 CI（需真实密钥）；如需直跑：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-stop-then-continue.spec.ts
 */

import { test, expect, type Page } from "@playwright/test";
import {
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

/** 必然长时间流式的提示：逐行输出大量数字，保证「停止」有稳定可点的窗口。 */
const LONG_STREAM_PROMPT =
  "请从 1 数到 800，每个数字独占一行，不要省略任何数字，也不要添加任何解释、标点或代码块标记。";

/** 会话内可见文本快照（失败时打印，便于定位）。 */
async function scrollerText(page: Page): Promise<string> {
  return (
    (await page
      .locator(SCROLLER)
      .innerText()
      .catch(() => "")) ?? ""
  );
}

/** 等待助手正文开始增长（证明流已经进入正文阶段，而非只在握手 / 排队）。 */
async function waitForStreamGrowth(
  page: Page,
  baseline: number,
): Promise<void> {
  await page.waitForFunction(
    ({ sel, min }: { sel: string; min: number }) => {
      const el = document.querySelector(sel);
      return !!el && (el.textContent ?? "").length > min;
    },
    { sel: SCROLLER, min: baseline + 8 },
    { timeout: 120_000 },
  );
}

test.describe("真实 LLM：中断后继续必须能拿到回复", () => {
  test("停止中断一个回合后，下一回合仍应产出非空正文", async ({ page }) => {
    test.setTimeout(300_000);

    const sid = `s_e2e_stop_continue_${Date.now()}`;
    await setupRealLLMSessionPreferringKeyring(page, sid, API_KEY);
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    // --- 第 1 回合：长流式 → 中途点「停止」 ---
    await sendMessage(page, LONG_STREAM_PROMPT);

    const stopButton = page.locator(STOP_BUTTON);
    await expect(stopButton).toBeVisible({ timeout: 60_000 });

    const baseline = (await scrollerText(page)).length;
    await waitForStreamGrowth(page, baseline);

    await stopButton.click();
    await expect(stopButton).toHaveCount(0, { timeout: 60_000 });

    const afterStop = await scrollerText(page);
    expect(
      afterStop.includes("已停止"),
      `点停止后应出现中断收口文案。当前会话文本：\n${afterStop}`,
    ).toBe(true);

    // --- 第 2 回合：继续 —— 本轮必须拿到非空正文 ---
    const assistantTurns = page.locator("section.chat-tui-turn--assistant");
    const turnsBeforeContinue = await assistantTurns.count();
    const lastTurnBeforeContinue = (
      await assistantTurns.last().innerText()
    ).trim();

    await sendMessage(page, "继续");

    // 先等「忙碌」真的出现，否则 waitForReadyWhileApproving 会因状态栏仍显示
    // 上一次的「就绪」而立即返回，导致断言在流开始前就通过（假绿）。
    await expect(stopButton).toBeVisible({ timeout: 60_000 });
    await waitForReadyWhileApproving(page, 180_000);

    await expect(page.locator(STATUS_BAR)).toContainText("就绪", {
      timeout: 5_000,
    });

    const transcript = await scrollerText(page);
    expect(
      transcript.includes("无回复") || transcript.includes("未进入正文阶段"),
      `中断后继续的回合不应再出现「无回复 / 未进入正文阶段」诊断。` +
        `当前会话文本：\n${transcript}`,
    ).toBe(false);

    const turnsAfterContinue = await assistantTurns.count();
    expect(
      turnsAfterContinue,
      `中断后继续应至少留下 2 个助手回合（中断的 + 继续的）。当前会话文本：\n${transcript}`,
    ).toBeGreaterThanOrEqual(2);

    const lastTurnText = (await assistantTurns.last().innerText()).trim();
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
      /[0-9]/.test(lastTurnText),
      `「继续」回合的助手正文应是真实内容（期望含数字）。末条回合文本：\n${lastTurnText}`,
    ).toBe(true);
    expect(
      lastTurnText.includes("已停止"),
      `「继续」回合不应仍是中断收口文案。末条回合文本：\n${lastTurnText}`,
    ).toBe(false);

    const errorToasts = await page
      .locator('[data-testid="error-toast"]')
      .count();
    expect(errorToasts).toBe(0);
  });
});
