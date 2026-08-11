/**
 * Android WebView 回归：工具结果后响应体提前 EOF、尚未收到 RUN_FINISHED 时，
 * 队列流必须按 job id + Last-Event-ID 续接，不能合成 completed。
 */
import { expect, test } from "@playwright/test";
import { seedSession, sendMessage } from "../fixtures/helpers";

type ResumeProbe = {
  callCount: number;
  resumeBody: unknown;
  lastEventId: string | null;
};

const FINAL_ANSWER = "已读取文件，main.rs 包含程序入口和 Hello 输出。";

test("tool stream EOF resumes and receives final answer", async ({ page }) => {
  await seedSession(page, `s_e2e_stream_resume_${Date.now()}`);

  await page.evaluate(
    ({ finalAnswer }) => {
      const originalFetch = window.fetch.bind(window);
      const probe: ResumeProbe = {
        callCount: 0,
        resumeBody: null,
        lastEventId: null,
      };
      (
        window as Window & { __crabmateStreamResumeProbe?: ResumeProbe }
      ).__crabmateStreamResumeProbe = probe;

      const firstBody = [
        'id: 1\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n',
        "id: 2\ndata: 我先读取文件。\n\n",
        'id: 3\ndata: {"type":"CUSTOM","customType":"turn_segment_start","data":{"segmentId":"seg-before-read","kind":"commentary","beforeToolCallId":"tc-read"}}\n\n',
        'id: 4\ndata: {"type":"CUSTOM","customType":"turn_segment_end","data":{"segmentId":"seg-before-read"}}\n\n',
        'id: 5\ndata: {"type":"TOOL_CALL_START","toolCallId":"tc-read","name":"read_file"}\ndata: {"type":"TOOL_CALL_ARGS","toolCallId":"tc-read","args":"{\\"path\\":\\"src/main.rs\\"}"}\ndata: {"type":"TOOL_CALL_END","toolCallId":"tc-read"}\n\n',
        'id: 6\ndata: {"type":"CUSTOM","customType":"tool_running","data":{"running":true}}\n\n',
        'id: 7\ndata: {"type":"TOOL_CALL_RESULT","toolCallId":"tc-read","content":"fn main() {}","metadata":{"name":"read_file","ok":true,"summary":"读取成功"}}\n\n',
        'id: 8\ndata: {"type":"CUSTOM","customType":"turn_tool_phase_end","data":{"phase":"tool_end"}}\n\n',
        'id: 9\ndata: {"type":"CUSTOM","customType":"tool_running","data":{"running":false}}\n\n',
      ].join("");
      const resumedBody = [
        'id: 10\ndata: {"type":"CUSTOM","customType":"assistant_answer_phase"}\n\n',
        `id: 11\ndata: ${finalAnswer}\n\n`,
        'id: 12\ndata: {"type":"RUN_FINISHED","threadId":"","runId":"resume"}\n\n',
      ].join("");

      window.fetch = async (input, init) => {
        const request =
          input instanceof Request ? input : new Request(input, init);
        if (
          !request.url.includes("/chat/stream") ||
          request.method.toUpperCase() !== "POST"
        ) {
          return originalFetch(input, init);
        }

        probe.callCount += 1;
        if (probe.callCount === 2) {
          probe.resumeBody = JSON.parse(await request.clone().text());
          probe.lastEventId = request.headers.get("Last-Event-ID");
        }
        return new Response(probe.callCount === 1 ? firstBody : resumedBody, {
          status: 200,
          headers: {
            "content-type": "text/event-stream; charset=utf-8",
            "x-conversation-id": "e2e-stream-resume",
            "x-stream-job-id": "77",
          },
        });
      };
    },
    { finalAnswer: FINAL_ANSWER },
  );

  await sendMessage(page, "读取 src/main.rs 并总结");

  const messages = page.locator('[data-testid="chat-messages-scroller"]');
  await expect(messages).toContainText(FINAL_ANSWER, { timeout: 20_000 });
  await expect(messages).not.toContainText("未收到正文片段");

  const probe = await page.evaluate(
    () =>
      (window as Window & { __crabmateStreamResumeProbe?: ResumeProbe })
        .__crabmateStreamResumeProbe,
  );
  expect(probe?.callCount).toBe(2);
  expect(probe?.lastEventId).toBe("9");
  expect(probe?.resumeBody).toMatchObject({
    stream_resume: { job_id: 77, after_seq: 9 },
  });
});
