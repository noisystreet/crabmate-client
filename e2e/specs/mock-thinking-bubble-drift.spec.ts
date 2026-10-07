/**
 * 复现：思维链（reasoning）气泡在工具调用边界发生**位置漂移**。
 *
 * 用户反馈（chat_export_20261007_164106.md）：
 *   「思考对应的消息气泡移位了，生成过程中是在『你有哪些技能』之后，
 *     然后又跑到工具调用后了」
 *
 * 即：流式过程中思考段（`section.chat-tui-turn--think`）位于所属助手气泡内、
 * 落在工具调用**之前**；工具声明后，承载 pre-tool reasoning 的 loading 尾泡被
 * `pin_loading_tail_in_messages` 移到工具行**之后**，思考段随气泡一起下移。
 *
 * 本用例用 mock SSE 重放该时序（reasoning delta → 工具前旁白 → TOOL_CALL_START
 * → TOOL_CALL_RESULT → 新一轮 answer），用 MutationObserver + 16ms 定时采样
 * 记录「思考段下标 vs 工具下标」，断言二者相对顺序**不得翻转**。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-thinking-bubble-drift.spec.ts
 */
import { expect, test } from "@playwright/test";
import { seedSession, sendMessage } from "../fixtures/helpers";

const STREAM_DELAY_MS = 120;
const CONV_ID = "e2e-thinking-bubble-drift";
/** pre-tool 思维链（工具前应当出现、且不应移到工具后）。 */
const PRE_TOOL_THINKING = "用户问技能清单，我先想想要不要调用 skill_manage。";
/** 工具前旁白（正文相）。 */
const COMMENTARY = "我先列一下当前工作区实际安装的技能清单，避免凭记忆漏报。";
const TOOL_NAME = "skill_manage";
const TOOL_SUMMARY = "Skill install/list/remove";
const TOOL_CALL_ID = "tc_skill";
const FINAL_ANSWER = "当前工作区未设置，所以没法列出完整清单。";

type DriftSample = {
  thinkIdx: number;
  toolIdx: number;
  thinkPreview: string;
  turnLabels: string[];
  atMs: number;
};

function buildReasoningThenToolSse(): string[] {
  let id = 1;
  const next = (payload: string) => {
    const line = `id: ${id}\ndata: ${payload}\n\n`;
    id += 1;
    return line;
  };
  return [
    // ① pre-tool 思维链（正文相之前：lane 默认 Reasoning，走 overlay.reasoning）
    next(
      JSON.stringify({
        type: "REASONING_MESSAGE_CONTENT",
        messageId: "r-1",
        delta: PRE_TOOL_THINKING,
      }),
    ),
    // ② 正文相开始 + 工具前旁白
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
        data: { segmentId: "seg-before-skill" },
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
        content: "错误：未设置工作区，禁止执行内置工具。",
        metadata: { name: TOOL_NAME, ok: false, summary: TOOL_SUMMARY },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_tool_phase_end",
        data: { phase: "tool_end" },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "turn_segment_start",
        data: { segmentId: "seg-final", kind: "answer" },
      }),
    ),
    next(
      JSON.stringify({
        type: "CUSTOM",
        customType: "assistant_answer_phase",
      }),
    ),
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

