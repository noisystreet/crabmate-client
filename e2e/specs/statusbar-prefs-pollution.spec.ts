import { expect, test } from "@playwright/test";
import {
  apiUrl,
  applyWebApiBearerHeaders,
  homeUrlWithOptionalWebBearer,
  resolveWebApiBearerToken,
} from "../fixtures/helpers";

// ---------------------------------------------------------------------------
// 复现「重启 desktop 端状态栏不显示」的 prefs 污染链：
//
//   1. 窄视口（≤768px）客户端启动即有一次 echo PUT（GET 就绪 phase→Ready 后防抖 400ms）；
//      `build_prefs_dto` 在窄/移动端返回 status_bar_visible=None，DTO `skip_serializing_if`
//      使该字段从请求体中**省略**（本意是「窄端布局不写回，隔离各端」）。
//   2. server `put_prefs_handler` 为 `Json<UserPrefs>` → `save_prefs` **整体落盘**（非按字段
//      merge）→ 省略的字段在磁盘 prefs.json 中**消失**。
//   3. 下次（宽屏桌面）启动 GET 回 None → `apply_shell_layout_prefs` 不应用 → 信号停在
//      首屏快照 false → 状态栏 footer（`<Show when=status_bar_visible>`）不渲染。
//   4. 宽屏启动自身的 echo PUT 又把 `status_bar_visible: false` 显式写回磁盘 → 失效粘性化。
//
// 注意：本用例会把目标 serve 的用户 prefs 永久污染（末态 status_bar_visible=false，
// 且 locale/theme/side_panel_view 被 seed 覆盖），仅限隔离环境运行，勿对个人 serve 执行。
// ---------------------------------------------------------------------------

function authJsonHeaders(): Record<string, string> {
  const bearer = resolveWebApiBearerToken();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (bearer) headers.Authorization = `Bearer ${bearer}`;
  return headers;
}

function parseJsonOrNull(body: string): Record<string, unknown> | null {
  try {
    return JSON.parse(body) as Record<string, unknown>;
  } catch {
    return null;
  }
}

test.describe("状态栏 prefs 污染链（窄视口 echo PUT → server 整写丢字段 → 重启隐藏）", () => {
  test("窄视口启动抹掉 status_bar_visible；宽屏重启后状态栏隐藏并被写死 false", async ({
    page,
  }) => {
    test.setTimeout(60_000);
    await applyWebApiBearerHeaders(page);

    // 1) 模拟「桌面端曾开启状态栏」：seed prefs status_bar_visible=true。
    const seed = await page.request.put(apiUrl("/user-data/prefs"), {
      data: {
        locale: "zh-Hans",
        theme: "light",
        side_panel_view: "hidden",
        side_width: 280,
        editor_layout_mode: false,
        status_bar_visible: true,
      },
      headers: authJsonHeaders(),
    });
    expect(seed.ok(), `seed prefs PUT ${seed.status()}`).toBe(true);

    // 收集页面自身发出的 prefs PUT（APIRequestContext 的 seed 请求不属于页面，不会混入）。
    const pagePrefsPuts: string[] = [];
    page.on("request", (req) => {
      if (req.method() === "PUT" && req.url().includes("/user-data/prefs")) {
        pagePrefsPuts.push(req.postData() ?? "");
      }
    });

    // 2) 窄视口打开页面：启动 echo PUT 应省略 status_bar_visible。
    await page.setViewportSize({ width: 480, height: 800 });
    await page.goto(homeUrlWithOptionalWebBearer("/"), {
      waitUntil: "networkidle",
      timeout: 20000,
    });
    await page.waitForSelector('[data-testid="chat-composer-input"]', {
      timeout: 15000,
    });

    await expect
      .poll(
        () =>
          pagePrefsPuts.filter((body) => {
            const parsed = parseJsonOrNull(body);
            return parsed !== null && !("status_bar_visible" in parsed);
          }).length,
        {
          timeout: 10_000,
          message: "窄视口启动 echo PUT 应省略 status_bar_visible 字段",
        },
      )
      .toBeGreaterThan(0);

    // 3) server 整写覆盖：磁盘 prefs 丢失该字段。
    const diskAfterNarrow = await page.request.get(apiUrl("/user-data/prefs"), {
      headers: authJsonHeaders(),
    });
    expect(diskAfterNarrow.ok()).toBe(true);
    const narrowDisk = (await diskAfterNarrow.json()) as {
      status_bar_visible?: boolean;
    };
    expect(
      narrowDisk.status_bar_visible ?? null,
      "服务端 prefs 的 status_bar_visible 被整写覆盖为缺失",
    ).toBeNull();

    // 4) 宽屏「重启」（reload）：GET 回 None → 信号停在首屏快照 false → 状态栏不渲染。
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.reload({ waitUntil: "networkidle", timeout: 20000 });
    await page.waitForSelector('[data-testid="chat-composer-input"]', {
      timeout: 15000,
    });
    await expect(page.locator('[data-testid="status-bar"]')).toHaveCount(0);

    // 5) 宽屏启动自身的 echo PUT 把 false 显式写回 → 失效粘性化。
    await expect
      .poll(
        () =>
          pagePrefsPuts.some((body) => {
            const parsed = parseJsonOrNull(body);
            return parsed?.status_bar_visible === false;
          }),
        {
          timeout: 10_000,
          message: "宽屏重启 echo PUT 应把 status_bar_visible=false 写回磁盘",
        },
      )
      .toBe(true);

    const diskAfterWide = await page.request.get(apiUrl("/user-data/prefs"), {
      headers: authJsonHeaders(),
    });
    expect(diskAfterWide.ok()).toBe(true);
    const wideDisk = (await diskAfterWide.json()) as {
      status_bar_visible?: boolean;
    };
    expect(wideDisk.status_bar_visible, "磁盘已固化为显式 false（粘性）").toBe(
      false,
    );
  });
});
