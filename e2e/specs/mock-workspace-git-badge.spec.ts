/**
 * Mock 端到端测试：顶栏工作区标题旁的 git 徽标（#324）。
 *
 * 共享纯逻辑（`crates/crabmate-client-api/src/workspace_git.rs`）自己有单测，本文件只守
 * **它接进 DOM 的那一层**：`GET /workspace/git` 的 body 决定徽标写什么，拿不到响应则整块不画。
 * 补这条的理由很具体：这一格此前只有 Rust 单测，DOM 层零护栏（`data-testid` 早就加了却没人用），
 * 而「徽标在真页面里到底出不出现」正是单测证明不了的部分。
 *
 * 六个用例覆盖契约里**互不相同**的几个态：
 *   1. `ok` + 有计数 → `main ●3`，脏着色 + 说明性 `aria-label` + 分类计数明细；
 *   2. `ok` + 干净 → 只有 `main`，不带脏标记（「无未提交改动」与「字段没给」必须能分开）；
 *   3. `detached` → `HEAD@1a2b3c4`（不是空分支名，也不是那句「非 git 仓库」）；
 *   4. `not_a_repo` → 「非 git 仓库」（三态各有各的话，不能退化成空白）；
 *   5. 404（老 serve 没有这条路由）→ 整块不渲染：宁可不显示，也不给一个看起来干净的分支名；
 *   6. 点一下 → 请求带 `refresh=true`（手动刷新手势；服务端据此跳过缓存读）。
 *
 * 运行方式（与其它 mock spec 同前置：`make web-release` 后 `crabmate-web` 托管 `frontend/dist`，
 * `CRABMATE_API_BASE` 指向纯 API serve）：
 *   cd e2e && npx playwright test specs/mock-workspace-git-badge.spec.ts
 */

import { test, expect, type Page, type Route } from "@playwright/test";

import { seedSession } from "../fixtures/helpers";

/** 与会话无关：徽标只看工作区，但 `seedSession` 要一个活动会话 id 才好落桶。 */
const SID = "sess-git-badge";

/** `paths::workspace_git` 的形态：`/workspace/git`，手动刷新时追加 `refresh=true`。 */
const GIT_ROUTE = "**/workspace/git*";

/** 契约 `WorkspaceGitCounts` 的 serde 形状（分类计数，`total` 是记录行总数）。 */
type Counts = {
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
  total: number;
};

/** 分类计数快捷构造：`total` 取各项之和（rename 两行只算一行的极端情形这里不需要）。 */
function counts(
  staged: number,
  unstaged: number,
  untracked: number,
  conflicted = 0,
): Counts {
  return {
    staged,
    unstaged,
    untracked,
    conflicted,
    total: staged + unstaged + untracked + conflicted,
  };
}

/** `WorkspaceGitHead::Branch`（`#[serde(tag = "kind")]`）。 */
function branchHead(name: string) {
  return { kind: "branch", name, oid: "1a2b3c4d5e6f" };
}

/** `WorkspaceGitHead::Detached`：没有 `name` 字段，用「字段缺失」区分于空分支名。 */
function detachedHead() {
  return { kind: "detached", oid: "1a2b3c4d5e6f" };
}

/** 一条 `state: ok` 的响应体。 */
function okBody(head: object, body_counts: Counts | null, dirty: boolean) {
  return {
    state: "ok",
    head,
    counts: body_counts ?? undefined,
    dirty,
    computed_at_unix: 1_700_000_000,
    source: "probe",
  };
}

/**
 * 顶替 `GET /workspace/git`，并记录每次请求的查询串。
 *
 * 记录查询串是为了断言「点击 = 强制刷新」——`refresh=true` 是本仓与服务端约定的唯一差别，
 * 而它不会体现在任何可见文本里。
 */
