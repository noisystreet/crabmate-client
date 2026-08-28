/**
 * 真实 LLM 端到端测试：思考回显复现（reasoning 被模型在正文中再次输出）
 *
 * 背景：部分思考型模型（本地 thinking model / DeepSeek 推理等）既通过
 * `reasoning_content` 流式返回推理，又会在 `content` 正文里以
 * 「### 思考过程 … --- …」章节**逐字回显**同一段推理。
 * 客户端把 `reasoning_text` 与正文分开保存/展示后，就会出现「思考过程显示两次」。
 *
 * 本用例在有真实模型凭证时**复现**该现象：
 *   - 断言 thinking 开启时 `reasoning_text` 非空（推理管线工作）；
 *   - 若模型在正文回显思考章节，断言该章节内容与 `reasoning_text` 重叠，
 *     证明重复来自**模型正文输出**而非客户端生成（客户端从不写「### 思考过程」文本）；
 *   - 模型未回显时记录注解并跳过复现断言（依赖模型行为，不算失败）。
 *
 * 前置条件：
 *   1. `crabmate serve` 在 127.0.0.1:8080 运行
 *   2. 模型密钥：环境变量 `API_KEY` / 本地 TOML，**或**本机钥匙串/E2E 注入已有 `client_llm` 密钥
 *   3. 若服务端启用了 Web Bearer：`CM_WEB_API_BEARER_TOKEN`
 *
 * 运行方式：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-thinking-echo.spec.ts
 */

import { test, expect } from "@playwright/test";
import {
  apiUrl,
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
  waitForReadyWhileApproving,
} from "../fixtures/helpers";
import {
  PersistedMessage,
  PersistedSession,
} from "../fixtures/session_assertions";

const API_KEY = resolveOptionalApiKeyFromEnvOrToml();
const SID = "s_e2e_real_thinking_echo";

/**
 * 模型端点可通过环境变量覆盖（默认 DeepSeek）：
 *   CRABMATE_E2E_LLM_API_BASE  例如 http://127.0.0.1:11434/v1（本地 Ollama/vLLM）
 *   CRABMATE_E2E_LLM_MODEL     例如你的本地思考模型名
 */
const LLM_API_BASE =
  process.env.CRABMATE_E2E_LLM_API_BASE?.trim() || "https://api.deepseek.com";
const LLM_MODEL =
  process.env.CRABMATE_E2E_LLM_MODEL?.trim() || "deepseek-v4-flash";

const THINKING_CONFIG = {
  apiBase: LLM_API_BASE,
  model: LLM_MODEL,
  thinkingMode: "on",
};

/** 触发多步推理、且显式要求把推理写进「### 思考过程」章节的问题（逼模型回显）。 */
const QUESTION_WATER_JUG =
  "请先仔细思考、不要急着给答案：一个 3 升瓶和一个 5 升瓶，如何恰好量出 4 升水？把推理过程写在「### 思考过程」章节里，再给最终答案。";
const QUESTION_ONE_PLUS_ONE = "请一步步推理：为什么 1+1 等于 2？";

type ThinkingMessage = PersistedMessage & {
  reasoning_text?: string;
  created_at?: number;
};

/** 思考回显章节标题（模型正文中的常见形态）。 */
const ECHO_HEADINGS = [
  "### 思考过程",
  "**思考过程**",
  "思考过程：",
  "### Thinking",
  "**Thinking**",
];

function hasEchoHeading(text: string): boolean {
  return ECHO_HEADINGS.some((heading) => text.includes(heading));
}

/** 去掉空白后的归一化（比较回显与 reasoning 是否重叠）。 */
function normalized(s: string): string {
  return s.replace(/\s+/g, "");
}

/** 模糊重叠：归一化后互相包含，或存在 ≥ 12 字符公共子串。 */
function fuzzyOverlap(a: string, b: string): boolean {
  const na = normalized(a);
  const nb = normalized(b);
  if (na.length < 8 || nb.length < 8) {
    return false;
  }
  if (na.includes(nb) || nb.includes(na)) {
    return true;
  }
  const probe = 12;
  for (let i = 0; i + probe <= na.length; i++) {
    if (nb.includes(na.slice(i, i + probe))) {
      return true;
    }
  }
  return false;
}

/**
 * 拉取当前工作区**全部**会话，按发送的问题文本定位本测试的会话，
 * 返回其中最新一条带正文的非工具 assistant。
 *
 * 前端「冷启动开新会话」会把聊天写入自动生成的会话 id（而非种子 SID），
 * 且 bucket 可能与桌面 app 会话混用，故用问题文本精确定位、避免取错会话。
 */
