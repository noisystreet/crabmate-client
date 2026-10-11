/**
 * Mock SSE 端到端测试：状态栏「会话」上下文用量百分比
 *
 * 目的：用确定性 mock 数据验证底栏同口径展示，取代原真实 LLM 用例
 * （`real-llm-statusbar-context.spec.ts`，已删除）——
 *   1. 单轮快照后，状态栏芯片显示实测 `used / max (xx.x%)`，
 *      used = used_input_tokens（含固定开销），max = 模型原始 max_input_tokens；
 *   2. 同一用量下换小窗口，使占比 >30%，证明前端无 30% 上限；
 *   3. 20 轮会话（中途 message 回落）下显示值单调不减（用量水位），且百分比恒等于 used/max。
 *
 * 数据来源：RUN_FINISHED 帧携带的 `tiktokenPromptTokens`（与真实服务端同形）写入
 * `chat.conversation_prompt_tokens`；`x-conversation-id` 头先建立绑定会话的
 * `server_conversation_id`，快照才能挂到当前活动会话上。
 *
 * 运行方式（前置：`cargo run -- serve` 在 127.0.0.1:8080 运行；无需 API_KEY）：
 *   cd e2e && npx playwright test specs/mock-statusbar-context.spec.ts
 */

import { test, expect, type Page } from "@playwright/test";
import {
  MOCK_SSE_CORS_EXPOSE,
  seedSession,
  sendMessage,
} from "../fixtures/helpers";

const CONTEXT_VALUE_SELECTOR =
  '[data-testid="status-bar"] .status-chip-context .status-chip-value';
const CONTEXT_METER_SELECTOR =
  '[data-testid="status-bar"] .status-context-meter';

/** 显示比例（紧凑格式）允许的误差：`21.2k` 的千位舍入 + 百分比一位小数。 */
const PCT_TOLERANCE = 0.3;

type ContextReading = {
  raw: string;
  estimate: boolean;
  /** used = used_input_tokens（含固定开销，已含用量水位）。 */
  used: number;
  /** max = 模型原始 max_input_tokens（文本分母）。 */
  cap: number;
  /** used / max * 100。 */
  pct: number;
};

/** 单轮 mock 的 token 快照参数。 */
type TurnTokens = {
  /** used_input_tokens（含固定开销）。 */
  used: number;
  /** 服务端原始 max_input_tokens。 */
  maxInput: number;
  /** message_tokens（会话内容主体）。 */
  message: number;
  attachment?: number;
  /** 助手正文（非 JSON，按正文增量处理）。 */
  answer: string;
  /** 本轮中途注入的 `context_trim` 事件 `after_tokens`（服务端压缩 / 裁剪后的 used_input_tokens）。 */
  trimAfterTokens?: number;
};

/** 构造与真实服务端同形的 `tiktokenPromptTokens` 子对象。 */
function tiktokenObject(t: TurnTokens): Record<string, number | string> {
  return {
    prompt_tokens: t.used,
    tiktoken_model: "gpt-4o",
    used_input_tokens: t.used,
    max_input_tokens: t.maxInput,
    reserved_output_tokens: 4096,
    message_tokens: t.message,
    tool_schema_tokens: t.used - t.message,
    attachment_tokens: t.attachment ?? 0,
    counting_source: "heuristic",
  };
}

/** 单轮 SSE 正文：answer 相 → [可选 context_trim] → 正文 → RUN_FINISHED（携带 tiktoken 快照）。 */
function sseBodyForTurn(t: TurnTokens): string {
  const runFinished = {
    type: "RUN_FINISHED",
    threadId: "",
    runId: "1",
    tiktokenPromptTokens: tiktokenObject(t),
  };
  const frames: string[] = [
    '{"type":"CUSTOM","customType":"assistant_answer_phase"}',
  ];
  if (t.trimAfterTokens !== undefined) {
    // 与真实服务端同形：`kind=context_trim`，`detail` 为 JSON 字符串（after_tokens 供水位回落）。
    frames.push(
      JSON.stringify({
        type: "CUSTOM",
        customType: "timeline_log",
        data: {
          kind: "context_trim",
          title: "已裁剪历史",
          detail: JSON.stringify({ after_tokens: t.trimAfterTokens }),
        },
      }),
    );
  }
  frames.push(t.answer);
  frames.push(JSON.stringify(runFinished));
  return frames.map((body, i) => `id: ${i + 1}\ndata: ${body}\n\n`).join("");
}

