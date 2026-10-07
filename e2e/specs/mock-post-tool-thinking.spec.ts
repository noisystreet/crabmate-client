/**
 * 复现：工具调用**之后**的第二段思维链（reasoning）在 UI 中**没有**渲染成思考样式。
 *
 * 用户反馈（chat_export_20261007_171605.md）：
 *   「第二次思考在 tauri 界面上没有显示成思考样式」
 *
 * 时序（与 serve 实际发射一致）：
 *   ① 工具前 reasoning（lane 默认 Reasoning → 思考块）
 *   ② assistant_answer_phase（首段正文前）
 *   ③ 工具前旁白（正文相）
 *   ④ parsing_tool_calls → ⑤ TOOL_CALL_START → ⑥ TOOL_CALL_RESULT → ⑦ turn_tool_phase_end
 *   ⑧ turn_segment_end{seg-model-round-1}
 *   ⑨ turn_segment_start{seg-model-round-2, kind:"answer"}（outer loop 非首轮，模型调用**前**）
 *   ⑩ assistant_answer_phase（outer loop 非首轮，模型调用**前**）
 *   ⑪ 工具后 reasoning ← 此时 lane 已被 ⑨/⑩ 推到 Answering，误写正文（BUG）
 *   ⑫ assistant_answer_phase（本轮首段正文前）
 *   ⑬ 终答 → ⑭ RUN_FINISHED
 *
 * 断言：工具后 reasoning 必须出现在 `section.chat-tui-turn--think` 内。
 * 修复前该用例失败（工具后 reasoning 落进普通正文气泡）。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-post-tool-thinking.spec.ts
 */
import { expect, test } from "@playwright/test";
import { seedSession, sendMessage } from "../fixtures/helpers";

const STREAM_DELAY_MS = 120;
const CONV_ID = "e2e-post-tool-thinking";
/** 工具前思维链。 */
const PRE_TOOL_THINKING = "用户问技能清单，我先想想要不要调用 skill_manage。";
/** 工具前旁白（正文相）。 */
const COMMENTARY = "我先查一下当前工作区实际安装的技能清单。";
const TOOL_NAME = "skill_manage";
const TOOL_SUMMARY = "Skill install/list/remove";
const TOOL_CALL_ID = "tc_skill";
/** 工具后思维链：本用例核心，必须渲染成思考块。 */
const POST_TOOL_THINKING =
  "List them, grouped. 25 skills, all at system layer.";
const FINAL_ANSWER = "下面是按分组整理的 25 个技能清单。";

function buildPostToolReasoningSse(): string[] {
  let id = 1;
  const next = (payload: string) => {
    const line = `id: ${id}\ndata: ${payload}\n\n`;
    id += 1;
    return line;
  };
  return [
    // ① pre-tool 思维链（lane 默认 Reasoning）
    next(
      JSON.stringify({
        type: "REASONING_MESSAGE_CONTENT",
        messageId: "r-1",
        delta: PRE_TOOL_THINKING,
      }),
    ),
    // ② 首段正文前：进入正文相
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "assistant_answer_phase",
      }),
    ),
    // ③ 工具前旁白
    next(COMMENTARY),
    // ④ 解析工具调用
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "parsing_tool_calls",
        data: { parsing: true },
      }),
    ),
    // ⑤ 工具调用声明
    next(
      JSON.stringify({
        type: "TOOL_CALL_START",
        toolCallId: TOOL_CALL_ID,
        name: TOOL_NAME,
        summary: TOOL_SUMMARY,
      }),
    ),
    // ⑥ 工具结果
    next(
      JSON.stringify({
        type: "TOOL_CALL_RESULT",
        toolCallId: TOOL_CALL_ID,
        content: "已列出 25 个技能。",
        metadata: { name: TOOL_NAME, ok: true, summary: TOOL_SUMMARY },
      }),
    ),
    // ⑦ 工具批结束
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_tool_phase_end",
        data: { phase: "tool_end" },
      }),
    ),
    // ⑧ 上一段结束
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_segment_end",
        data: { segmentId: "seg-model-round-1" },
      }),
    ),
    // ⑨ 新一轮 answer 段开始（outer loop 非首轮）
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_segment_start",
        data: { segmentId: "seg-model-round-2", kind: "answer" },
      }),
    ),
    // ⑩ outer loop 非首轮：模型调用前发 answer_phase
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "assistant_answer_phase",
      }),
    ),
    // ⑪ 工具后第二段 reasoning ← 本用例的 bug 触发点
    next(
      JSON.stringify({
        type: "REASONING_MESSAGE_CONTENT",
        messageId: "r-2",
        delta: POST_TOOL_THINKING,
      }),
    ),
    // ⑫ 本轮首段正文前：再发一次 answer_phase
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "assistant_answer_phase",
      }),
    ),
    // ⑬ 终答
    next(FINAL_ANSWER),
    next(JSON.stringify({ type: "RUN_FINISHED", threadId: "", runId: "1" })),
  ];
}

