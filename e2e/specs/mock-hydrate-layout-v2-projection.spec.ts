/**
 * 水合用服务端会话级 `layout` 把历史行升级成 v2 投影键（#545）。
 *
 * 场景：`GET /conversation/messages` 同时返回 v2 `layout` 与 OpenAI 兼容 `messages`
 * （user → 工具前旁白 → 工具结果 → 终答）。修复前客户端把会话压成 `LEGACY`，
 * 每次加载都用新的 `h_{毫秒}_{序号}` 重建整页；修复后历史行拿到稳定键：
 *
 *   - `turn-user-u0`（user 行也有确定性键）
 *   - `turn-commentary-<tool_call_id>` / `turn-tool-<tool_call_id>`
 *   - `turn-final-answer#prev1`（规范键 `turn-final-answer` 留给流式路径）
 *
 * 断言（对应 #545 验收）：
 *   1. 刷新两次，持久化 `session.messages` 的 id 序列**完全一致**；
 *   2. DOM 工具卡数量在刷新前后不变（没有多出/丢掉工具卡）；
 *   3. 会话被标成 v2（`layout_schema_version === 2`）。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-hydrate-layout-v2-projection.spec.ts
 */
import { expect, type Page, test } from "@playwright/test";
import { apiUrl, openSessionInRail, seedSession } from "../fixtures/helpers";
import { fetchPersistedSession } from "../fixtures/session_assertions";

const TOOL_NAME = "get_current_time";
const TOOL_CALL_ID = "tc_time";
const USER_PROMPT = "现在时间是什么";
const COMMENTARY = "我先读一下当前时间。";
const TOOL_RESULT_BODY = "当前时间：2026-11-21 09:30:00";
const FINAL_ANSWER = "当前时间是 **2026-11-21 09:30:00**。";

async function installLayoutHistoryRoute(
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
        layout: {
          layout_schema_version: 2,
          projection_hash: "e2e0layout0hash",
          segments: [
            {
              turn_id: "u0",
              segment_id: `seg-before-${TOOL_CALL_ID}`,
              segment_kind: "assistant_commentary",
              before_tool_call_id: TOOL_CALL_ID,
              sequence: 0,
            },
            {
              turn_id: "u0",
              segment_id: TOOL_CALL_ID,
              segment_kind: "tool",
              sequence: 1,
            },
            {
              turn_id: "u0",
              segment_id: "assistant_answer-2",
              segment_kind: "assistant_answer",
              sequence: 2,
            },
          ],
        },
        messages: [
          { role: "user", content: USER_PROMPT },
          { role: "assistant", content: COMMENTARY },
          {
            role: "tool",
            name: TOOL_NAME,
            tool_call_id: TOOL_CALL_ID,
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

async function seedEmptyBoundSession(page: Page, sid: string, cid: string) {
  await page.evaluate(
    async ({ url, sessionId, conversationId }) => {
      await fetch(url, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sessions: [
            {
              id: sessionId,
              // 冷启动：空缓存、无 v2 投影，等水合按 layout 升级
              layout_schema_version: 1,
              title: "e2e-hydrate-layout-v2",
              draft: "",
              updated_at: Date.now(),
              pinned: false,
              starred: false,
              server_conversation_id: conversationId,
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
      conversationId: cid,
    },
  );
}

async function countDomToolTurns(page: Page): Promise<number> {
  return page.locator("section.chat-tui-turn--tool").count();
}

async function persistedIds(page: Page, sid: string): Promise<string[]> {
  const session = await fetchPersistedSession(page, sid);
  return (session?.messages ?? []).map((m) => m.id ?? "");
}

test.describe("水合按 layout 升级 v2 投影（#545）", () => {
  test("刷新两次：id 序列与工具卡数量不变", async ({ page }) => {
    const sid = `s_e2e_layout_v2_${Date.now()}`;
    const conversationId = `conv-layout-v2-${Date.now()}`;
    await seedSession(page, sid);
    await installLayoutHistoryRoute(page, conversationId, 3);
    await seedEmptyBoundSession(page, sid, conversationId);

    await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
    await page.waitForSelector('[data-testid="chat-composer-input"]');
    await openSessionInRail(page, sid);

    await expect
      .poll(async () => countDomToolTurns(page), { timeout: 15_000 })
      .toBe(1);

    await expect
      .poll(
        async () =>
          (await persistedIds(page, sid)).some(
            (id) => id === `turn-commentary-${TOOL_CALL_ID}`,
          ),
        { timeout: 15_000 },
      )
      .toBe(true);

    const firstIds = await persistedIds(page, sid);

    expect(
      firstIds,
      `首次水合后的 id 序列: ${JSON.stringify(firstIds)}`,
    ).toEqual([
      "turn-user-u0",
      `turn-commentary-${TOOL_CALL_ID}`,
      `turn-tool-${TOOL_CALL_ID}`,
      "turn-final-answer#prev1",
    ]);

    const firstSession = await fetchPersistedSession(page, sid);
    expect(firstSession?.layout_schema_version).toBe(2);

    await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
    await page.waitForSelector('[data-testid="chat-composer-input"]');
    await openSessionInRail(page, sid);
    await expect
      .poll(async () => countDomToolTurns(page), { timeout: 15_000 })
      .toBe(1);

    const secondIds = await persistedIds(page, sid);
    expect(
      secondIds,
      `刷新后的 id 序列必须逐字不变（before=${JSON.stringify(firstIds)}）`,
    ).toEqual(firstIds);
    expect(await countDomToolTurns(page)).toBe(1);
  });
});