/** 按调用次数逐轮返回不同 SSE 正文（不足则沿用最后一轮）。 */
function installSequencedMockSse(
  page: Page,
  turns: TurnTokens[],
  convId: string,
) {
  let callCount = 0;
  return page.route("**/chat/stream", (route) => {
    if (route.request().method() !== "POST") {
      return route.continue();
    }
    const idx = Math.min(callCount, turns.length - 1);
    callCount++;
    return route.fulfill({
      status: 200,
      headers: {
        "content-type": "text/event-stream; charset=utf-8",
        ...MOCK_SSE_CORS_EXPOSE,
        "x-conversation-id": convId,
        "x-stream-job-id": String(callCount),
      },
      body: sseBodyForTurn(turns[idx]),
    });
  });
}

/** 解析紧凑 token：`21200` → 21200；`21.2k` → 21200。 */
function parseCompactTokens(token: string): number {
  return token.endsWith("k")
    ? Math.round(Number.parseFloat(token) * 1000)
    : Number.parseFloat(token);
}

/** 解析状态栏会话芯片文本：`[~]used / max (xx.x%)`（used/max 为纯整数或 `N.Nk`）。 */
function parseContextText(raw: string): ContextReading | null {
  const text = raw.trim();
  const m = /^(~?)([\d.]+k?)\s*\/\s*([\d.]+k?)\s*\(\s*([\d.]+)%\s*\)$/.exec(
    text,
  );
  if (!m) return null;
  return {
    raw: text,
    estimate: m[1] === "~",
    used: parseCompactTokens(m[2]),
    cap: parseCompactTokens(m[3]),
    pct: Number.parseFloat(m[4]),
  };
}

/**
 * 读取状态栏会话芯片，轮询至「实测值」（非 `~` 基线估算）且 used 等于 `expectUsed`。
 *
 * 传 `expectUsed` 可把 `~` 基线（其 used = system 基线用量）与新到的实测快照区分开。
 */
async function readChip(
  page: Page,
  expectUsed: number,
): Promise<ContextReading> {
  const chip = page.locator(CONTEXT_VALUE_SELECTOR);
  await expect(chip).toBeVisible({ timeout: 10_000 });
  let last: ContextReading | null = null;
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    last = parseContextText((await chip.textContent()) ?? "");
    if (last && !last.estimate && last.used === expectUsed) return last;
    await page.waitForTimeout(120);
  }
  expect(
    false,
    `状态栏会话芯片未在 10s 内显示实测 used=${expectUsed}（当前 "${(await chip.textContent()) ?? ""}"）`,
  ).toBe(true);
  return last as ContextReading;
}

type MeterReading = {
  /** 会话内容段宽度百分比（raw_cap 刻度，= (used − F) / max）。 */
  contentPct: number;
  /** 固定开销段宽度百分比（raw_cap 刻度，= F / max）。 */
  fixedPct: number;
};

/** 读取用量条的内联刻度：`--status-context-pct` 与 `--status-context-fixed-pct`。
 *
 * 有实测用量时用量条必渲染，故此处硬断言其可见（自动等待），不返回 null——避免调用侧
 * 用 `if (meter !== null)` 把断言静默跳过。 */
async function readMeter(page: Page): Promise<MeterReading> {
  await expect(page.locator(CONTEXT_METER_SELECTOR)).toBeVisible();
  return page.evaluate((sel: string) => {
    const el = document.querySelector<HTMLElement>(sel);
    if (!el) throw new Error(`用量条未找到：${sel}`);
    const read = (name: string): number =>
      Number.parseFloat(el.style.getPropertyValue(name).trim());
    return {
      contentPct: read("--status-context-pct"),
      fixedPct: read("--status-context-fixed-pct"),
    };
  }, CONTEXT_METER_SELECTOR);
}

