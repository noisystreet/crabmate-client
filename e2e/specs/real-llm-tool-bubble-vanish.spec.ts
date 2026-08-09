/**
 * 真实 LLM：按 `chat_export_20260730_225530.md` 同类任务复现
 * 「有工具调用时助手气泡先出现再消失」。
 *
 * 前置：
 *   1. `crabmate serve` 在 127.0.0.1:8080
 *   2. `API_KEY` / TOML，或本机钥匙串/E2E 注入已有 `client_llm` 密钥
 *   3. 启用 Web Bearer 时设 `CM_WEB_API_BEARER_TOKEN`
 *
 * 运行：
 *   cd e2e && no_proxy=127.0.0.1,localhost,api.deepseek.com \
 *     npx playwright test specs/real-llm-tool-bubble-vanish.spec.ts
 */
import { expect, test, type Page } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import {
  ensureRealLlmModelCredential,
  resolveOptionalApiKeyFromEnvOrToml,
  sendMessage,
  setupRealLLMSessionPreferringKeyring,
  waitForHealth,
} from "../fixtures/helpers";

const API_KEY = resolveOptionalApiKeyFromEnvOrToml();
const PROMPT = "分析一下当前目录下的源码";

type VanishGap = {
  reason:
    | "assistant_body_sections_zero"
    | "seen_text_missing"
    | "seen_text_flicker_reappear";
  needle: string;
  tMs: number;
  bodyAssistantCount: number;
};

type MonitorResult = {
  gaps: VanishGap[];
  samples: number;
  firstBodyAtMs: number | null;
  maxBodyCount: number;
  toolCardSeen: boolean;
  /** 首次归零瞬间 / 归零后恢复瞬间的 transcript 结构，用于定位旁白落到哪一行。 */
  zeroSnapshot: string[] | null;
  recoverSnapshot: string[] | null;
};

function seedMiniCppWorkspace(): string {
  const projectRoot = path.resolve(process.cwd(), "..");
  const wsDir = path.join(projectRoot, `.e2e_tmp_vanish_${Date.now()}`);
  fs.mkdirSync(wsDir, { recursive: true });
  const demo = path.join(wsDir, "cpp-demo");
  fs.mkdirSync(demo, { recursive: true });
  fs.writeFileSync(
    path.join(demo, "main.cpp"),
    `#include <iostream>
#include <vector>
#include <numeric>
int main() {
  std::vector<int> fib{0, 1};
  for (int i = 2; i < 15; ++i) fib.push_back(fib[i - 1] + fib[i - 2]);
  for (int v : fib) std::cout << v << " ";
  std::cout << "\\nsum=" << std::accumulate(fib.begin(), fib.end(), 0) << "\\n";
  return 0;
}
`,
  );
  fs.writeFileSync(
    path.join(demo, "CMakeLists.txt"),
    `cmake_minimum_required(VERSION 3.10)
project(CppDemo VERSION 1.0.0 LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
add_executable(demo main.cpp)
`,
  );
  fs.writeFileSync(
    path.join(wsDir, "package.json"),
    `{ "dependencies": { "@playwright/test": "^1.62.0" } }\n`,
  );
  return wsDir;
}

async function bindWorkspaceAndSession(page: Page, wsDir: string, sid: string) {
  await page.evaluate((dir: string) => {
    return fetch("/workspace", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: dir }),
    });
  }, wsDir);
  await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });
  await page.evaluate((s: string) => {
    const body = JSON.stringify({
      sessions: [
        {
          id: s,
          title: "e2e-real-vanish",
          draft: "",
          messages: [],
          updated_at: Date.now(),
          pinned: false,
          starred: false,
        },
      ],
      active_session_id: s,
    });
    return fetch("/user-data/workspaces/current/sessions", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body,
    });
  }, sid);
  await page.reload({ waitUntil: "networkidle", timeout: 20_000 });
  await page.waitForSelector('[data-testid="chat-composer-input"]', {
    timeout: 15_000,
  });
}

