/**
 * 基于导出会话 `chat_export_20260808_212552.md`：
 *
 *   ## 工具
 *   工具：get_current_time          ← 空 assistant.tool_calls 水合出的调用短卡
 *   ## 助手
 *   当前时间是 **…**                ← 终答被夹在中间
 *   ## 工具
 *   get_current_time + 完整结果      ← role=tool 水合出的结果卡
 *
 * 且工具前无解读（模型本轮无 preamble；水合也不会凭空造旁白）。
 *
 * 根因：
 *   1. `conversation_hydrate` 把「空正文 + tool_calls」与 `role=tool` 各建成一条 is_tool
 *   2. legacy `merge_session_tail` 的 tool_pool FIFO：本地 1 工具先吃到调用短卡，
 *      结果卡被 append 到终答之后 → 导出双「## 工具」夹心
 *
 * 本文件用 **legacy layout + mock GET /conversation/messages** 复现水合路径
 * （不依赖真实 LLM）。期望（修复后应绿）：
 *   - 同一次 get_current_time 仅 1 张工具卡
 *   - 工具在终答之前
 *   - 不得出现「工具短卡 → 终答 → 工具结果」三明治
 *
 * 运行：
 *   cd e2e && no_proxy=127.0.0.1,localhost npx playwright test \
 *     specs/mock-hydrate-duplicate-tool-cards.spec.ts
 */
import { expect, type Page, test } from "@playwright/test";
import { apiUrl, seedSession, openSessionInRail } from "../fixtures/helpers";
import {
  fetchPersistedSession,
  type PersistedMessage,
} from "../fixtures/session_assertions";

const TOOL_NAME = "get_current_time";
const CALL_CARD_TEXT = `工具：${TOOL_NAME}`;
/** 水合 API / 持久化里的助手正文（可含 markdown）。 */
const FINAL_ANSWER = "当前时间是 **2026-08-08 21:25:05**。";
const TOOL_RESULT_BODY = `当前时间：2026-08-08 21:25:05`;
const USER_PROMPT = "现在时间是什么";

type TurnSnap = {
  kind: "user" | "assistant" | "tool";
  text: string;
};

async function installOpenAiHistoryRoute(
  page: Page,
  conversationId: string,
  revision: number,
) {
  await page.route("**/conversation/messages?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        conversation_id: conversationId,
        revision,
        messages: [
          { role: "user", content: USER_PROMPT },
          {
            role: "assistant",
            content: "",
            tool_calls: [
              {
                id: "tc_time",
                type: "function",
                function: {
                  name: TOOL_NAME,
                  arguments: "{}",
                },
              },
            ],
          },
          {
            role: "tool",
            name: TOOL_NAME,
            tool_call_id: "tc_time",
            content: TOOL_RESULT_BODY,
          },
          { role: "assistant", content: FINAL_ANSWER },
        ],
        total_count: 4,
        window_start_index: 0,
        has_older: false,
      }),
    }),
  );
}

async function readDomTurns(page: Page): Promise<TurnSnap[]> {
  return page.evaluate(() => {
    const sections = [
      ...document.querySelectorAll<HTMLElement>(
        "section.chat-tui-turn--user, section.chat-tui-turn--assistant, section.chat-tui-turn--tool",
      ),
    ];
    return sections.map((el) => {
      const kind = el.classList.contains("chat-tui-turn--tool")
        ? ("tool" as const)
        : el.classList.contains("chat-tui-turn--user")
          ? ("user" as const)
          : ("assistant" as const);
      return {
        kind,
        text: (el.innerText ?? "").replace(/\s+/g, " ").trim(),
      };
    });
  });
}

function isCallStub(message: PersistedMessage): boolean {
  const text = (message.text ?? "").trim();
  return (
    !!message.is_tool &&
    (text === CALL_CARD_TEXT || text === TOOL_NAME || text.startsWith("工具："))
  );
}

function isResultCard(message: PersistedMessage): boolean {
  const text = `${message.text ?? ""}\n${(message as { reasoning_text?: string }).reasoning_text ?? ""}`;
  return !!message.is_tool && text.includes(TOOL_RESULT_BODY);
}

function assertNoToolAnswerSandwich(messages: PersistedMessage[]) {
  const kinds = messages.map((m) => {
    if (m.is_tool) return "tool";
    if (m.role === "user") return "user";
    return "assistant";
  });
  const joined = kinds.join(">");
  expect(
    joined,
    `不得出现工具→助手→工具夹心（export 形态），实际 role 链: ${joined}`,
  ).not.toMatch(/tool>assistant>tool/);
}

