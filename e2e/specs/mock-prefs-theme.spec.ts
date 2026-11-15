/**
 * 偏好：主题与右列（side panel）回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_prefs_theme.rs` 迁入（#152 Wave 2）。
 * 原用例 `eval_js` 里 PUT `/user-data/prefs` 再 reload，断言 `data-theme` 与
 * `.side-column` 宽度 —— 纯页面行为，不需要壳。
 */

import { test, expect } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

test("prefs 里的 dark 主题在加载后落到 data-theme", async ({ page }) => {
  await seedSession(page, "s_e2e_theme", { theme: "crabmate-dark" });

  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    "crabmate-dark",
  );
});

test("prefs 说 workspace 时右列展开并按存储宽度恢复", async ({ page }) => {
  await seedSession(page, "s_e2e_side", {
    sidePanelView: "workspace",
    sideWidth: 320,
  });

  await expect(page.getByTestId("workspace-panel")).toBeVisible();

  // 宽度来自布局计算，可能晚一帧到位
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          document.querySelector(".side-column")?.getBoundingClientRect()
            .width ?? 0,
      ),
    )
    .toBeGreaterThan(319);

  const sideWidth = await page.evaluate(
    () =>
      document.querySelector(".side-column")?.getBoundingClientRect().width ??
      0,
  );
  expect(Math.abs(sideWidth - 320)).toBeLessThan(1);
});
