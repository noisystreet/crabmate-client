/**
 * 侧栏浮动工具栏：DOM 位置、设置/视图按钮、GitHub 仓库按钮。
 *
 * 由 `victauri_side_toolbar.rs`(4) 迁入（#152 Wave 3）。
 * 原用例覆盖 `window.fetch` 造 `/github/repo-context`，这里用 `page.route`。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

const GITHUB_URL = "https://github.com/octocat/Hello-World";

/** GitHub 上下文桩：让仓库按钮拿到 URL 从而可点。 */
async function stubGithubRoutes(page: Page) {
  await page.route("**/github/repo-context**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        connected: false,
        url: GITHUB_URL,
        repo: "octocat/Hello-World",
      }),
    }),
  );
  await page.route("**/github/pr/current/checks**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
    }),
  );
}

test("侧栏收起时浮动工具栏可见且不被 rail 列挡住", async ({ page }) => {
  await stubGithubRoutes(page);
  await seedSession(page, "s_e2e_side_tb");

  // 收起态下工具栏以浮动条渲染，仍在页面上
  await expect(page.getByTestId("side-shell-toolbar")).toBeVisible();

  // rail-only 列若存在，不得用 pointer-events:none（会挡住工具栏点击）
  const pointerEvents = await page.evaluate(() => {
    const col = document.querySelector(".side-column.side-column-rail-only");
    return col ? getComputedStyle(col).pointerEvents : "absent";
  });
  expect(pointerEvents).not.toBe("none");
});

test("设置按钮打开设置页", async ({ page }) => {
  await stubGithubRoutes(page);
  await seedSession(page, "s_e2e_side_tb_settings");

  await page.getByTestId("settings-open").click();

  await expect(
    page.locator('[data-testid="settings-page"].settings-page-visible'),
  ).toBeVisible();
});

test("视图按钮展开视图菜单", async ({ page }) => {
  await stubGithubRoutes(page);
  await seedSession(page, "s_e2e_side_tb_view");

  await page.getByTestId("side-view-trigger").click();

  await expect(page.locator(".toolbar-view-menu")).toBeVisible();
});

test("GitHub 仓库按钮走系统浏览器路径，不打开嵌入页", async ({ page }) => {
  await stubGithubRoutes(page);
  await seedSession(page, "s_e2e_side_tb_github");

  const repoBtn = page.getByTestId("side-toolbar-github-repo");
  // 等 URL 到位后按钮才可点
  await expect(repoBtn).toBeEnabled({ timeout: 10_000 });

  // 点击会尝试打开新窗口：接住并关掉，避免影响后续断言
  page.on("popup", (popup) => void popup.close().catch(() => {}));
  await repoBtn.click();

  // 已废弃的嵌入页不得出现
  await expect(page.getByTestId("github-embed-page")).toHaveCount(0);
});
