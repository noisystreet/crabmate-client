/**
 * Mock SSE 回归测试：流式进行中切换会话（后台流，Issue #28 / ADR-0001）
 *
 * 验证验收标准：
 *   1. 流式进行中侧栏该会话行显示「生成中」指示（is-streaming / nav-session-streaming）
 *   2. 流式进行中可切换到其它会话（不被阻塞），SSE 后台继续写入原会话
 *   3. 切回后正文完整、无 loading 残留
 *   4. 切换后再发送仍携带正确的 conversation_id（不错写服务器会话）
 *
 * 运行方式（前置：`cargo run -- serve` 在 127.0.0.1:8080 运行）：
 *   cd e2e && npx playwright test specs/mock-stream-switch-background.spec.ts
 */

import { test, expect, Page } from "@playwright/test";
import {
  apiUrl,
  applyWebApiBearerHeaders,
  homeUrlWithOptionalWebBearer,
  resolveWebApiBearerToken,
  sendMessage,
} from "../fixtures/helpers";

const SID_A = "s_e2e_bg_stream_a";
const SID_B = "s_e2e_bg_stream_b";
const CONV_ID = "e2e-conv-A";
const ANSWER = "这是后台流生成的完整回答内容。";

/** seed 两个会话：A 为 active，B 为第二个（参考 helpers.seedSession 的防抖时序）。 */
async function seedTwoSessions(page: Page) {
  await applyWebApiBearerHeaders(page);
  const bearer = resolveWebApiBearerToken();
  const jsonHeaders: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (bearer) {
    jsonHeaders.Authorization = `Bearer ${bearer}`;
  }

  const prefs = {
    locale: "zh",
    theme: "light",
    side_panel_view: "hidden",
    side_width: 280,
    editor_layout_mode: false,
    status_bar_visible: true,
  };
  const now = Date.now();
  const sessionsBody = {
    sessions: [
      {
        id: SID_A,
        title: "会话A",
        draft: "",
        messages: [],
        updated_at: now,
        pinned: false,
        starred: false,
      },
      {
        id: SID_B,
        title: "会话B",
        draft: "",
        messages: [],
        updated_at: now,
        pinned: false,
        starred: false,
      },
    ],
    active_session_id: SID_A,
  };

  expect(
    (
      await page.request.put(apiUrl("/user-data/prefs"), {
        data: prefs,
        headers: jsonHeaders,
      })
    ).ok(),
  ).toBe(true);
  expect(
    (
      await page.request.put(apiUrl("/user-data/workspaces/current/sessions"), {
        data: sessionsBody,
        headers: jsonHeaders,
      })
    ).ok(),
  ).toBe(true);

  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "networkidle",
    timeout: 20000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });

  // 等过防抖窗口，确认 SPA 没有把 seed 桶写坏。
  await expect
    .poll(
      async () => {
        const r = await page.request.get(
          apiUrl("/user-data/workspaces/current/sessions"),
          {
            headers: bearer ? { Authorization: `Bearer ${bearer}` } : undefined,
          },
        );
        if (!r.ok()) return false;
        const data = (await r.json()) as {
          sessions?: { id?: string }[];
          active_session_id?: string;
        };
        const ids = (data.sessions ?? []).map((s) => s.id);
        return (
          data.active_session_id === SID_A &&
          ids.includes(SID_A) &&
          ids.includes(SID_B)
        );
      },
      { timeout: 15000 },
    )
    .toBe(true);
}

/**
 * 慢速分块 mock SSE + 记录每次 /chat/stream 请求体。
 * `installDelayedMockSse` 不暴露请求体，故内联实现（fetch 层替换，分块输出制造「流式中」窗口）。
 */