/** 发送一轮并等到该轮流结束（正文可见 + 状态栏回到就绪）。 */
async function runTurn(page: Page, prompt: string, answer: string) {
  await sendMessage(page, prompt);
  await expect(
    page.locator('[data-testid="chat-messages-scroller"]'),
  ).toContainText(answer, { timeout: 25_000 });
  await expect(page.locator('[data-testid="status-bar"]')).toContainText(
    "就绪",
    { timeout: 25_000 },
  );
}

test.describe("Mock：状态栏上下文用量百分比", () => {
  test("1M 窗口下显示实测 used/max 百分比，且无 ~ 基线前缀", async ({
    page,
  }) => {
    // used = 40000（含固定开销），F = used − message = 7000，max = 993856。
    const turn: TurnTokens = {
      used: 40_000,
      maxInput: 993_856,
      message: 33_000,
      answer: "收到",
    };
    await installSequencedMockSse(page, [turn], "e2e-mock-ctx-1m");
    await seedSession(page, "s_e2e_mock_ctx_1m");
    await runTurn(page, "请只回复两个字：收到", turn.answer);

    const reading = await readChip(page, turn.used);
    const meter = await readMeter(page);
    console.log(
      `[mock-statusbar-context][1M] chip="${reading.raw}" estimate=${reading.estimate} ` +
        `used=${reading.used} max=${reading.cap} pct=${reading.pct} ` +
        `meterContent=${meter.contentPct} meterFixed=${meter.fixedPct}`,
    );

    // 实测值（非新会话基线估算）：readChip 已要求 !estimate，这里再显式确认无 `~` 前缀。
    expect(reading.estimate, "实测轮不应是 ~ 基线估算").toBe(false);
    expect(
      reading.raw.startsWith("~"),
      `chip 不应带 ~ 前缀：${reading.raw}`,
    ).toBe(false);
    // 分子 = used（含固定开销），分母 = 模型原始 max_input。
    expect(reading.used).toBe(turn.used);
    expect(reading.cap).toBeGreaterThan(0);
    const expectedPct = (turn.used / turn.maxInput) * 100;
    expect(
      Math.abs(reading.pct - expectedPct),
      `pct=${reading.pct} 应≈ used/max=${expectedPct}`,
    ).toBeLessThan(PCT_TOLERANCE);
    // 用量条：固定开销段 + 内容段各自在 [0,100]；两段均以 raw_cap 为刻度，
    // 故两段之和 (used/max) 与文本百分比同归一化。readMeter 已硬断言用量条可见。
    expect(meter.contentPct).toBeGreaterThanOrEqual(0);
    expect(meter.contentPct).toBeLessThanOrEqual(100);
    expect(meter.fixedPct).toBeGreaterThanOrEqual(0);
    expect(meter.fixedPct).toBeLessThanOrEqual(100);
    expect(
      Math.abs(meter.contentPct + meter.fixedPct - reading.pct),
      `两段之和 ${meter.contentPct + meter.fixedPct} 应≈ 文本 pct=${reading.pct}`,
    ).toBeLessThan(0.5);
  });

  test("缩小窗口后同一用量占比 >30%（证明无 30% 上限）", async ({ page }) => {
    // used = 37000（含固定开销），max = 60000 → pct ≈ 61.7%。
    const turn: TurnTokens = {
      used: 37_000,
      maxInput: 60_000,
      message: 30_000,
      answer: "收到",
    };
    await installSequencedMockSse(page, [turn], "e2e-mock-ctx-small");
    await seedSession(page, "s_e2e_mock_ctx_small");
    await runTurn(page, "请只回复两个字：收到", turn.answer);

    const reading = await readChip(page, turn.used);
    console.log(
      `[mock-statusbar-context] 小窗口 chip="${reading.raw}" used=${reading.used} ` +
        `max=${reading.cap} pct=${reading.pct}`,
    );

    expect(reading.estimate).toBe(false);
    expect(reading.cap).toBe(turn.maxInput);
    // 核心断言：占比可超过 30%（若前端有 30% 上限则必失败）。
    expect(reading.pct).toBeGreaterThan(30);
  });

  test("20 轮会话：百分比恒等于 used/max，显示用量单调不减且峰值 >10%", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    // 固定开销 F = 7000 恒定，max = 200000。
    const maxInput = 200_000;
    const fixed = 7_000;
    // message 序列中途回落（模拟 L5 Skills 重选导致的 used 回落），显示值应取用量水位。
    const messages = [
      15_000, 18_000, 21_000, 24_000, 12_000, 27_000, 30_000, 33_000, 9_000,
      36_000, 39_000, 42_000, 45_000, 48_000, 51_000, 54_000, 57_000, 60_000,
      63_000, 66_000,
    ];
    const turns: TurnTokens[] = messages.map((m, i) => ({
      used: m + fixed,
      maxInput,
      message: m,
      answer: `第 ${i + 1} 轮回答。`,
    }));
    await installSequencedMockSse(page, turns, "e2e-mock-ctx-turns");
    await seedSession(page, "s_e2e_mock_ctx_turns");

    const readings: ContextReading[] = [];
    let runningMax = 0;
    for (let i = 0; i < turns.length; i++) {
      await runTurn(page, `第 ${i + 1} 轮问题`, turns[i].answer);
      runningMax = Math.max(runningMax, messages[i]);
      // 用量水位：显示 used 取历史最大值（回落轮沿用上轮显示值）。
      const r = await readChip(page, runningMax + fixed);
      readings.push(r);
      console.log(
        `[mock-statusbar-context][20turn] turn=${String(i + 1).padStart(2, "0")} ` +
          `chip="${r.raw}" used=${r.used} max=${r.cap} pct=${r.pct}`,
      );
    }

    // 每轮显示百分比都必须等于 used/max（无任何前端截断）。
    for (const r of readings) {
      expect(
        Math.abs(r.pct - (r.used / r.cap) * 100),
        `turn chip="${r.raw}" 百分比与 used/max 不一致`,
      ).toBeLessThan(PCT_TOLERANCE);
    }

    // 用量水位：显示 used 单调不减。
    for (let i = 1; i < readings.length; i++) {
      expect(
        readings[i].used,
        `turn ${i + 1} 显示 used ${readings[i].used} 低于上一轮 ${readings[i - 1].used}`,
      ).toBeGreaterThanOrEqual(readings[i - 1].used);
    }

    const first = readings[0];
    const last = readings[readings.length - 1];
    const maxPct = Math.max(...readings.map((r) => r.pct));
    console.log(
      `[mock-statusbar-context][20turn] 首轮=${first.pct}% 末轮=${last.pct}% 峰值=${maxPct}% ` +
        `(used ${first.used}→${last.used})`,
    );

    // 用量随轮次累积增长（水位保证不回落）。
    expect(last.used).toBeGreaterThan(first.used);
    // 峰值越过 10%（同口径含固定开销后占比显著）。
    expect(maxPct).toBeGreaterThan(10);
  });

  test("服务端压缩 / 裁剪后用量水位回落（context_trim.after_tokens）", async ({
    page,
  }) => {
    // 第 1 轮：used = 50000（峰值）。
    const before: TurnTokens = {
      used: 50_000,
      maxInput: 200_000,
      message: 48_000,
      answer: "收到",
    };
    // 第 2 轮中途发生压缩 / 裁剪：after_tokens = 18000，回合末快照 used = 20000。
    const after: TurnTokens = {
      used: 20_000,
      maxInput: 200_000,
      message: 18_000,
      answer: "已压缩历史",
      trimAfterTokens: 18_000,
    };
    await installSequencedMockSse(page, [before, after], "e2e-mock-ctx-trim");
    await seedSession(page, "s_e2e_mock_ctx_trim");

    await runTurn(page, "第一轮问题", before.answer);
    const peak = await readChip(page, before.used);
    expect(peak.used).toBe(before.used);

    // 第二轮中途压缩 / 裁剪：水位应回落到压缩后的 used，而不是停留在峰值（若未实现回落，
    // readChip(after.used) 会因显示仍为 50000 而超时失败）。
    await runTurn(page, "第二轮问题", after.answer);
    const trimmed = await readChip(page, after.used);
    console.log(
      `[mock-statusbar-context][trim] peak="${peak.raw}" → trimmed="${trimmed.raw}" ` +
        `(used ${peak.used}→${trimmed.used})`,
    );

    expect(trimmed.used).toBe(after.used);
    expect(trimmed.used).toBeLessThan(peak.used);
  });
});