async function installDriftSampler(
  page: import("@playwright/test").Page,
  toolNeedle: string,
) {
  await page.evaluate((toolText: string) => {
    const w = window as unknown as {
      __cmDriftSamples?: DriftSample[];
      __cmDriftObserver?: MutationObserver;
      __cmDriftTimer?: number;
    };
    w.__cmDriftSamples = [];
    const started = performance.now();
    const sample = () => {
      const turns = [
        ...document.querySelectorAll<HTMLElement>("section.chat-tui-turn"),
      ];
      const turnLabels = turns.map((el) => {
        const role = [...el.classList]
          .find((c) => c.startsWith("chat-tui-turn--"))
          ?.replace("chat-tui-turn--", "");
        const preview = (el.innerText ?? "")
          .replace(/\s+/g, " ")
          .trim()
          .slice(0, 40);
        return `${role}:${preview}`;
      });
      const thinkIdx = turns.findIndex((el) =>
        el.classList.contains("chat-tui-turn--think"),
      );
      const toolIdx = turns.findIndex(
        (el) =>
          el.classList.contains("chat-tui-turn--tool") &&
          (el.innerText ?? "").includes(toolText),
      );
      const thinkPreview =
        thinkIdx >= 0
          ? (turns[thinkIdx].innerText ?? "").replace(/\s+/g, " ").trim()
          : "";
      w.__cmDriftSamples!.push({
        thinkIdx,
        toolIdx,
        thinkPreview,
        turnLabels,
        atMs: Math.round(performance.now() - started),
      });
    };
    sample();
    const root =
      document.querySelector('[data-testid="chat-tui-transcript"]') ??
      document.body;
    const observer = new MutationObserver(() => sample());
    observer.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    w.__cmDriftObserver = observer;
    w.__cmDriftTimer = window.setInterval(sample, 16) as unknown as number;
  }, toolNeedle);
}

async function readDriftSamples(
  page: import("@playwright/test").Page,
): Promise<DriftSample[]> {
  return page.evaluate(() => {
    const w = window as unknown as { __cmDriftSamples?: DriftSample[] };
    return [...(w.__cmDriftSamples ?? [])];
  });
}

test("pre-tool thinking bubble must not drift below the tool call", async ({
  page,
}) => {
  const chunks = buildReasoningThenToolSse();
  const sid = `s_e2e_thinking_drift_${Date.now()}`;

  await installDelayedSse(page, chunks, CONV_ID);
  await seedSession(page, sid);
  await installDriftSampler(page, TOOL_SUMMARY);
  await sendMessage(page, "你有哪些技能");

  const transcript = page.getByTestId("chat-tui-transcript");

  // ① 思考段先出现，此时工具尚未可见 → 思考「在工具之前」。
  await expect(transcript).toContainText(PRE_TOOL_THINKING, {
    timeout: 20_000,
  });
  const beforeTool = await readDriftSamples(page);
  const sawThinkBeforeTool = beforeTool.some(
    (s) => s.thinkIdx >= 0 && s.toolIdx === -1,
  );
  expect(
    sawThinkBeforeTool,
    `思考段应先于工具可见：${JSON.stringify(beforeTool.slice(-3))}`,
  ).toBe(true);

  // ② 工具出现。
  await expect(transcript).toContainText(TOOL_SUMMARY, { timeout: 20_000 });

  await expect(page.getByTestId("status-bar")).toContainText("就绪", {
    timeout: 45_000,
  });

  const samples = await readDriftSamples(page);
  const bothVisible = samples.filter((s) => s.thinkIdx >= 0 && s.toolIdx >= 0);
  expect(
    bothVisible.length,
    "至少应有一个采样同时看到思考段与工具",
  ).toBeGreaterThan(0);

  const above = bothVisible.filter((s) => s.thinkIdx < s.toolIdx);
  const below = bothVisible.filter((s) => s.thinkIdx > s.toolIdx);

  // 主断言：二者同时可见时，思考段相对工具的顺序不得翻转（不得从工具前跑到工具后）。
  if (above.length > 0 && below.length > 0) {
    const firstAbove = above[0];
    const firstBelow = below[0];
    throw new Error(
      `思考气泡漂移：先出现在工具前（atMs=${firstAbove.atMs}, ` +
        `thinkIdx=${firstAbove.thinkIdx}, toolIdx=${firstAbove.toolIdx}），` +
        `后移到工具后（atMs=${firstBelow.atMs}, thinkIdx=${firstBelow.thinkIdx}, ` +
        `toolIdx=${firstBelow.toolIdx}）\n` +
        `漂移后 turns=${JSON.stringify(firstBelow.turnLabels)}`,
    );
  }

  // 兜底：最终应稳定在工具之前。
  expect(
    below,
    `最终思考段落在工具之后：${JSON.stringify(below.slice(-3))}`,
  ).toEqual([]);
});
