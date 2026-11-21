/**
 * 「加载更早消息」分页回归（#548 Wave 4，由 Victauri 迁入）。
 *
 * 来源：`desktop-tauri/src-tauri/tests/victauri_conversation.rs`（
 * `click_load_older_fetches_older_and_hides_control`）与
 * `victauri_pagination.rs` 中可由 UI 观察到的部分。
 *
 * 原用例在 webview 里 `eval_js("fetch('/conversation/messages?...')")` 直打后端，
 * 并靠 `CM_E2E_FIXTURES=1` 的播种路由塞 100 条消息 —— 那个前置条件本来就只为
 * 「造够分页的数据」服务，与本用例要断言的行为无关。这里改用 `page.route`
 * 按 query 返回分页响应，于是 `CM_E2E_FIXTURES` 从 E2E 前置条件里彻底摘掉。
 *
 * 分工（与 Server 的边界）：
 *   - 本 spec 断言 **UI 如何消费** `window_start_index` / `has_older`：
 *     尾页渲染 → 「加载更早」可见 → 点击带 `before_index` → 更早页前置 → 按钮消失。
 *   - `GET /conversation/messages` 的响应形状本身（省略 `limit` 返回全量窗口、
 *     `limit` 截尾页、未知 conversation 回 404）是 Server 契约，归 Server 仓集成测试。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-conversation-pagination.spec.ts
 */
import { expect, type Page, test } from "@playwright/test";
import { openSessionInRail, seedSession } from "../fixtures/helpers";

/** 会话总消息数（须大于一页，才能产生 `has_older`）。 */
const TOTAL = 100;
/** 前端页大小，与 `conversation_messages_page::CONVERSATION_MESSAGES_PAGE_LIMIT` 对齐。 */
const PAGE = 80;
/** 尾页的 `window_start_index`（= TOTAL - PAGE）。 */
const TAIL_START = TOTAL - PAGE;

type SeenRequest = { limit?: number; beforeIndex?: number };

function messagesBetween(start: number, end: number) {
  return Array.from({ length: end - start }, (_, i) => ({
    role: "user",
    content: `e2e-msg-${start + i}`,
  }));
}

function pageResponse(cid: string, start: number, end: number) {
  return {
    conversation_id: cid,
    revision: 1,
    messages: messagesBetween(start, end),
    total_count: TOTAL,
    window_start_index: start,
    has_older: start > 0,
  };
}

/** 注册 `/conversation/messages` 分页 mock，并记录每次请求的 query 供断言。 */
async function installPaginationRoute(page: Page, cid: string) {
  const seen: SeenRequest[] = [];
  await page.route("**/conversation/messages*", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== "/conversation/messages") return route.continue();

    const limitRaw = url.searchParams.get("limit");
    const beforeRaw = url.searchParams.get("before_index");
    const limit = limitRaw === null ? undefined : Number(limitRaw);
    const beforeIndex = beforeRaw === null ? undefined : Number(beforeRaw);
    seen.push({ limit, beforeIndex });

    // 尾页 = 后 PAGE 条；`before_index` = 该下标之前的一页；无 `limit` = 全量。
    // `before_index` 是页**尾**（不含），页首要再往前推一页。
    const end = beforeIndex ?? TOTAL;
    const start =
      beforeIndex === undefined
        ? limit === undefined
          ? 0
          : Math.max(0, TOTAL - limit)
        : Math.max(0, beforeIndex - (limit ?? PAGE));
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(pageResponse(cid, start, end)),
    });
  });
  return seen;
}

/** 单页会话（`has_older=false`），用于「按钮不该出现」的反向断言。 */
async function installSinglePageRoute(page: Page, cid: string) {
  await page.route("**/conversation/messages*", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== "/conversation/messages") return route.continue();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        conversation_id: cid,
        revision: 1,
        messages: messagesBetween(0, 3),
        total_count: 3,
        window_start_index: 0,
        has_older: false,
      }),
    });
  });
}

test("点击「加载更早」前置更早页、按钮消失且不丢尾页", async ({ page }) => {
  const sid = `s_e2e_pagination_${Date.now()}`;
  const cid = `conv-pagination-${Date.now()}`;
  const seen = await installPaginationRoute(page, cid);

  await seedSession(page, sid, {
    session: { server_conversation_id: cid, server_revision: 1 },
  });
  await openSessionInRail(page, sid);

  // 尾页水合：最后一条可见，首条还在更早的页里
  const newest = page.getByText(`e2e-msg-${TOTAL - 1}`);
  const oldest = page.getByText("e2e-msg-0");
  await expect(newest).toBeVisible({ timeout: 20_000 });
  await expect(oldest).toHaveCount(0);

  // 加载条在滚动容器顶部（初始在底部、未必在视口内），故断言 DOM 存在 + 可点，
  // 不要求 `toBeVisible`；`click()` 会自行滚动到位。
  const loadOlder = page.getByTestId("chat-load-older");
  await expect(loadOlder).toHaveCount(1);
  await loadOlder.click();

  await expect(oldest).toBeVisible({ timeout: 20_000 });

  // loading 期间按钮位被 busy span 取代，所以「按钮消失」本身可能是假阳性；
  // 先等前页请求确实落地，再断言按钮不再回来。
  await expect
    .poll(() => seen.filter((r) => r.beforeIndex === TAIL_START).length, {
      timeout: 15_000,
    })
    .toBe(1);
  await expect(loadOlder).toHaveCount(0);

  // 前置必须是「拼到前面」，尾页不得被替换掉
  await expect(newest).toBeVisible();

  // 请求形状：尾页一次（limit=80），前页一次（limit=80 + before_index=20）
  expect(
    seen.filter((r) => r.limit === PAGE && r.beforeIndex === undefined),
  ).toHaveLength(1);
  expect(seen.at(-1)).toEqual({ limit: PAGE, beforeIndex: TAIL_START });
});

test("单页会话（has_older=false）不出现「加载更早」", async ({ page }) => {
  const sid = `s_e2e_single_page_${Date.now()}`;
  const cid = `conv-single-page-${Date.now()}`;
  await installSinglePageRoute(page, cid);

  await seedSession(page, sid, {
    session: { server_conversation_id: cid, server_revision: 1 },
  });
  await openSessionInRail(page, sid);

  await expect(page.getByText("e2e-msg-2")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("chat-load-older")).toHaveCount(0);
});