async function stubGit(
  page: Page,
  respond: () => { status?: number; body?: unknown },
): Promise<string[]> {
  const queries: string[] = [];
  await page.route(GIT_ROUTE, async (route: Route) => {
    queries.push(new URL(route.request().url()).search);
    const reply = respond();
    if (reply.status && reply.status !== 200) {
      await route.fulfill({
        status: reply.status,
        contentType: "application/json",
        body: "{}",
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(reply.body),
    });
  });
  return queries;
}

/** 等首帧 + 工作区树加载完（否则徽标那个 `Effect` 还会因为「路径为空」而早退）。 */
async function openShell(page: Page): Promise<void> {
  // `side_panel_view: workspace`：工作区面板要挂上，树加载完才会把路径写进 draft。
  await seedSession(page, SID, { sidePanelView: "workspace" });
  await expect(page.getByTestId("workspace-root-title")).toBeVisible();
}

test.describe("顶栏 git 徽标", () => {
  test("分支 + 脏计数：main ●3，脏着色与分类明细都在", async ({ page }) => {
    await stubGit(page, () => ({
      body: okBody(branchHead("main"), counts(1, 1, 1), true),
    }));
    await openShell(page);

    const badge = page.getByTestId("workspace-git-badge");
    await expect(badge).toHaveText("main ●3");
    // 脏标记不靠颜色单独表达：字符 `●3` 本身就是信息，class 只负责着色。
    await expect(badge).toHaveClass(/shell-topbar-git-badge-dirty/);
    await expect(badge).toHaveAttribute(
      "aria-label",
      "当前分支与未提交改动（点击刷新）",
    );
    await expect(badge).toHaveAttribute(
      "title",
      "当前工作区根下未提交改动：已暂存 1 · 未暂存 1 · 未跟踪 1 · 冲突 0",
    );
  });

  test("干净工作区：只有分支名，没有脏标记", async ({ page }) => {
    await stubGit(page, () => ({
      body: okBody(branchHead("main"), counts(0, 0, 0), false),
    }));
    await openShell(page);

    const badge = page.getByTestId("workspace-git-badge");
    await expect(badge).toHaveText("main");
    await expect(badge).not.toHaveClass(/shell-topbar-git-badge-dirty/);
    // 措辞是「当前工作区根下」而不是「仓库干净」：仓库根可能在工作区根之上。
    await expect(badge).toHaveAttribute("title", "当前工作区根下无未提交改动");
  });

  test("detached HEAD：HEAD@短 oid，而不是空分支名", async ({ page }) => {
    await stubGit(page, () => ({
      body: okBody(detachedHead(), counts(0, 0, 0), false),
    }));
    await openShell(page);

    await expect(page.getByTestId("workspace-git-badge")).toHaveText(
      "HEAD@1a2b3c4",
    );
  });

  test("非 git 工作区：明确文案，不是空白", async ({ page }) => {
    await stubGit(page, () => ({
      body: {
        state: "not_a_repo",
        computed_at_unix: 1_700_000_000,
        source: "probe",
      },
    }));
    await openShell(page);

    await expect(page.getByTestId("workspace-git-badge")).toHaveText(
      "非 git 仓库",
    );
  });

  test("老 serve 没有这条路由（404）：整块不渲染", async ({ page }) => {
    const queries = await stubGit(page, () => ({ status: 404 }));
    await openShell(page);

    // 先确认请求**确实发过**：否则「没有徽标」可能只是还没加载，断言等于没测。
    await expect.poll(() => queries.length).toBeGreaterThan(0);
    await expect(page.getByTestId("workspace-git-badge")).toHaveCount(0);
  });

  test("点击徽标 = 强制刷新（请求带 refresh=true）", async ({ page }) => {
    const queries = await stubGit(page, () => ({
      body: okBody(branchHead("main"), counts(2, 0, 0), true),
    }));
    await openShell(page);

    const badge = page.getByTestId("workspace-git-badge");
    await expect(badge).toHaveText("main ●2");
    // 首次拉取不带 flag：那是常规 TTL 读，不该每次都打穿服务端缓存。
    expect(queries.every((q) => !q.includes("refresh=true"))).toBe(true);

    await badge.click();
    await expect
      .poll(() => queries.some((q) => q.includes("refresh=true")))
      .toBe(true);
  });
});
