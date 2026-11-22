/**
 * 单窗口多 lane：切 lane 要换**数据**，不只是换显示（#569）。
 *
 * 报告的缺陷是：连两台不同 serve，点另一条 lane 主区内容不变。根因有两层，本文件各守一条：
 *
 *   1. **数据面**（已修）：请求基址只有 `ACTIVE` 一个出口，后台那条的请求打到了前台那条的
 *      serve 上。现在每条 lane 的会话桶按它自己的 `ScopeHolder` 读 —— 用例断言「第二条
 *      lane 的请求命中了它自己那个 origin」。
 *   2. **切换本身**（已知未修，见下面 `test.fixme`）：`LaneSet::set_active` 写回 `LanePhase`
 *      时，某个订阅者访问了已 dispose 的反应式值 → wasm panic → `self.active.set(index)`
 *      根本没执行，所以界面上「点了没反应」。
 *
 * 浏览器里没有 Tauri 壳，所以这里装一个**最小假壳**：只实现 `ctl_list_instances`（两个可连
 * 实例）与 `list_connections`（本窗口那一行）。与 `mock-storage-scope.spec.ts` 同一范式 ——
 * 初始化脚本先于前端模块执行，等价于壳的 `initialization_script`。第二台 serve 用
 * `page.route` 顶替（`lane-b.test` 不需要能解析，路由在请求发出前就拦下了）。
 */
import { test, expect, type Page } from "@playwright/test";

import { apiBase, homeUrlWithOptionalWebBearer } from "../fixtures/helpers";

/** 第二台 serve 的「地址」：不可解析，只用于让请求落到本 spec 的 route 上。 */
const LANE_B_ORIGIN = "https://lane-b.test";

/** 第二台那边唯一的会话 id。 */
const LANE_B_SESSION = "sess-lane-b";

/**
 * 装最小假壳：左栏连接区要 `list_connections`（非空才渲染整块），「可连实例」要
 * `ctl_list_instances`。其余命令一律 resolve `null`。
 */
async function installFakeShell(page: Page): Promise<void> {
  const origin = apiBase() || "http://127.0.0.1:8080";
  const instances = [
    { name: "lane-a", observed: "ready", base_url: origin, enabled: true },
    {
      name: "lane-b",
      observed: "ready",
      base_url: LANE_B_ORIGIN,
      enabled: true,
    },
  ];
  // 本窗口已登记的那一行：`lane-a` 因此从「可连实例」里被过滤掉，只剩 lane-b 可点。
  const connections = [
    {
      key: "lane-a",
      display_name: "lane-a",
      authority: new URL(origin).host,
      active: true,
      status: "Idle",
      unread: false,
      context: null,
      busy_secs: null,
    },
  ];
  await page.addInitScript(
    (payload: {
      instances: unknown[];
      connections: unknown[];
      connKey: string;
    }) => {
      const w = window as unknown as Record<string, unknown>;
      w["__TAURI_INTERNALS__"] = {
        invoke: (cmd: string) => {
          if (cmd === "ctl_list_instances") {
            return Promise.resolve(JSON.stringify(payload.instances));
          }
          if (cmd === "list_connections") {
            return Promise.resolve(JSON.stringify(payload.connections));
          }
          return Promise.resolve(null);
        },
      };
      w["__CM_CONN_KEY__"] = payload.connKey;
    },
    { instances, connections, connKey: "e2e-lane" },
  );
}

/** 第二台的响应：会话桶有实质内容，其余给空对象；命中的路径记进 `hits`。 */
async function routeLaneB(page: Page, hits: string[]): Promise<void> {
  await page.route(`${LANE_B_ORIGIN}/**`, async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    hits.push(pathname);
    if (pathname.endsWith("/user-data/workspaces/current/sessions")) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          sessions: [
            {
              id: LANE_B_SESSION,
              title: "lane-b",
              draft: "",
              messages: [],
              updated_at: Date.now(),
              pinned: false,
              starred: false,
            },
          ],
          active_session_id: LANE_B_SESSION,
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
    });
  });
}

/** 打开 UI → 点「连接」建出第二条 lane，返回前台 slot 与 lane 行 locator。 */
async function openSecondLane(page: Page) {
  await installFakeShell(page);
  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "domcontentloaded",
    timeout: 20000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });

  const connect = page.getByTestId("nav-rail-ctl-connect").first();
  await expect(connect).toBeVisible({ timeout: 15000 });
  await connect.click();

  // 每条 lane 有自己一整棵子树（含侧栏），行要限定在前台那条里数。
  const activeSlot = page.locator(".lane-slot--active");
  const laneRows = activeSlot.getByTestId("nav-rail-lane-row");
  await expect(laneRows).toHaveCount(2, { timeout: 15000 });
  return { activeSlot, laneRows };
}

test("第二条 lane 的会话桶按它自己的 serve 取（#569）", async ({ page }) => {
  const hits: string[] = [];
  await installFakeShell(page);
  await routeLaneB(page, hits);
  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "domcontentloaded",
    timeout: 20000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });

  const connect = page.getByTestId("nav-rail-ctl-connect").first();
  await expect(connect).toBeVisible({ timeout: 15000 });
  await connect.click();

  // 修 #569 之前这条断言必红：请求走 `ACTIVE`，命中的是 A 台的样子。
  await expect
    .poll(() => hits.filter((p) => p.endsWith("/sessions")).length, {
      timeout: 15000,
    })
    .toBeGreaterThan(0);
});

// 切换本身仍红：`set_active` 写 `LanePhase` 时 panic（disposed 反应式值），
// 于是 `active` 从未被写入，前台 slot 停在 0。修好后把 `test.fixme` 改回 `test`。
test.fixme("点第二条 lane 会把前台切过去（#569 已知未修）", async ({
  page,
}) => {
  const hits: string[] = [];
  await routeLaneB(page, hits);
  const { activeSlot, laneRows } = await openSecondLane(page);

  await laneRows.nth(1).click();
  await expect(activeSlot).toHaveAttribute("data-lane", "1", {
    timeout: 15000,
  });
  await expect(
    activeSlot.getByTestId(`nav-session-${LANE_B_SESSION}`),
  ).toBeVisible({
    timeout: 15000,
  });

  // 切回 #1：lane-b 的会话不能跟过来（那正是「切了不换内容」的反例）。
  await activeSlot.getByTestId("nav-rail-lane-row").nth(0).click();
  await expect(activeSlot).toHaveAttribute("data-lane", "0", {
    timeout: 15000,
  });
  await expect(
    activeSlot.getByTestId(`nav-session-${LANE_B_SESSION}`),
  ).toHaveCount(0);
});
