/**
 * `/user-data` API 往返回归。
 *
 * 由 `victauri_user_data.rs`(4) 迁入（#152 Wave 3）。原用例通过 webview 内
 * `fetch()` 打后端再断言 JSON —— 本质是接口契约，不需要页面，也不需要壳；
 * 这里用 `page.request` 直接发同样请求。
 */

import { test, expect } from "@playwright/test";
import { apiUrl, resolveWebApiBearerToken } from "../fixtures/helpers";

/** 跨 Origin 且 serve 启用 Bearer 时要带上（与页面内 fetch 同源口径）。 */
function authHeaders(): Record<string, string> {
  const bearer = resolveWebApiBearerToken();
  return bearer ? { Authorization: `Bearer ${bearer}` } : {};
}

function jsonHeaders(): Record<string, string> {
  return { "Content-Type": "application/json", ...authHeaders() };
}

test("GET/PUT prefs 往返", async ({ page }) => {
  const before = await page.request.get(apiUrl("/user-data/prefs"), {
    headers: authHeaders(),
  });
  expect(before.ok()).toBe(true);
  expect(typeof (await before.json())).toBe("object");

  const put = await page.request.put(apiUrl("/user-data/prefs"), {
    headers: jsonHeaders(),
    data: {
      locale: "en",
      theme: "crabmate-dark",
      side_panel_view: "workspace",
      side_width: 300,
    },
  });
  expect(put.ok()).toBe(true);

  const after = await page.request.get(apiUrl("/user-data/prefs"), {
    headers: authHeaders(),
  });
  const prefs = (await after.json()) as {
    locale?: string;
    theme?: string;
    side_width?: number;
  };
  expect(prefs.locale).toBe("en");
  expect(prefs.theme).toBe("crabmate-dark");
  expect(prefs.side_width).toBe(300);
});

test("PUT/GET 当前工作区会话", async ({ page }) => {
  // 先把工作区重置到默认，避免上一个用例留下的桶
  await page.request.post(apiUrl("/workspace"), {
    headers: jsonHeaders(),
    data: { path: null },
  });

  const put = await page.request.put(
    apiUrl("/user-data/workspaces/current/sessions"),
    {
      headers: jsonHeaders(),
      data: {
        sessions: [
          {
            id: "s_e2e_ud",
            title: "E2E user-data",
            draft: "",
            messages: [],
            updated_at: 1,
            pinned: false,
            starred: false,
          },
        ],
        active_session_id: "s_e2e_ud",
      },
    },
  );
  expect(put.ok()).toBe(true);

  const got = await page.request.get(
    apiUrl("/user-data/workspaces/current/sessions"),
    { headers: authHeaders() },
  );
  const bucket = (await got.json()) as {
    active_session_id?: string;
    sessions?: { id?: string }[];
  };
  expect(bucket.active_session_id).toBe("s_e2e_ud");
  expect((bucket.sessions ?? []).some((s) => s.id === "s_e2e_ud")).toBe(true);
});

// 注：原 `victauri_user_data` 的另两例（MCP servers PUT 派生 slug /
// MCP status 列表）**未迁**：它们依赖 `schema_version` + `tool_timeout_secs` 这版
// MCP 契约，而 CI 的 serve 钉在 `v0.8.0`（早于该契约），PUT 直接失败。
// MCP 契约属 Server 仓，应由 Server 侧覆盖（#152 已记录）。