async function installSlowSseRecording(
  page: Page,
  events: string[],
  delayMs: number,
) {
  await page.evaluate(
    ({ events, delay, conversationId }) => {
      const originalFetch = window.fetch.bind(window);
      const recorded: Array<{ conversation_id: unknown }> = [];
      (
        window as unknown as { __cm_streamRequests: typeof recorded }
      ).__cm_streamRequests = recorded;
      window.fetch = async (
        input: Parameters<typeof fetch>[0],
        init?: RequestInit,
      ) => {
        const req = input instanceof Request ? input : new Request(input, init);
        const url = req.url;
        const method = req.method.toUpperCase();
        if (!url.includes("/chat/stream") || method !== "POST") {
          return originalFetch(input, init);
        }
        let body: { conversation_id?: unknown } = {};
        try {
          // 前端经 `fetch_with_request` 发送 Request 对象，body 在其内（而非 init.body）。
          body = JSON.parse(await req.clone().text());
        } catch {
          /* 忽略非 JSON 请求体 */
        }
        recorded.push({ conversation_id: body.conversation_id });
        const encoder = new TextEncoder();
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            let index = 0;
            const pushNext = () => {
              if (index >= events.length) {
                controller.close();
                return;
              }
              controller.enqueue(encoder.encode(events[index]));
              index += 1;
              window.setTimeout(pushNext, delay);
            };
            pushNext();
          },
        });
        return new Response(stream, {
          status: 200,
          headers: {
            "content-type": "text/event-stream; charset=utf-8",
            "x-conversation-id": conversationId,
            "x-stream-job-id": "1",
          },
        });
      };
    },
    { events, delay: delayMs, conversationId: CONV_ID },
  );
}

test("流式进行中切换会话不丢流且不错写 conversation_id", async ({ page }) => {
  const sseEvents = [
    'id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n',
    "id: 2\ndata: 第一段\n\n",
    "id: 3\ndata: 第二段\n\n",
    `id: 4\ndata: ${ANSWER}\n\n`,
    'id: 5\ndata: {"type":"RUN_FINISHED"}\n\n',
  ];

  await seedTwoSessions(page);
  await installSlowSseRecording(page, sseEvents, 400);

  const rowA = page.getByTestId(`nav-session-${SID_A}`);
  const rowB = page.getByTestId(`nav-session-${SID_B}`);
  const statusBar = page.locator('[data-testid="status-bar"]');

  // ── 1. 发送 → 流式进行中，A 行显示「生成中」badge ──
  await sendMessage(page, "请生成一份文档");
  await expect(
    rowA.locator('[data-testid="nav-session-streaming"]'),
  ).toHaveCount(1, {
    timeout: 10000,
  });
  await expect(rowA).toHaveClass(/is-streaming/);

  // ── 2. 流式中切换到 B：不被阻塞，B 变为 active ──
  await rowB.click();
  await expect(rowB).toHaveClass(/is-active/);

  // ── 3. 后台流继续完成：A 行「生成中」badge 消失、状态栏就绪 ──
  await expect(
    rowA.locator('[data-testid="nav-session-streaming"]'),
  ).toHaveCount(0, {
    timeout: 25000,
  });
  await expect(statusBar).toContainText("就绪", { timeout: 25000 });

  // ── 4. 切回 A：正文完整、无 loading 残留 ──
  await rowA.click();
  await expect(rowA).toHaveClass(/is-active/);
  const scroller = page.locator('[data-testid="chat-messages-scroller"]');
  await expect(scroller).toContainText(ANSWER, { timeout: 5000 });
  const emptyAssistants = await page.evaluate(() => {
    const turns = document.querySelectorAll("section.chat-tui-turn--assistant");
    let n = 0;
    for (const turn of turns) {
      const text = (turn.querySelector(".chat-tui-body")?.textContent ?? "")
        .replace(/\s+/g, " ")
        .trim();
      if (!text) n += 1;
    }
    return n;
  });
  expect(emptyAssistants).toBe(0);

  // ── 5. 再发送：断言第 2 次 /chat/stream 请求携带 A 的 conversation_id ──
  await sendMessage(page, "补充说明");
  await expect(scroller).toContainText(ANSWER, { timeout: 25000 });
  const requests = await page.evaluate(
    () =>
      (
        window as unknown as {
          __cm_streamRequests: Array<{ conversation_id: unknown }>;
        }
      ).__cm_streamRequests,
  );
  expect(requests.length).toBeGreaterThanOrEqual(2);
  expect(requests[1].conversation_id).toBe(CONV_ID);
});
