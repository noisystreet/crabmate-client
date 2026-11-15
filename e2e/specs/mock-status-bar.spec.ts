/**
 * 状态栏（footer status bar）回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_status_bar.rs` 迁入（#152 Wave 1）。
 * 原用例在 Tauri WebView 里靠 `eval_js` 注入 + `Locator::test_id` 断言，本质是
 * 页面行为，不需要壳：现由 Playwright 在 Chromium 上跑，随 PR 门禁执行。
 *
 * 状态栈位于右列；`side_panel_view: "hidden"` 时栈保留 DOM 但视觉隐藏，
 * 故凡是断言可见性 / 点击的用例都须展开右列（`sidePanelView: "workspace"`）。
 */

import { test, expect } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

test("页面加载后 footer 状态栏可见", async ({ page }) => {
  await seedSession(page, "s_e2e_status", { sidePanelView: "workspace" });

  await expect(page.getByTestId("status-bar")).toBeVisible();
});

test("点击 agent role 触发器弹出向上菜单", async ({ page }) => {
  await seedSession(page, "s_e2e_role", { sidePanelView: "workspace" });

  // 先确认状态栏在位，否则下面的点击失败会指向错误的原因
  await expect(page.getByTestId("status-bar")).toBeVisible();

  await page.getByTestId("status-agent-role-trigger").click();

  await expect(page.locator(".status-agent-role-menu")).toBeVisible();
});
