import { expect, Page, Route } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";

// ---------------------------------------------------------------------------
// 辅助函数：seed 会话 & prefs，mock SSE 拦截
// ---------------------------------------------------------------------------

/** 设置 prefs、session，重载页面等待输入框就绪。*/
export async function seedSession(page: Page, sid: string) {
  await applyWebApiBearerHeaders(page);
  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "networkidle",
    timeout: 20000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });

  await page.evaluate(() =>
    fetch("/user-data/prefs", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        locale: "zh",
        theme: "light",
        side_panel_view: "hidden",
        side_width: 280,
        editor_layout_mode: false,
        status_bar_visible: true,
      }),
    }).catch(() => {}),
  );

  await page.evaluate((s: string) => {
    const body = JSON.stringify({
      sessions: [
        {
          id: s,
          title: "e2e",
          draft: "",
          messages: [],
          updated_at: Date.now(),
          pinned: false,
          starred: false,
        },
      ],
      active_session_id: s,
    });
    return fetch("/user-data/workspaces/current/sessions", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body,
    }).catch(() => {});
  }, sid);

  await page.reload({ waitUntil: "networkidle", timeout: 20000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });
}

/** 在页面中发送消息（填值 + Enter）。*/
export async function sendMessage(page: Page, text: string) {
  await page.focus('[data-testid="chat-composer-input"]');
  await page.evaluate((msg: string) => {
    const el = document.querySelector<HTMLTextAreaElement>(
      '[data-testid="chat-composer-input"]',
    );
    if (!el) return;
    const s = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!;
    s.call(el, msg);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, text);
  await page.keyboard.press("Enter");
}

/** 注册 page.route 拦截 /chat/stream POST -> 返回 mock SSE 正文。
 * 必须携带 x-conversation-id / x-stream-job-id 头，前端据此初始化流式会话。 */
export function installMockSse(
  page: Page,
  sseBody: string,
  convId = "e2e-conv",
) {
  return page.route("**/chat/stream", (route) => {
    if (route.request().method() !== "POST") {
      return route.continue();
    }
    return route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream; charset=utf-8",
        "x-conversation-id": convId,
        "x-stream-job-id": "1",
      },
      body: sseBody,
    });
  });
}

/** 用浏览器 ReadableStream 分批返回 SSE，覆盖真实流式渲染与滚动时序。 */
export async function installDelayedMockSse(
  page: Page,
  sseEvents: string[],
  delayMs = 35,
  convId = "e2e-streaming-conv",
) {
  await page.evaluate(
    ({ events, delay, conversationId }) => {
      const originalFetch = window.fetch.bind(window);
      const mockedFetch: typeof window.fetch = async (input, init) => {
        const requestUrl =
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        const method =
          init?.method ?? (input instanceof Request ? input.method : "GET");
        if (
          !requestUrl.includes("/chat/stream") ||
          method.toUpperCase() !== "POST"
        ) {
          return originalFetch(input, init);
        }

        const encoder = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
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
        return new Response(body, {
          status: 200,
          headers: {
            "content-type": "text/event-stream; charset=utf-8",
            "x-conversation-id": conversationId,
            "x-stream-job-id": "1",
          },
        });
      };
      window.fetch = mockedFetch;
    },
    { events: sseEvents, delay: delayMs, conversationId: convId },
  );
}

/** 等待页面中出现指定文本（超时 ms）。*/
export async function waitForText(page: Page, text: string, timeoutMs = 20000) {
  await page.waitForFunction((t) => document.body.innerText.includes(t), text, {
    timeout: timeoutMs,
  });
}

// ---------------------------------------------------------------------------
// 真实 LLM 测试辅助函数
// ---------------------------------------------------------------------------

/** 健康检查：poll /health 直到后端就绪。*/
export async function waitForHealth(
  baseUrl: string,
  maxRetries = 15,
  intervalMs = 1000,
) {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const resp = await fetch(`${baseUrl}/health`);
      if (resp.ok) return;
    } catch {
      // 后端尚未就绪
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error(`后端 ${baseUrl} 在 ${maxRetries * intervalMs}ms 内未就绪`);
}

/** 环境变量 / 本地 TOML 中的明文模型密钥（可选）。不含系统钥匙串。 */
export function resolveOptionalApiKeyFromEnvOrToml(): string {
  const env = process.env.API_KEY?.trim();
  if (env) return env;

  const projectRoot = path.resolve(process.cwd(), "..");
  for (const name of ["config.toml", ".agent_demo.toml"] as const) {
    const fromFile = readApiKeyFromToml(path.join(projectRoot, name));
    if (fromFile) return fromFile;
  }
  return "";
}

