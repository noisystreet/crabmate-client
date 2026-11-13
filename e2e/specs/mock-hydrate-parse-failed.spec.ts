/**
 * Issue #26：服务端 messages 非空但客户端解析为 0 条时须显示水合错误、保留本地时间线、可重试。
 */
import { expect, type Page, test } from "@playwright/test";
import { openSessionInRail, seedSession } from "../fixtures/helpers";

const LOCAL_USER = "e2e-local-user-before-hydrate";

async function installUnparsableHistoryRoute(
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
  // 路由必须先装：`seedSession` 里的那次导航就会触发水合拉取。
  await installUnparsableHistoryRoute(page, conversationId, 2);
  // 桶里直接落「已绑定 server_conversation_id + 一条本地 user」的会话，且只导航这一次。
  // 旧写法是「先载入页面 → 再 PUT 桶 → reload」：PUT 会与 SPA 自己的 400ms 防抖回写
  // （用内存里的旧列表整体覆盖桶）竞态，慢机器上 reload 拿到的是 seed 前的空会话，
  // 水合不触发，失败点被推到下游的元素等待超时（2026-11-12 CI 即此）。
  await seedSession(page, sid, {
    session: {
      layout_schema_version: 1,
      title: "e2e-hydrate-parse-fail",
      server_conversation_id: conversationId,
      server_revision: 1,
      messages: [
        {
          id: "m_local_user",
          role: "user",
          text: LOCAL_USER,
          is_tool: false,
          created_at: Date.now(),
        },
      ],
    },
  });
  await openSessionInRail(page, sid);

  // 先钉住「载入的确实是本用例 seed 的会话」：桶被旧状态覆盖时失败点应在这里。
  await expect(page.getByTestId(`nav-session-${sid}`)).toContainText(
    "e2e-hydrate-parse-fail",
    { timeout: 15_000 },
  );
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
