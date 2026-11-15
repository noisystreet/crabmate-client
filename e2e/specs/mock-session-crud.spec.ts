/**
 * 会话 CRUD（rail + 会话管理模态框）回归。
 *
 * 由 `desktop-tauri/src-tauri/tests/victauri_session_crud.rs` 迁入（#152 Wave 2）。
 * 原用例靠 `eval_js` 播种 + `Locator::test_id` / `press_key` 断言，全是页面行为。
 *
 * 播种两个会话（`s_e2e_keep` 为活动会话）；`seedSession` 的 `sessions` 选项用来
 * 一次写入多条，`active_session_id` 取自传入的 `sid`。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { seedSession, openSessionInRail } from "../fixtures/helpers";

const KEEP = "s_e2e_keep";
const DROP = "s_e2e_drop";

/** rail 里的会话条目（`nav-session-<sid>`）。 */
function railRows(page: Page) {
  return page.locator('[data-testid^="nav-session-"]');
}

async function seedTwoSessions(page: Page) {
  await seedSession(page, KEEP, {
    sessions: [
      {
        id: KEEP,
        title: "E2E keep",
        messages: [{ id: "m1", role: "user", text: "keep me" }],
        updated_at: 2,
      },
      {
        id: DROP,
        title: "E2E drop",
        messages: [{ id: "m2", role: "user", text: "drop me" }],
        updated_at: 1,
      },
    ],
  });
}

/** 右键 nav-rail 打开上下文菜单，点「管理会话」进模态框。 */
async function openSessionListModal(page: Page) {
  await page.locator(".nav-rail-scroll").click({ button: "right" });
  await page.getByText("管理会话").click();
  await expect(page.getByTestId("session-list-modal")).toBeVisible();
}

test("新建对话复用已有空会话，不堆积 rail 条目", async ({ page }) => {
  await seedTwoSessions(page);

  // 两个播种会话 + 冷启动空会话
  await expect(railRows(page)).toHaveCount(3);

  // 当前活动会话就是那个空会话，所以「新对话」换掉它的 id 而不新增条目
  await page.getByTestId("nav-new-chat").click();

  await expect(railRows(page)).toHaveCount(3);
});

test("在管理模态框里固定会话后 rail 条目带上 is-pinned", async ({ page }) => {
  await seedTwoSessions(page);
  await openSessionListModal(page);

  await page.getByTestId(`session-modal-pin-${DROP}`).click();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("session-list-modal")).toBeHidden();

  await expect(page.getByTestId(`nav-session-${DROP}`)).toHaveClass(
    /is-pinned/,
  );
});

test("确认删除后会话从模态框与 rail 同时移除", async ({ page }) => {
  await seedTwoSessions(page);
  await openSessionListModal(page);

  await page.getByTestId(`session-modal-delete-${DROP}`).click();

  // 删除走项目内确认框（`shell-confirm-*`），不是 window.confirm
  await expect(page.getByTestId("shell-confirm-dialog")).toBeVisible();
  await page.getByTestId("shell-confirm-ok").click();

  await expect(page.getByTestId(`session-modal-open-${DROP}`)).toHaveCount(0);
  await expect(page.getByTestId(`nav-session-${DROP}`)).toHaveCount(0);
});

test("Escape 关闭模态框不改变 rail", async ({ page }) => {
  await seedTwoSessions(page);
  await openSessionListModal(page);

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("session-list-modal")).toBeHidden();

  await expect(railRows(page)).toHaveCount(3);
});

test("管理模态框可以切换活动会话", async ({ page }) => {
  await seedTwoSessions(page);
  await openSessionInRail(page, KEEP);

  await expect(page.getByText("keep me").first()).toBeVisible();

  await openSessionListModal(page);
  await page.getByTestId(`session-modal-open-${DROP}`).click();

  await expect(page.getByTestId("session-list-modal")).toBeHidden();
  await expect(page.getByText("drop me").first()).toBeVisible();
  await expect(page.getByText("keep me")).toHaveCount(0);
});