/** 浏览器访问受保护 `/user-data` 等接口所需的 Web API Bearer（≠ 模型 API_KEY）。 */
export function resolveWebApiBearerToken(): string {
  return (process.env.CM_WEB_API_BEARER_TOKEN || "").trim();
}

/** 首页 URL；若设了 `CM_WEB_API_BEARER_TOKEN` 则带上 hash 交接（供 WASM 鉴权层）。 */
export function homeUrlWithOptionalWebBearer(pathname = "/"): string {
  const bearer = resolveWebApiBearerToken();
  if (!bearer) return pathname;
  const sep = pathname.includes("#") ? "&" : "#";
  return `${pathname}${sep}cm_web_api_bearer=${encodeURIComponent(bearer)}`;
}

/**
 * 为 Playwright 页内 `fetch`（含 `page.evaluate`）注入 Authorization。
 * 仅 localStorage/hash 不够：裸 fetch 不走前端 `api` 封装。
 */
export async function applyWebApiBearerHeaders(page: Page): Promise<void> {
  const bearer = resolveWebApiBearerToken();
  if (!bearer) return;
  await page.setExtraHTTPHeaders({
    Authorization: `Bearer ${bearer}`,
  });
}

/** 经已鉴权页面探测服务端钥匙串是否已有 `client_llm`（不读明文）。 */
export async function isClientLlmSetViaSecretsStatus(
  page: Page,
): Promise<boolean> {
  return page.evaluate(async () => {
    const response = await fetch("/user-data/secrets/status");
    if (!response.ok) return false;
    const data = (await response.json()) as {
      client_llm?: { set?: boolean };
    };
    return Boolean(data.client_llm?.set);
  });
}

/**
 * 真实 LLM 凭证是否可用：明文 `API_KEY` / TOML，或服务端钥匙串已有 `client_llm`。
 * 调用前须已完成带 Web Bearer 的页面导航（见 `setupRealLLMSessionPreferringKeyring`）。
 */
export async function ensureRealLlmModelCredential(
  page: Page,
  apiKey: string,
): Promise<boolean> {
  if (apiKey.trim()) return true;
  return isClientLlmSetViaSecretsStatus(page);
}

function readApiKeyFromToml(filePath: string): string {
  try {
    const raw = fs.readFileSync(filePath, "utf8");
    const inAgentSection: string[] = [];
    let inAgent = false;
    for (const line of raw.split("\n")) {
      const trimmed = line.trim();
      if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
        inAgent = trimmed.slice(1, -1).trim() === "agent";
        continue;
      }
      if (inAgent && trimmed.startsWith("api_key")) {
        const eqIdx = trimmed.indexOf("=");
        if (eqIdx === -1) continue;
        let val = trimmed.slice(eqIdx + 1).trim();
        if (
          (val.startsWith('"') && val.endsWith('"')) ||
          (val.startsWith("'") && val.endsWith("'"))
        ) {
          val = val.slice(1, -1);
        }
        if (val) inAgentSection.push(val);
      }
    }
    if (inAgentSection.length > 0) {
      return inAgentSection[inAgentSection.length - 1];
    }
  } catch {
    /* ignore */
  }
  return "";
}

export type RealLlmSetupOptions = {
  apiBase?: string;
  model?: string;
  contextTokens?: string;
  thinkingMode?: string;
};

/** 为真实 LLM 测试设置会话：API key + LLM 配置 + prefs + 空会话。
 *
 * 流程：
 *   1. 导航至首页并等待输入框就绪
 *   2. 设置 client_llm.api_key 到后端 secrets 存储
 *   3. 设置 LLM 覆盖配置（api_base、model 等）
 *   4. 设置用户偏好
 *   5. 创建空会话
 *   6. 重载页面等待 UI 就绪
 */
export async function setupRealLLMSession(
  page: Page,
  sid: string,
  apiKey: string,
  llmConfig?: RealLlmSetupOptions,
) {
  const cfg = {
    apiBase: llmConfig?.apiBase ?? "https://api.deepseek.com",
    model: llmConfig?.model ?? "deepseek-v4-flash",
    contextTokens: llmConfig?.contextTokens ?? "1000000",
    thinkingMode: llmConfig?.thinkingMode ?? "off",
  };

  await applyWebApiBearerHeaders(page);
  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "networkidle",
    timeout: 20000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });

  // 设置 API key
  await page.evaluate(
    (key: string) =>
      fetch("/user-data/secrets/client-llm", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ api_key: key }),
      }),
    apiKey,
  );

  // 设置 LLM 覆盖配置
  await page.evaluate(
    (c: {
      apiBase: string;
      model: string;
      contextTokens: string;
      thinkingMode: string;
    }) =>
      fetch("/user-data/llm-overrides", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_llm: {
            api_base: c.apiBase,
            model: c.model,
            llm_context_tokens: c.contextTokens,
            llm_thinking_mode: c.thinkingMode,
          },
        }),
      }),
    cfg,
  );

  // 设置 prefs
  await page.evaluate(() =>
    fetch("/user-data/prefs", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        locale: "zh",
        theme: "light",
        side_panel_view: "hidden",
        side_width: 280,
        editor_layout_mode: false,
        status_bar_visible: true,
      }),
    }).catch(() => {}),
  );

  // 创建空会话
  await page.evaluate((s: string) => {
    const body = JSON.stringify({
      sessions: [
        {
          id: s,
          title: "e2e-real-llm",
          draft: "",
          messages: [],
          updated_at: Date.now(),
          pinned: false,
          starred: false,
        },
      ],
      active_session_id: s,
    });
    return fetch("/user-data/workspaces/current/sessions", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body,
    }).catch(() => {});
  }, sid);

  // 重载等待 UI（Bearer 已在首屏 hash 交接后写入本页存储）
  await page.reload({ waitUntil: "networkidle", timeout: 20000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15000,
  });
}