test.describe("水合双工具卡 / 终答夹心（export 20260808）", () => {
  test("冷启动水合：OpenAI tool_calls + tool 不得拆成两张卡", async ({
    page,
  }) => {
    const sid = `s_e2e_hydrate_dup_cold_${Date.now()}`;
    const conversationId = `conv-hydrate-dup-cold-${Date.now()}`;
    await seedSession(page, sid);
    await installOpenAiHistoryRoute(page, conversationId, 1);

    await page.evaluate(
      async ({ url, sessionId, cid }) => {
        await fetch(url, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            sessions: [
              {
                id: sessionId,
                // legacy：走 merge_session_tail，而非 v2 append-only
                layout_schema_version: 1,
                title: "e2e-hydrate-dup-cold",
                draft: "",
                updated_at: Date.now(),
                pinned: false,
                starred: false,
                server_conversation_id: cid,
                server_revision: 0,
                messages: [],
              },
            ],
            active_session_id: sessionId,
          }),
        });
      },
      {
        url: apiUrl("/user-data/workspaces/current/sessions"),
        sessionId: sid,
        cid: conversationId,
      },
    );

    await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
    await page.waitForSelector('[data-testid="chat-composer-input"]');
    await openSessionInRail(page, sid);
    // 等 DOM 出现工具行（sessions API 可能尚未回写）。
    await expect
      .poll(
        async () =>
          (await readDomTurns(page)).filter((t) => t.kind === "tool").length,
        {
          timeout: 15_000,
        },
      )
      .toBeGreaterThanOrEqual(1);

    const turns = await readDomTurns(page);
    const toolTurns = turns.filter((t) => t.kind === "tool");
    // 修复前：call 短卡 + result 卡 = 2（可见两张 ✅🕐get_current_time）
    expect(
      toolTurns.length,
      `DOM tools=${JSON.stringify(toolTurns.map((t) => t.text.slice(0, 48)))}`,
    ).toBe(1);

    const session = await fetchPersistedSession(page, sid);
    const messages = session?.messages ?? [];
    if (messages.some((m) => m.is_tool)) {
      expect(
        messages.filter((m) => m.is_tool).length,
        `persisted tools=${JSON.stringify(messages.filter((m) => m.is_tool).map((t) => t.text?.slice(0, 40)))}`,
      ).toBe(1);
      assertNoToolAnswerSandwich(messages);
    }

    const answerIdx = turns.findIndex(
      (t) => t.kind === "assistant" && t.text.includes("当前时间是"),
    );
    const toolIdx = turns.findIndex((t) => t.kind === "tool");
    expect(toolIdx).toBeGreaterThanOrEqual(0);
    expect(answerIdx).toBeGreaterThan(toolIdx);
  });

  test("legacy merge：本地 1 工具 + 服务端 call/result 双卡不得夹心", async ({
    page,
  }) => {
    const sid = `s_e2e_hydrate_dup_merge_${Date.now()}`;
    const conversationId = `conv-hydrate-dup-merge-${Date.now()}`;
    await seedSession(page, sid);
    await installOpenAiHistoryRoute(page, conversationId, 2);

    await page.evaluate(
      async ({
        url,
        sessionId,
        cid,
        callCard,
        finalAnswer,
        resultBody,
        userPrompt,
      }) => {
        await fetch(url, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            sessions: [
              {
                id: sessionId,
                layout_schema_version: 1,
                title: "e2e-hydrate-dup-merge",
                draft: "",
                updated_at: Date.now(),
                pinned: false,
                starred: false,
                server_conversation_id: cid,
                server_revision: 1,
                history_window_start: 0,
                messages: [
                  {
                    id: "u-local",
                    role: "user",
                    text: userPrompt,
                    reasoning_text: "",
                    created_at: 1,
                  },
                  // 流式结束后本地通常只剩 1 条结果工具行（与 export 前半一致）
                  {
                    id: "sse-tool-result",
                    role: "system",
                    text: "get_current_time",
                    reasoning_text: resultBody,
                    is_tool: true,
                    tool_call_id: "tc_time",
                    tool_name: "get_current_time",
                    created_at: 2,
                  },
                  {
                    id: "a-local",
                    role: "assistant",
                    text: finalAnswer,
                    reasoning_text: "",
                    created_at: 3,
                  },
                ],
              },
            ],
            active_session_id: sessionId,
          }),
        });
        void callCard;
      },
      {
        url: apiUrl("/user-data/workspaces/current/sessions"),
        sessionId: sid,
        cid: conversationId,
        callCard: CALL_CARD_TEXT,
        finalAnswer: FINAL_ANSWER,
        resultBody: TOOL_RESULT_BODY,
        userPrompt: USER_PROMPT,
      },
    );

    await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
    await page.waitForSelector('[data-testid="chat-composer-input"]');
    await openSessionInRail(page, sid);
    await expect
      .poll(async () => {
        const session = await fetchPersistedSession(page, sid);
        return session?.server_revision ?? 0;
      })
      .toBeGreaterThanOrEqual(2);

    const session = await fetchPersistedSession(page, sid);
    const messages = session?.messages ?? [];
    const tools = messages.filter((m) => m.is_tool);

    // 修复前典型失败：2 张工具卡，且 call stub 在终答前、result 在终答后
    expect(
      tools.length,
      `tools=${JSON.stringify(tools.map((t) => t.text?.slice(0, 40)))}`,
    ).toBe(1);
    assertNoToolAnswerSandwich(messages);

    const callStubs = messages.filter(isCallStub);
    const results = messages.filter(isResultCard);
    // 允许保留结果卡，但不得同时残留「工具：name」短卡
    expect(
      callStubs.length,
      `call stubs should be collapsed into result: ${JSON.stringify(callStubs)}`,
    ).toBe(0);
    expect(results.length).toBe(1);

    const turns = await readDomTurns(page);
    const kinds = turns.map((t) => t.kind).join(">");
    expect(kinds, `DOM turn chain: ${kinds}`).not.toMatch(
      /tool>assistant>tool/,
    );
    expect(turns.filter((t) => t.kind === "tool")).toHaveLength(1);

    // 本轮无 preamble：助手区只有终答，工具前不应多出解读气泡
    const assistantsBeforeFirstTool = (() => {
      const firstTool = turns.findIndex((t) => t.kind === "tool");
      if (firstTool < 0) return turns.filter((t) => t.kind === "assistant");
      return turns.slice(0, firstTool).filter((t) => t.kind === "assistant");
    })();
    expect(assistantsBeforeFirstTool).toHaveLength(0);
  });
});