async function fetchAssistantForQuestion(
  page: import("@playwright/test").Page,
  question: string,
): Promise<ThinkingMessage | null> {
  const needle = normalized(question).slice(0, 20);
  const deadline = Date.now() + 60_000;
  // 记录「仅正文、无 reasoning」的兜底，超时后返回它让断言给出清晰失败。
  let textOnly: ThinkingMessage | null = null;
  while (Date.now() < deadline) {
    const all = await page.evaluate(async (url) => {
      const response = await fetch(url);
      const data = await response.json();
      return (
        (data.current?.sessions as PersistedSession[] | undefined) ??
        (data.sessions as PersistedSession[] | undefined) ??
        []
      );
    }, apiUrl("/user-data/workspaces/current/sessions"));
    for (const s of all) {
      const messages = (s.messages ?? []) as ThinkingMessage[];
      const asked = messages.some(
        (m) => m.role === "user" && normalized(m.text ?? "").includes(needle),
      );
      if (!asked) {
        continue;
      }
      // reasoning 可能晚于 text 落盘：优先返回带 reasoning 的助手消息。
      const withReasoning = messages.find(
        (m) =>
          m.role === "assistant" &&
          !m.is_tool &&
          (m.text ?? "").trim().length > 0 &&
          (m.reasoning_text ?? "").trim().length > 10,
      );
      if (withReasoning) {
        return withReasoning;
      }
      const anyText = messages.find(
        (m) =>
          m.role === "assistant" &&
          !m.is_tool &&
          (m.text ?? "").trim().length > 0,
      );
      if (anyText) {
        textOnly = anyText;
      }
    }
    await page.waitForTimeout(1_000);
  }
  return textOnly;
}

test.describe("真实 LLM：思考回显复现", () => {
  test("thinking 开启时 reasoning_text 与正文思考章节并存（模型回显）", async ({
    page,
  }) => {
    test.setTimeout(300_000);
    await setupRealLLMSessionPreferringKeyring(
      page,
      SID,
      API_KEY,
      THINKING_CONFIG,
    );
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    // 触发多步推理的问题，逼出 reasoning_content。
    await sendMessage(page, QUESTION_WATER_JUG);

    await waitForReadyWhileApproving(page, 240_000);

    const assistant = await fetchAssistantForQuestion(page, QUESTION_WATER_JUG);
    expect(
      assistant,
      "流完成后应持久化一条带正文的 assistant 消息",
    ).not.toBeNull();

    const reasoning = (assistant?.reasoning_text ?? "").trim();
    const text = (assistant?.text ?? "").trim();

    // 1) 推理管线工作：thinking 开启时 reasoning_text 非空。
    expect(
      reasoning.length,
      `reasoning_text 应为非空（thinking=on）：${JSON.stringify(
        assistant,
      )?.slice(0, 200)}`,
    ).toBeGreaterThan(10);

    // 2) 复现：模型是否在正文回显了思考章节。
    const echo = hasEchoHeading(text);
    test.info().annotations.push({
      type: "repro",
      description: `reasoning_text(${reasoning.length}字符) 正文(${text.length}字符) 回显章节=${echo}`,
    });

    if (!echo) {
      test.info().annotations.push({
        type: "info",
        description:
          "本模型未在正文回显思考章节，未观察到重复；复现断言跳过（依赖模型行为）。正文开头：" +
          text.slice(0, 120),
      });
      return;
    }

    // 3) 回显内容与 reasoning_text 重叠 → 重复来自模型正文输出。
    const headingIdx = [
      "### 思考过程",
      "**思考过程**",
      "思考过程：",
      "### Thinking",
      "**Thinking**",
    ]
      .map((h) => text.indexOf(h))
      .filter((i) => i >= 0)
      .sort((a, b) => a - b)[0];
    const echoSection = headingIdx >= 0 ? text.slice(headingIdx) : text;
    expect(
      fuzzyOverlap(reasoning, echoSection),
      "正文中的思考章节应与 reasoning_text 内容重叠（重复=模型回显）\n" +
        `reasoning: ${reasoning.slice(0, 120)}\n正文思考章节: ${echoSection.slice(
          0,
          120,
        )}`,
    ).toBe(true);
  });

  test("UI：回显思考章节被展示层剥除，思考只出现一次", async ({ page }) => {
    test.setTimeout(300_000);
    await setupRealLLMSessionPreferringKeyring(
      page,
      SID + "_ui",
      API_KEY,
      THINKING_CONFIG,
    );
    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    await sendMessage(page, QUESTION_ONE_PLUS_ONE);
    await waitForReadyWhileApproving(page, 240_000);

    // 折叠思考块（summary 标签「思考过程」）应存在。
    const thinkSummary = page.locator(".chat-tui-think-summary");
    await expect(thinkSummary).toBeVisible({ timeout: 10_000 });

    // 正文是否回显思考章节：若持久化正文包含标题，展示层应已剥除回显——
    // 气泡里「思考过程」只出现在折叠块摘要一次（修复后的行为）。
    const assistant = await fetchAssistantForQuestion(
      page,
      QUESTION_ONE_PLUS_ONE,
    );
    const text = (assistant?.text ?? "").trim();
    if (hasEchoHeading(text)) {
      const bodies = await page.evaluate(() =>
        [
          ...document.querySelectorAll<HTMLElement>(
            "section.chat-tui-turn--assistant",
          ),
        ]
          .map((el) => (el.innerText ?? "").replace(/\s+/g, " ").trim())
          .filter((t) => t.includes("思考过程")),
      );
      expect(
        bodies.length,
        `回显章节应被展示层剥除，「思考过程」只在折叠块出现一次：${JSON.stringify(
          bodies,
        )}`,
      ).toBe(1);
    } else {
      test.info().annotations.push({
        type: "info",
        description: "本模型未回显思考章节，仅断言折叠思考块可见。",
      });
    }
  });
});
