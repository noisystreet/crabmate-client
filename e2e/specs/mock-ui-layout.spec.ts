/**
 * 核心布局区域回归。
 *
 * 由 `victauri_ui_layout.rs`(4) 迁入（#152 Wave 3）。原用例走 `eval_js` 数 DOM，
 * 全是页面结构断言。
 *
 * 原第 3 例等的是 SSE 正文里出现的 `sse_capabilities` 字样（V1 帧被当正文渲染的
 * 副产物）；这里用 V2 正文，断言真正的回复文本。
 */

import { test, expect } from "@playwright/test";
import { seedSession, sendMessage, installMockSse } from "../fixtures/helpers";

test("核心布局区域存在且侧栏默认收起", async ({ page }) => {
  await seedSession(page, "s_layout");

  await expect(page.getByTestId("chat-column")).toBeVisible();
  // 状态栈常驻 DOM（右列收起时视觉隐藏）
  await expect(page.getByTestId("status-bar")).toHaveCount(1);
  // 注：`.side-body`（testid=side-panel）收起时仍留在 DOM 且保有尺寸，
  // `toBeHidden` 不成立，故这里不按它断言「侧栏收起」。
  await expect(page.getByTestId("chat-messages-scroller")).toBeVisible();
});

test("输入栏结构", async ({ page }) => {
  await seedSession(page, "s_layout_composer");

  await expect(page.getByTestId("chat-composer-input")).toBeVisible();
  await expect(page.getByTestId("chat-send-button")).toBeVisible();
});

test("发送消息后渲染聊天回合", async ({ page }) => {
  await seedSession(page, "s_layout_bubble");

  await installMockSse(
    page,
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: 布局测试回复\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  );

  await sendMessage(page, "你好");

  await expect(
    page.locator("section.chat-tui-turn[data-tui-msg-id]").first(),
  ).toBeVisible();
  await expect(page.getByText("布局测试回复").first()).toBeVisible();
});

test("对话模式下 IDE 布局根隐藏", async ({ page }) => {
  await seedSession(page, "s_layout_ide");

  await expect(page.getByTestId("ide-layout-root")).toBeHidden();
});
