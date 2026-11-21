/**
 * 设置页（打开 / 关闭 / section 导航 / MCP JSON 导入与保存）回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_settings.rs`（3 个）与
 * `victauri_settings2.rs` 的 `import_mcp_json_adds_server_rows`（1 个）迁入（#152 Wave 2）。
 *
 * 注：`victauri_settings2` 另外两个用例已拆解（#548 Wave 5），都不在本文件：
 * `model_and_api_key_save` 的**页面行为**（新增预设 → 列表出现 → 服务端不含明文密钥）
 * 迁到 `mock-settings-model-registry.spec.ts`，而它的「密钥进设备钥匙串」那半由
 * `crates/crabmate-connect` 的 keyring 单测覆盖（浏览器没有钥匙串）；
 * `web_api_bearer_not_in_local_storage` 按设计**降级**不改写 —— 浏览器路径本就允许
 * 弱存储并弹显式警告，键名单一来源由 `scripts/check-storage-scope.sh` 守住。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

/** 设置页可见 = 同时带 `settings-page-visible` 类（与迁移前的判定一致）。 */
const SETTINGS_VISIBLE = '[data-testid="settings-page"].settings-page-visible';

async function openSettings(page: Page, section?: string) {
  await page.getByTestId("settings-open").click();
  await expect(page.locator(SETTINGS_VISIBLE)).toBeVisible();
  if (section) {
    await page.getByTestId(`settings-nav-${section}`).click();
  }
}

async function closeSettings(page: Page) {
  await page.getByTestId("settings-back").click();
  await expect(page.locator(SETTINGS_VISIBLE)).toHaveCount(0);
}

async function hashOf(page: Page) {
  return page.evaluate(() => window.location.hash || "");
}

test("MCP 导航写入 URL hash 并高亮导航项", async ({ page }) => {
  await seedSession(page, "s_e2e_settings_nav");
  await openSettings(page, "mcp");

  expect(await hashOf(page)).toContain("settings/mcp");
  await expect(page.getByTestId("settings-mcp-block")).toBeVisible();
  await expect(page.getByTestId("settings-nav-mcp")).toHaveClass(/active/);
});

test("设置页可以从主界面打开并关闭", async ({ page }) => {
  await seedSession(page, "s_e2e_settings_open");

  await openSettings(page);
  await expect(page.getByTestId("settings-page")).toBeVisible();

  await closeSettings(page);
});

test("设置页 section 导航切换 hash", async ({ page }) => {
  await seedSession(page, "s_e2e_settings_section");
  await openSettings(page);

  // 导航项里 `Shortcuts` 的 testid 为 None（`settings_page/layout.rs`），
  // 故只覆盖有 testid 的 section。
  for (const section of ["llm", "tools", "appearance"]) {
    await page.getByTestId(`settings-nav-${section}`).click();
    expect(
      await hashOf(page),
      `hash should contain settings/${section}`,
    ).toContain(`settings/${section}`);
  }

  await closeSettings(page);
});

test("导入 MCP JSON 后保存会 PUT mcp-servers", async ({ page }) => {
  await seedSession(page, "s_e2e_mcp_import");
  await openSettings(page, "mcp");

  const mcpConfig = {
    mcpServers: {
      "e2e-import": { command: "npx", args: ["-y", "echo", "mcp-e2e"] },
    },
  };
  await page
    .getByTestId("settings-mcp-import-json")
    .fill(JSON.stringify(mcpConfig));

  await page.getByTestId("settings-mcp-import-apply").click();
  await expect(page.getByText("已导入")).toBeVisible();

  // 保存请求：记录是否真的发了 PUT（用 204 挡掉写库）
  let saved = false;
  await page.route("**/user-data/mcp-servers", (route) => {
    if (route.request().method() !== "PUT") return route.continue();
    saved = true;
    return route.fulfill({ status: 204, body: "" });
  });

  await page.getByTestId("settings-mcp-save").click();

  // 保存成功后的反馈文案不是稳定契约（且本地化），断言「确实发出了 PUT」更贴近行为
  await expect.poll(() => saved, { timeout: 15_000 }).toBe(true);
});
