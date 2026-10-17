/**
 * 回归：本回合以「声明 tool_call 后即结束」（无最终正文）收尾时，
 * 主列不得补「(无回复：未进入正文阶段)」诊断。
 *
 * 背景：模型先流正文 → 随后声明 tool_call 时，客户端 `demote_answer_before_tools`
 * 把已流正文降级为 commentary 旁注并清零 `answer_delta_chars`；服务端因本轮确有
 * 可见输出（ToolCalls）以 `Completed` 收尾。旧实现仅凭
 * `answer_delta_chars == 0 && !saw_final_response_timeline` 就补诊断，与
 * 「Completed ⇒ 本轮必有可见输出」矛盾，于是出现「有工具卡却报无回复」。
 *
 * 时序（与 serve 一致；工具后**没有**最终正文）：
 *   assistant_answer_phase → 旁白 delta → turn_segment_end →
 *   parsing_tool_calls → tool_running → TOOL_CALL_START → TOOL_CALL_RESULT →
 *   turn_tool_phase_end → RUN_FINISHED
 *
 * 注：同目录既有 mock 用例（`mock-commentary-before-tool-order` 等）都在工具后补了
 * 最终正文，不覆盖本形态。
 *
 * 断言：
 *   1. 工具卡可见；
 *   2. transcript 不出现「未进入正文阶段」/「无回复」；
 *   3. 状态栏回到「就绪」。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-tool-only-turn-no-empty-diagnostic.spec.ts
 */
import { expect, test } from "@playwright/test";
import { installMockSse, seedSession, sendMessage } from "../fixtures/helpers";

const CONV_ID = "e2e-tool-only-turn-no-empty-diagnostic";
const COMMENTARY = "我先看一下工作区当前的结构。";
const TOOL_NAME = "list_dir";
const TOOL_SUMMARY = "列出目录";
const TOOL_CALL_ID = "tc-list";
/** 诊断占位文案（ZhHans）——修复后不得出现。 */
const EMPTY_REPLY_DIAGNOSTIC = "未进入正文阶段";
const EMPTY_REPLY_PREFIX = "无回复";

/** tool_call 收尾、工具后直接 RUN_FINISHED（无终答正文）。 */
function buildToolOnlyTurnSse(): string {
  let id = 1;
  const next = (payload: string) => {
    const line = `id: ${id}\ndata: ${payload}\n\n`;
    id += 1;
    return line;
  };
  return [
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "assistant_answer_phase",
      }),
    ),
    next(COMMENTARY),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_segment_end",
        data: { segmentId: "seg-before-list" },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "parsing_tool_calls",
        data: { parsing: true },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "tool_running",
        data: { running: true },
      }),
    ),
    next(
      JSON.stringify({
        type: "TOOL_CALL_START",
        toolCallId: TOOL_CALL_ID,
        name: TOOL_NAME,
        summary: TOOL_SUMMARY,
      }),
    ),
    next(
      JSON.stringify({
        type: "TOOL_CALL_RESULT",
        toolCallId: TOOL_CALL_ID,
        content: ".",
        metadata: { name: TOOL_NAME, ok: true, summary: TOOL_SUMMARY },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_tool_phase_end",
        data: { phase: "tool_end" },
      }),
    ),
    next(JSON.stringify({ type: "RUN_FINISHED", threadId: "", runId: "1" })),
  ].join("");
}

test.describe("tool-only turn must not fill empty-reply diagnostic", () => {
  test("工具后无终答时只展示工具卡，不补「无回复」诊断", async ({ page }) => {
    await installMockSse(page, buildToolOnlyTurnSse(), CONV_ID);
    await seedSession(page, `s_e2e_tool_only_${Date.now()}`);
    await sendMessage(page, "看一下工作区");

    await expect(page.locator('[data-testid="status-bar"]')).toContainText(
      "就绪",
      { timeout: 45_000 },
    );

    const scroller = page.locator('[data-testid="chat-messages-scroller"]');

    // 1. 本回合确有可见输出：工具卡 + 工具前旁白。
    await expect(page.locator("section.chat-tui-turn--tool")).toHaveCount(1);
    await expect(scroller).toContainText(TOOL_SUMMARY, { timeout: 10_000 });
    await expect(scroller).toContainText(COMMENTARY, { timeout: 10_000 });

    // 2. 不得补「无回复」诊断（与 Server `Completed ⇒ 本轮有可见输出` 矛盾）。
    const text = (await scroller.textContent()) ?? "";
    expect(
      text.includes(EMPTY_REPLY_DIAGNOSTIC) ||
        text.includes(EMPTY_REPLY_PREFIX),
      `工具卡已在场，却出现了空回复诊断占位：\n${text}`,
    ).toBe(false);
  });
});