async function waitForReadyWhileApproving(page: Page, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs;
  const statusBar = page.locator('[data-testid="status-bar"]');
  const approvalModal = page.locator('[data-testid="approval-modal"]');
  while (Date.now() < deadline) {
    if (await approvalModal.isVisible().catch(() => false)) {
      const allowAlways = page.locator('[data-testid="approval-allow-always"]');
      if (await allowAlways.isVisible().catch(() => false)) {
        await allowAlways.click();
      } else {
        await page.locator('[data-testid="approval-allow-once"]').click();
      }
      await expect(approvalModal).not.toBeVisible({ timeout: 10_000 });
      continue;
    }
    if ((await statusBar.textContent())?.includes("就绪")) return;
    await page.waitForTimeout(250);
  }
  throw new Error(`流在 ${timeoutMs}ms 内未进入就绪状态`);
}

test.describe("真实 LLM：工具回合气泡闪没", () => {
  test("源码分析多工具流：助手气泡不得出现后消失", async ({ page }) => {
    const baseUrl = `http://127.0.0.1:${process.env.CRABMATE_PORT || "8080"}`;
    await waitForHealth(baseUrl);

    const sid = `s_e2e_real_vanish_${Date.now()}`;
    await setupRealLLMSessionPreferringKeyring(page, sid, API_KEY);

    if (!(await ensureRealLlmModelCredential(page, API_KEY))) {
      test.skip(
        true,
        "未设置 API_KEY 且无 client_llm 密钥（钥匙串/E2E），跳过真实 LLM 用例",
      );
      return;
    }

    const wsDir = seedMiniCppWorkspace();
    try {
      await bindWorkspaceAndSession(page, wsDir, sid);
      await expect(page.getByTestId("chat-tui-stream-view")).toBeVisible();

      await page.evaluate((sampleMs) => {
        const state = globalThis as typeof globalThis & {
          __cmRealVanishStop?: () => MonitorResult;
        };
        const startedAt = performance.now();
        const gaps: VanishGap[] = [];
        let samples = 0;
        let firstBodyAtMs: number | null = null;
        let maxBodyCount = 0;
        let toolCardSeen = false;
        /** 首次正文助手的稳定前缀（定长）；之后只要求 transcript 仍含该子串。 */
        let frozenPrefix: string | null = null;
        let prefixGonePendingFlicker = false;
        let lastZeroGapAt = -1_000;
        let lastMissingGapAt = -1_000;
        let zeroSnapshot: string[] | null = null;
        let recoverSnapshot: string[] | null = null;
        let sawZero = false;

        const nowMs = () => Math.round(performance.now() - startedAt);

        const snapshotSections = (tMs: number): string[] => {
          const root = document.querySelector(
            '[data-testid="chat-tui-transcript"]',
          );
          const rows = [
            ...(root?.querySelectorAll<HTMLElement>("section") ?? []),
          ];
          return [
            `t=${tMs} sections=${rows.length}`,
            ...rows.map((el) => {
              const text = (el.innerText ?? "").replace(/\s+/g, " ").trim();
              return `${el.className} | len=${text.length} | ${text.slice(0, 60)}`;
            }),
          ];
        };

        const sample = () => {
          samples += 1;
          const tMs = nowMs();
          if (
            document.querySelector('[data-testid="chat-tui-tool-process"]') ||
            document.querySelector("section.chat-tui-turn--tool")
          ) {
            toolCardSeen = true;
          }
          const assistants = [
            ...document.querySelectorAll<HTMLElement>(
              "section.chat-tui-turn--assistant",
            ),
          ];
          const bodyAssistants = assistants.filter((el) => {
            const text = (el.innerText ?? "").replace(/\s+/g, " ").trim();
            return Boolean(text);
          });
          maxBodyCount = Math.max(maxBodyCount, bodyAssistants.length);
          const transcriptText = (
            document.querySelector('[data-testid="chat-tui-transcript"]')
              ?.textContent ?? ""
          ).replace(/\s+/g, " ");

          if (bodyAssistants.length > 0 && frozenPrefix === null) {
            const first = (bodyAssistants[0].innerText ?? "")
              .replace(/\s+/g, " ")
              .trim();
            if (first.length >= 12) {
              frozenPrefix = first.slice(0, 18);
              firstBodyAtMs = tMs;
            }
          }

          if (firstBodyAtMs === null) return;

          if (bodyAssistants.length === 0) {
            if (zeroSnapshot === null) {
              zeroSnapshot = snapshotSections(tMs);
            }
            sawZero = true;
            // 节流：同一闪空窗口只记一次
            if (tMs - lastZeroGapAt > 40) {
              gaps.push({
                reason: "assistant_body_sections_zero",
                needle: frozenPrefix ?? "*",
                tMs,
                bodyAssistantCount: 0,
              });
              lastZeroGapAt = tMs;
            }
          } else if (sawZero && recoverSnapshot === null) {
            recoverSnapshot = snapshotSections(tMs);
          }

          if (frozenPrefix) {
            const visible = transcriptText.includes(frozenPrefix);
            if (visible) {
              if (prefixGonePendingFlicker) {
                gaps.push({
                  reason: "seen_text_flicker_reappear",
                  needle: frozenPrefix,
                  tMs,
                  bodyAssistantCount: bodyAssistants.length,
                });
                prefixGonePendingFlicker = false;
              }
            } else if (tMs - lastMissingGapAt > 40) {
              prefixGonePendingFlicker = true;
              gaps.push({
                reason: "seen_text_missing",
                needle: frozenPrefix,
                tMs,
                bodyAssistantCount: bodyAssistants.length,
              });
              lastMissingGapAt = tMs;
            }
          }
        };

        const root = document.querySelector(
          '[data-testid="chat-tui-transcript"]',
        );
        if (!root) throw new Error("transcript missing");
        const mo = new MutationObserver(sample);
        mo.observe(root, {
          childList: true,
          characterData: true,
          subtree: true,
        });
        const timer = window.setInterval(sample, sampleMs);
        state.__cmRealVanishStop = () => {
          mo.disconnect();
          window.clearInterval(timer);
          return {
            gaps,
            samples,
            firstBodyAtMs,
            maxBodyCount,
            toolCardSeen,
            zeroSnapshot,
            recoverSnapshot,
          };
        };
      }, 8);

      await sendMessage(page, PROMPT);
      await waitForReadyWhileApproving(page, 240_000);

      await expect(page.locator('[data-testid="status-bar"]')).toContainText(
        "就绪",
        { timeout: 5_000 },
      );
      await page.waitForTimeout(800);

      const result = await page.evaluate(() => {
        const state = globalThis as typeof globalThis & {
          __cmRealVanishStop?: () => MonitorResult;
        };
        return state.__cmRealVanishStop?.() ?? null;
      });

      expect(result, "vanish monitor must be installed").not.toBeNull();
      expect(
        result!.firstBodyAtMs,
        "at least one assistant body bubble must appear",
      ).not.toBeNull();
      expect(
        result!.toolCardSeen || result!.maxBodyCount >= 1,
        "expected tool cards or multi-step assistant bodies (tool-call path)",
      ).toBeTruthy();

      // 压缩同 reason 的连续采样（节流后仍可能有多段闪空窗口）
      const compactGaps = result!.gaps.filter((gap, index, all) => {
        if (index === 0) return true;
        const prev = all[index - 1];
        return !(
          prev.reason === gap.reason &&
          prev.needle === gap.needle &&
          gap.tMs - prev.tMs < 120
        );
      });

      const zeroGaps = compactGaps.filter(
        (g) => g.reason === "assistant_body_sections_zero",
      );
      expect(
        zeroGaps,
        `real-LLM assistant body bubble count hit zero after first paint: ${JSON.stringify(
          {
            zeroGaps,
            allGaps: compactGaps.slice(0, 20),
            samples: result!.samples,
            firstBodyAtMs: result!.firstBodyAtMs,
            maxBodyCount: result!.maxBodyCount,
            toolCardSeen: result!.toolCardSeen,
            zeroSnapshot: result!.zeroSnapshot,
            recoverSnapshot: result!.recoverSnapshot,
          },
          null,
          2,
        )}`,
      ).toEqual([]);

      expect(
        compactGaps,
        `real-LLM bubble vanished during tool stream: ${JSON.stringify({
          gaps: compactGaps.slice(0, 20),
          gapCount: compactGaps.length,
          samples: result!.samples,
          firstBodyAtMs: result!.firstBodyAtMs,
          maxBodyCount: result!.maxBodyCount,
          toolCardSeen: result!.toolCardSeen,
        })}`,
      ).toEqual([]);
      expect(result!.samples).toBeGreaterThan(50);
    } finally {
      fs.rmSync(wsDir, { recursive: true, force: true });
    }
  });
});
