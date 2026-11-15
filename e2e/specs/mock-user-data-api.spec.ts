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

test("PUT MCP 服务器会派生 slug，且不回显 command", async ({ page }) => {
  const put = await page.request.put(apiUrl("/user-data/mcp-servers"), {
    headers: jsonHeaders(),
    data: {
      schema_version: 1,
      global_enabled: true,
      tool_timeout_secs: 60,
      servers: [
        {
          id: "mcp_e2e_ud",
          name: "E2E Test Server",
          slug: "",
          command: "true",
          enabled: false,
          created_at_ms: 0,
          updated_at_ms: 0,
        },
      ],
    },
  });
  expect(put.ok()).toBe(true);

  const got = await page.request.get(apiUrl("/user-data/mcp-servers"), {
    headers: authHeaders(),
  });
  const file = (await got.json()) as {
    global_enabled?: boolean;
    servers?: Record<string, unknown>[];
  };
  expect(file.global_enabled).toBe(true);
  const servers = file.servers ?? [];
  expect(servers).toHaveLength(1);
  expect(servers[0].id).toBe("mcp_e2e_ud");
  expect(servers[0].slug).toBe("e2e_test_server");
  expect(servers[0].enabled).toBe(false);
  expect(servers[0].has_command).toBe(true);
  // command 本身不得回显给客户端
  expect(servers[0].command).toBeUndefined();
});

test("GET MCP status 列出已配置服务器", async ({ page }) => {
  const put = await page.request.put(apiUrl("/user-data/mcp-servers"), {
    headers: jsonHeaders(),
    data: {
      schema_version: 1,
      global_enabled: false,
      tool_timeout_secs: 45,
      servers: [
        {
          id: "mcp_e2e_status",
          name: "Status Probe",
          slug: "status_probe",
          command: "true",
          enabled: true,
          created_at_ms: 0,
          updated_at_ms: 0,
        },
      ],
    },
  });
  expect(put.ok()).toBe(true);

  const file = (await (
    await page.request.get(apiUrl("/user-data/mcp-servers"), {
      headers: authHeaders(),
    })
  ).json()) as { global_enabled?: boolean };
  expect(file.global_enabled).toBe(false);

  const status = (await (
    await page.request.get(apiUrl("/user-data/mcp-servers/status"), {
      headers: authHeaders(),
    })
  ).json()) as {
    global_enabled?: boolean;
    tool_timeout_secs?: number;
    servers?: Record<string, unknown>[];
  };
  expect(status.global_enabled).toBe(false);
  expect(status.tool_timeout_secs).toBe(45);

  const row = (status.servers ?? []).find((r) => r.id === "mcp_e2e_status");
  expect(row, "status 里应列出该 MCP 服务器").toBeTruthy();
  expect(row?.slug).toBe("status_probe");
  expect(row?.enabled).toBe(true);
  expect(row?.connected).toBe(false);
});
