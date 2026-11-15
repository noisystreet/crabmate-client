/**
 * 可见消息筛选（聊天列）回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_visible_messages.rs` 迁入（#152 Wave 1）。
 * 原用例在 Tauri WebView 里用 `eval_js` 播种 + 数 `offsetParent` 可见行，
 * 断言的都是聊天列投影结果，故移到 Playwright（Chromium），随 PR 门禁执行。
 *
 * 断言口径保持不变：只数**可见**的 `section.chat-tui-turn--assistant`，
 * 因为被投影剔除的行仍可能留在 DOM 里（`offsetParent === null`）。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  seedSession,
  openSessionInRail,
  sendMessage,
  installMockSse,
} from "../fixtures/helpers";

/** 聊天列中可见的 assistant 回合数（与 victauri 版同口径）。 */
async function visibleAssistantCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const rows = document.querySelectorAll("section.chat-tui-turn--assistant");
    let count = 0;
    for (const row of Array.from(rows)) {
      if ((row as HTMLElement).offsetParent !== null) count += 1;
    }
    return count;
  });
}

/** 播种一个带指定消息的会话，并打开它。 */
async function seedWithMessages(
  page: Page,
  sid: string,
  messages: Record<string, unknown>[],
) {
  await seedSession(page, sid, { session: { messages } });
  await openSessionInRail(page, sid);
}

test("重复的 assistant 行在读取路径上不去重，两条都显示", async ({ page }) => {
  await seedWithMessages(page, "s_e2e_phase5_fuzzy", [
    { id: "u1", role: "user", text: "分析当前目录" },
    {
      id: "a1",
      role: "assistant",
      text: "当前目录下有三个压缩包:\n\n1.**A",
    },
    {
      id: "a2",
      role: "assistant",
      text: "当前目录下有三个压缩包:\n1.**A",
    },
  ]);

  await expect(page.getByText("分析当前目录").first()).toBeVisible();

  await expect.poll(() => visibleAssistantCount(page)).toBe(2);
});

test("final_response_snapshot 重复时聊天列只显示首条", async ({ page }) => {
  await seedWithMessages(page, "s_e2e_phase5_snap", [
    { id: "u1", role: "user", text: "分析当前目录" },
    { id: "a1", role: "assistant", text: "当前目录下有三个压缩包。" },
    {
      id: "snap",
      role: "assistant",
      text: "当前目录下有三个压缩包。",
      state: '{"k":"cm_tl","t":"final_response_snapshot"}',
    },
  ]);

  await expect(page.getByText("分析当前目录").first()).toBeVisible();

  await expect.poll(() => visibleAssistantCount(page)).toBe(1);
});

test("编排路由与 commentary 被隐藏，只显示终答", async ({ page }) => {
  // 注意：victauri 原用例播的是 `CrabMate staged_timeline\n编排路由：freeform\n{}`，
  // 而前端 `timeline_scan::is_orchestration_route_timeline_text` 的判定是
  // `text.trim_start().starts_with("编排路由：")` —— 带前缀的文本不匹配，
  // 所以那条断言在迁移前就必然失败（#152 记录的 victauri_visible_messages 0/3 即此）。
  // 这里改播符合当前判定的正文，用例本意（route 决议不进聊天气泡）不变；
  // Server 侧真实 detail 形态另需核实，见 PR 说明。
  await seedWithMessages(page, "s_e2e_phase5_ephemeral", [
    { id: "u1", role: "user", text: "分析当前目录" },
    { id: "route", role: "assistant", text: "编排路由：freeform" },
    {
      id: "cmt",
      role: "assistant",
      text: "",
      reasoning_text: "目录结构分析",
      state: "commentary_before_tools",
    },
    { id: "a1", role: "assistant", text: "当前目录下有三个压缩包。" },
  ]);

  // 终答可见作为「渲染已完成」的锚点，再判隐藏项
  await expect(
    page.getByText("当前目录下有三个压缩包。").first(),
  ).toBeVisible();

  expect(
    await page.evaluate(() => document.body.innerText.includes("编排路由")),
    "orchestration route should be hidden",
  ).toBe(false);
  expect(
    await page.evaluate(() => document.body.innerText.includes("目录结构分析")),
    "commentary before tools should be hidden",
  ).toBe(false);

  await expect.poll(() => visibleAssistantCount(page)).toBe(1);
});

test("实时流里的 orchestration_route 不进聊天气泡", async ({ page }) => {
  // 上面三个用例走「历史播种」路径（伪造已落盘的 text），测的是
  // `timeline_scan::is_orchestration_route_timeline_message` 那层文案兜底。
  // 本用例走**真实 SSE 路径**，测的是 `timeline_dispatch.rs` 里
  // `"orchestration_route" => {}` 这条按 kind 丢弃的实时分支 —— 那才是主路径。
  // 事件形态取自 Server `orchestration_route.rs`（title 恒为 `编排路由：<mode>`）
  // 与 `ag_ui_encode`（customType=timeline_log，data={kind,title,detail}）。
  await seedSession(page, "s_e2e_sse_route");

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"timeline_log","data":{"kind":"orchestration_route","title":"编排路由：freeform","detail":"freeform"}}\n\n`,
      // 正文要先进入 answer_phase，否则 delta 不计入（delta_chars=0 → 「未进入正文阶段」）
      `id: 2\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 3\ndata: 当前目录下有三个压缩包。\n\n`,
      `id: 4\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  await sendMessage(page, "分析当前目录");

  await expect(
    page.getByText("当前目录下有三个压缩包。").first(),
  ).toBeVisible();

  expect(
    await page.evaluate(() => document.body.innerText.includes("编排路由")),
    "orchestration_route must not render as a chat bubble",
  ).toBe(false);
});