/**
 * 优先明文 `API_KEY`；否则不写钥匙串，复用服务端已有 `client_llm`。
 * 旧 `$XDG_DATA_HOME/crabmate/secrets/client_llm` 明文文件路径已废弃，勿再读取。
 */
export async function setupRealLLMSessionPreferringKeyring(
  page: Page,
  sid: string,
  apiKey?: string,
  llmConfig?: RealLlmSetupOptions,
) {
  const key = (apiKey ?? resolveOptionalApiKeyFromEnvOrToml()).trim();
  if (key) {
    await setupRealLLMSession(page, sid, key, llmConfig);
    return;
  }

  const cfg = {
    apiBase: llmConfig?.apiBase ?? "https://api.deepseek.com",
    model: llmConfig?.model ?? "deepseek-v4-flash",
    contextTokens: llmConfig?.contextTokens ?? "1000000",
    thinkingMode: llmConfig?.thinkingMode ?? "off",
  };

  await applyWebApiBearerHeaders(page);
  await page.goto(homeUrlWithOptionalWebBearer("/"), {
    waitUntil: "networkidle",
    timeout: 20_000,
  });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });

  await page.evaluate(
    (c: {
      apiBase: string;
      model: string;
      contextTokens: string;
      thinkingMode: string;
    }) =>
      fetch("/user-data/llm-overrides", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_llm: {
            api_base: c.apiBase,
            model: c.model,
            llm_context_tokens: c.contextTokens,
            llm_thinking_mode: c.thinkingMode,
          },
        }),
      }),
    cfg,
  );

  await page.evaluate(() =>
    fetch("/user-data/prefs", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        locale: "zh",
        theme: "light",
        side_panel_view: "hidden",
        side_width: 280,
        editor_layout_mode: false,
        status_bar_visible: true,
      }),
    }).catch(() => {}),
  );

  await page.evaluate((s: string) => {
    const body = JSON.stringify({
      sessions: [
        {
          id: s,
          title: "e2e-real-llm",
          draft: "",
          messages: [],
          updated_at: Date.now(),
          pinned: false,
          starred: false,
        },
      ],
      active_session_id: s,
    });
    return fetch("/user-data/workspaces/current/sessions", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body,
    }).catch(() => {});
  }, sid);

  await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });
}

/** 等待状态栏出现「就绪」文本（真实 LLM 调用可能耗时较长）。*/
export async function waitForReady(page: Page, timeout = 120_000) {
  // Playwright 签名为 waitForFunction(fn, arg?, options?)；不可把 options 当作第 2 参。
  await page.waitForFunction(
    () => document.body.innerText.includes("就绪"),
    undefined,
    { timeout },
  );
}

/** 等待就绪；若弹出审批对话框则点「始终允许」。 */
export async function waitForReadyWhileApproving(
  page: Page,
  timeoutMs = 180_000,
) {
  const deadline = Date.now() + timeoutMs;
  const statusBar = page.locator('[data-testid="status-bar"]');
  const approvalModal = page.locator('[data-testid="approval-modal"]');
  while (Date.now() < deadline) {
    if (await approvalModal.isVisible().catch(() => false)) {
      const allowAlways = page.locator('[data-testid="approval-allow-always"]');
      if (await allowAlways.isVisible().catch(() => false)) {
        await allowAlways.click();
      } else {
        await page.locator('[data-testid="approval-allow-once"]').click();
      }
      await expect(approvalModal).not.toBeVisible({ timeout: 10_000 });
      continue;
    }
    if ((await statusBar.textContent())?.includes("就绪")) return;
    await page.waitForTimeout(250);
  }
  throw new Error(`流在 ${timeoutMs}ms 内未进入就绪状态`);
}
