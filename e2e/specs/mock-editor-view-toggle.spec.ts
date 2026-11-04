/**
 * Mock 端到端测试：编辑器只替换聊天列主区（状态栈 `view` chip 切换）。
 *
 * 目的：钉住「切视图只动主区」这条分界——旧实现是把会话侧栏与整个聊天列藏在全窗口的
 * IDE 叠层后面，现在编辑器只占聊天列 body，右列 / 会话侧栏 / composer 全部常驻。
 *
 * 断言点：
 *   1. 初始：transcript 可见、编辑器 pane 不可见、composer 与侧栏在位；
 *   2. 点 `view` chip → 编辑器 pane 可见、transcript 不可见，而 composer / 侧栏仍在
 *      （这条就是本次改动与旧整屏 IDE 的分界）；
 *   3. 再点回对话：编辑器 pane 上留的探针标记还在 —— 两层是**常驻挂载只切显隐**，
 *      卸载会连带丢掉 CodeMirror 句柄、已打开标签与 transcript 的滚动锚点。
 *
 * 运行（前置：`make web-release` 后 `crabmate-web` 在 127.0.0.1:4173 托管 `frontend/dist`，
 * 且有一个 `serve` 提供 `/user-data/*`，`CRABMATE_API_BASE` 指向它）：
 *   cd e2e && npx playwright test specs/mock-editor-view-toggle.spec.ts
 */
import { expect, test, type Page } from "@playwright/test";
import { seedSession } from "../fixtures/helpers";

/** 编辑器 pane（`chat/column.rs` 的 `data-testid`）。 */
const EDITOR_PANE = '[data-testid="chat-body-editor"]';
/** transcript pane：`.chat-body` 的第一个子层，没有自己的 testid（刻意不新增）。 */
const TRANSCRIPT_PANE = ".chat-body > .chat-body-pane:first-child";
const VIEW_CHIP = '[data-testid="status-view-chip"]';

/** 探针：给节点打标记，用来证明切换视图没有把它卸载重挂。 */
const PROBE = "e2eMountProbe";

async function markEditorPane(page: Page): Promise<void> {
  await page.evaluate(
    (args: { selector: string; key: string }) => {
      const el = document.querySelector<HTMLElement>(args.selector);
      if (el) el.dataset[args.key] = "1";
    },
    { selector: EDITOR_PANE, key: PROBE },
  );
}

async function editorPaneProbe(page: Page): Promise<string | undefined> {
  return page.evaluate(
    (args: { selector: string; key: string }) =>
      document.querySelector<HTMLElement>(args.selector)?.dataset[args.key],
    { selector: EDITOR_PANE, key: PROBE },
  );
}

test.describe("编辑器视图切换", () => {
  test("view chip swaps only the chat body and keeps both layers mounted", async ({
    page,
  }) => {
    const sid = `s_e2e_editor_slot_${Date.now()}`;
    // 状态栈要可见：右列默认收起（`hidden`）时 `view` chip 不渲染。
    await seedSession(page, sid, { sidePanelView: "workspace" });

    const editorPane = page.locator(EDITOR_PANE);
    const transcript = page.locator(TRANSCRIPT_PANE);
    const composer = page.getByTestId("chat-composer-input");
    const chip = page.locator(VIEW_CHIP);

    // 1. 初态：对话可见、编辑器不可见；composer 与侧栏都在。
    await expect(transcript).toBeVisible();
    await expect(editorPane).toBeHidden();
    await expect(composer).toBeVisible();
    await expect(page.locator(".nav-rail")).toBeVisible();

    // 2. 切到编辑器：主区互换，列内的 composer 与侧栏不动。
    await expect(chip).toBeVisible();
    await expect(chip).toHaveClass(/status-chip-view/);
    await chip.click();

    await expect(editorPane).toBeVisible();
    await expect(page.getByTestId("ide-editor-body")).toBeVisible();
    await expect(transcript).toBeHidden();
    await expect(composer).toBeVisible();
    await expect(page.locator(".nav-rail")).toBeVisible();

    // 3. 切回对话：两层仍挂在同一处（探针还在），只是显隐互换。
    await markEditorPane(page);
    await chip.click();

    await expect(transcript).toBeVisible();
    await expect(editorPane).toBeHidden();
    expect(await editorPaneProbe(page)).toBe("1");
  });
});
