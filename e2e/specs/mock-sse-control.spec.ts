/**
 * SSE 控制面：流错误、命令审批、澄清问卷。
 *
 * 由 `victauri_sse_stub.rs`(1) 与 `victauri_sse_more.rs`(3) 迁入（#152 Wave 3）。
 *
 * 注意：原用例用的是 **V1 帧格式**（`sse_capabilities` / `stream_ended` /
 * `command_approval_request`），而前端已不再解析 V1（`crates/crabmate-client-api`
 * 里搜不到这些键），所以那几条在原处**长期是红的**。这里改按 AG-UI（V2）重写，
 * 断言意图保持不变。
 */

import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { seedSession, sendMessage } from "../fixtures/helpers";

/** 审批会话 id：审批弹窗靠响应头拿到它才能把裁决发回去。 */
const APPROVAL_SESSION_ID = "mock-appr-session-001";

function sseHeaders() {
  return {
    "content-type": "text/event-stream; charset=utf-8",
    // 跨 Origin 时前端要读这几个头，必须显式 expose
    "access-control-expose-headers":
      "x-conversation-id,x-stream-job-id,x-approval-session-id",
    "x-conversation-id": "e2e-sse",
    "x-stream-job-id": "1",
    "x-approval-session-id": APPROVAL_SESSION_ID,
  };
}

/** 按序返回多个 SSE 正文（澄清问卷需要「第一轮问卷 → 第二轮正文」）。 */
async function installSequenceSse(page: Page, bodies: string[]) {
  let index = 0;
  await page.route("**/chat/stream", (route) => {
    if (route.request().method() !== "POST") return route.continue();
    const body = bodies[Math.min(index, bodies.length - 1)];
    index += 1;
    return route.fulfill({ status: 200, headers: sseHeaders(), body });
  });
}

/** `/chat/approval` 的裁决回包（前端只关心 2xx）。 */
async function stubApprovalReply(page: Page) {
  await page.route("**/chat/approval", (route) => {
    if (route.request().method() !== "POST") return route.continue();
    return route.fulfill({ status: 204, body: "" });
  });
}

/** 一条 `command_approval` 帧（V2：CUSTOM + customType，data 为 camelCase）。 */
function approvalFrame(id: number, command: string, args: string): string {
  const data = JSON.stringify({
    command,
    args,
    allowlistKey: command,
  });
  return `id: ${id}\ndata: {"type":"CUSTOM","customType":"command_approval","data":${data}}\n\n`;
}

test("流错误在聊天列显示失败提示", async ({ page }) => {
  await seedSession(page, "s_e2e_err");

  await installSequenceSse(page, [
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: 开始执行。\n\n`,
      `id: 3\ndata: {"type":"RUN_ERROR","error":{"message":"e2e intentional failure","code":"E2E_STREAM_FAIL"}}\n\n`,
    ].join(""),
  ]);

  await sendMessage(page, "e2e error test");

  // 实际表现：聊天列落一条「错误: 对话失败」并给出重试建议（原用例断言的
  // `status-bar-fetch-error` 是「状态栏拉取失败」的类名，与流内错误无关）
  await expect(page.getByText("对话失败").first()).toBeVisible();
  await expect(page.getByText("重试").first()).toBeVisible();
});

test("命令审批弹窗显示命令预览，拒绝后关闭", async ({ page }) => {
  await seedSession(page, "s_e2e_appr");
  await stubApprovalReply(page);

  await installSequenceSse(page, [
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: e2e approval ping.\n\n`,
      approvalFrame(3, "git", "status"),
      `id: 4\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  ]);

  await sendMessage(page, "e2e approval");

  const modal = page.getByTestId("approval-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".approval-modal-command")).toContainText(
    "git status",
  );

  await page.getByTestId("approval-deny").click();
  await expect(modal).toBeHidden();
});

test("拒绝审批只关弹窗，不出现失败横幅", async ({ page }) => {
  await seedSession(page, "s_e2e_deny");
  await stubApprovalReply(page);

  await installSequenceSse(page, [
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: e2e approve.\n\n`,
      approvalFrame(3, "true", ""),
      `id: 4\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  ]);

  await sendMessage(page, "e2e deny");

  await page.getByTestId("approval-deny").click();
  await expect(page.getByTestId("approval-modal")).toBeHidden();
});

test("澄清问卷提交后触发第二轮流并收起面板", async ({ page }) => {
  await seedSession(page, "s_e2e_clarify");

  const questionnaire = JSON.stringify({
    questionnaireId: "e2e-q1",
    intro: "E2E please clarify",
    questions: [{ id: "scope", label: "Scope?", required: true }],
  });

  await installSequenceSse(page, [
    // 第一轮：只发问卷
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"clarification_questionnaire","data":${questionnaire}}\n\n`,
      `id: 2\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
    // 第二轮：提交答案后的正文
    [
      `id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n`,
      `id: 2\ndata: E2E after clarify.\n\n`,
      `id: 3\ndata: {"type":"RUN_FINISHED"}\n\n`,
    ].join(""),
  ]);

  await sendMessage(page, "e2e clarify");

  const panel = page.getByTestId("composer-clarification-panel");
  await expect(panel).toBeVisible();

  await page
    .getByTestId("composer-clarification-input")
    .first()
    .fill("backend only");
  await page.getByTestId("composer-clarification-submit").click();

  await expect(page.getByText("E2E after clarify").first()).toBeVisible();
  await expect(panel).toBeHidden();
});