async function installDelayedSse(
  page: import("@playwright/test").Page,
  events: string[],
  conversationId: string,
) {
  await page.addInitScript(
    ({ sseEvents, delayMs, convId }) => {
      Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
        configurable: true,
        value: { invoke: () => Promise.resolve(null) },
      });
      const originalFetch = window.fetch.bind(window);
      window.fetch = (input, init) => {
        const url =
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        const method = (
          init?.method ?? (input instanceof Request ? input.method : "GET")
        ).toUpperCase();
        if (!url.includes("/chat/stream") || method !== "POST") {
          return originalFetch(input, init);
        }
        const encoder = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
          start(controller) {
            let index = 0;
            const push = () => {
              if (index >= sseEvents.length) {
                controller.close();
                return;
              }
              controller.enqueue(encoder.encode(sseEvents[index]));
              index += 1;
              window.setTimeout(push, delayMs);
            };
            push();
          },
        });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: {
              "content-type": "text/event-stream; charset=utf-8",
              "x-conversation-id": convId,
              "x-stream-job-id": "1",
            },
          }),
        );
      };
    },
    { sseEvents: events, delayMs: STREAM_DELAY_MS, convId: conversationId },
  );
}

/** 读取所有思考段（含折叠内容）的 textContent，以及各 turn 的 role:preview。 */
async function readTranscriptTurns(
  page: import("@playwright/test").Page,
): Promise<{ thinkTexts: string[]; turnLabels: string[] }> {
  return page.evaluate(() => {
    const turns = [
      ...document.querySelectorAll<HTMLElement>("section.chat-tui-turn"),
    ];
    const thinkTexts = turns
      .filter((el) => el.classList.contains("chat-tui-turn--think"))
      .map((el) => el.textContent ?? "");
    const turnLabels = turns.map((el) => {
      const role = [...el.classList]
        .find((c) => c.startsWith("chat-tui-turn--"))
        ?.replace("chat-tui-turn--", "");
      const preview = (el.textContent ?? "")
        .replace(/\s+/g, " ")
        .trim()
        .slice(0, 60);
      return `${role}:${preview}`;
    });
    return { thinkTexts, turnLabels };
  });
}

test("post-tool reasoning must render as a thinking block", async ({
  page,
}) => {
  const chunks = buildPostToolReasoningSse();
  const sid = `s_e2e_post_tool_think_${Date.now()}`;

  await installDelayedSse(page, chunks, CONV_ID);
  await seedSession(page, sid);
  await sendMessage(page, "你有哪些技能");

  const transcript = page.getByTestId("chat-tui-transcript");

  // 流式完成
  await expect(page.getByTestId("status-bar")).toContainText("就绪", {
    timeout: 45_000,
  });
  await expect(transcript).toContainText(FINAL_ANSWER, { timeout: 20_000 });

  const { thinkTexts, turnLabels } = await readTranscriptTurns(page);

  // 核心断言：工具后 reasoning 必须落在思考段内。
  const postToolInThink = thinkTexts.some((t) =>
    t.includes(POST_TOOL_THINKING),
  );
  expect(
    postToolInThink,
    `工具后 reasoning 未渲染成思考块。\n` +
      `thinkTexts=${JSON.stringify(thinkTexts)}\n` +
      `turns=${JSON.stringify(turnLabels)}`,
  ).toBe(true);
});
