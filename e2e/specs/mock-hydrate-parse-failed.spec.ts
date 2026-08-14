/**
 * Issue #26：服务端 messages 非空但客户端解析为 0 条时须显示水合错误、保留本地时间线、可重试。
 */
import { expect, type Page, test } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

const LOCAL_USER = "e2e-local-user-before-hydrate";

async function installUnparseableHistoryRoute(
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
        messages: [{ invalid: true }, { role: "" }],
        total_count: 2,
        window_start_index: 0,
        has_older: false,
      }),
    }),
  );
}

test("hydration parse failure shows error, keeps local messages, retry refetches", async ({
  page,
}) => {
  const sid = `s_e2e_hydrate_parse_fail_${Date.now()}`;
  const conversationId = `conv-hydrate-parse-fail-${Date.now()}`;
  await seedSession(page, sid);
  await installUnparseableHistoryRoute(page, conversationId, 2);

  await page.evaluate(
    async ({ sessionId, cid, localText }) => {
      await fetch("/user-data/workspaces/current/sessions", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sessions: [
            {
              id: sessionId,
              layout_schema_version: 1,
              title: "e2e-hydrate-parse-fail",
              draft: "",
              updated_at: Date.now(),
              pinned: false,
              starred: false,
              server_conversation_id: cid,
              server_revision: 1,
              messages: [
                {
                  id: "m_local_user",
                  role: "user",
                  text: localText,
                  is_tool: false,
                  created_at: Date.now(),
                },
              ],
            },
          ],
          active_session_id: sessionId,
        }),
      });
    },
    { sessionId: sid, cid: conversationId, localText: LOCAL_USER },
  );

  await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]');

  await expect(page.getByTestId("hydration-parse-error")).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("status-bar")).toContainText("解析失败", {
    timeout: 5_000,
  });
  await expect(page.getByText(LOCAL_USER)).toBeVisible();

  let fetchCount = 0;
  await page.route("**/conversation/messages?**", (route) => {
    fetchCount += 1;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        conversation_id: conversationId,
        revision: 2,
        messages: [{ invalid: true }],
        total_count: 1,
        window_start_index: 0,
        has_older: false,
      }),
    });
  });

  await page.getByTestId("hydration-retry").click();
  await expect.poll(() => fetchCount, { timeout: 10_000 }).toBeGreaterThan(0);
  await expect(page.getByText(LOCAL_USER)).toBeVisible();
});
