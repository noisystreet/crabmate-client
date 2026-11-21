/**
 * 设置页「模型列表」：新增预设 → 列表出现 → 服务端持久化不含明文密钥（#548 Wave 5）。
 *
 * 由 `victauri_settings2.rs` 的 `model_and_api_key_save` 迁入**页面行为**那一半。
 * 原用例另外两条断言在浏览器里没有等价物，故意不迁，并各有替代：
 *
 *   - 「密钥写进设备钥匙串」（`invoke('get_llm_secret')` 读回）—— 浏览器没有钥匙串。
 *     该路径现由 `crates/crabmate-connect` 的 keyring 单测覆盖
 *     （`secret_backend::testing` 装内存后端，`cargo test` 里真做写入 → 读回）；
 *   - 「密钥不进 localStorage」—— 浏览器路径**本来就**允许弱存储并弹显式警告（产品设计），
 *     所以浏览器里反向断言这件事是错的。键名的单一来源由
 *     `scripts/check-storage-scope.sh` + `specs/mock-storage-scope.spec.ts` 守住。
 *
 * 浏览器侧仍值得断言的：密钥**不进服务端** `llm_overrides.saved_models`
 * （`preset_for_server_put` 恒把 `api_key` 清空）。
 *
 * 运行：
 *   ./scripts/e2e-playwright.sh specs/mock-settings-model-registry.spec.ts
 */
import { expect, type Page, test } from "@playwright/test";
import {
  apiUrl,
  resolveWebApiBearerToken,
  seedSession,
} from "../fixtures/helpers";

const SETTINGS_VISIBLE = '[data-testid="settings-page"].settings-page-visible';
const LABEL = "E2E registry";
const API_BASE = "https://api.example.com/v1";
const MODEL = "e2e-test-model";
const PLAINTEXT_KEY = "E2E_STUB_KEY";

async function openLlmSettings(page: Page) {
  await page.getByTestId("settings-open").click();
  await expect(page.locator(SETTINGS_VISIBLE)).toBeVisible();
  await page.getByTestId("settings-nav-llm").click();
}

/** 服务端已持久化的模型预设列表（`GET /user-data/llm-overrides`）。 */
async function savedModelsFromServer(
  page: Page,
): Promise<Record<string, unknown>[]> {
  const bearer = resolveWebApiBearerToken();
  const res = await page.request.get(apiUrl("/user-data/llm-overrides"), {
    headers: bearer ? { Authorization: `Bearer ${bearer}` } : undefined,
  });
  if (!res.ok()) return [];
  const body = (await res.json()) as {
    saved_models?: Record<string, unknown>[];
  };
  return body.saved_models ?? [];
}

test("新增模型预设：列表出现该行，且服务端列表不含明文密钥", async ({
  page,
}) => {
  await seedSession(page, `s_e2e_model_registry_${Date.now()}`);
  await openLlmSettings(page);

  await page.getByTestId("settings-models-add").click();
  await page.getByTestId("settings-models-new-label").fill(LABEL);
  await page.getByTestId("settings-models-new-base").fill(API_BASE);
  await page.getByTestId("settings-models-new-model").fill(MODEL);
  await page.getByTestId("settings-models-new-key").fill(PLAINTEXT_KEY);
  await page.getByTestId("settings-models-add-submit").click();

  // 列表行（该列表没有 data-testid，原 Victauri 用例也按这个类名定位）
  await expect(
    page.locator(".settings-saved-models-label", { hasText: LABEL }),
  ).toBeVisible({ timeout: 15_000 });

  // 非机密字段随 llm-overrides 落服务端（写入有防抖，故轮询）
  await expect
    .poll(async () => (await savedModelsFromServer(page)).length, {
      timeout: 15_000,
    })
    .toBeGreaterThan(0);

  const saved = await savedModelsFromServer(page);
  const row = saved.find((m) => m.label === LABEL);
  expect(
    row,
    `服务端已保存模型应包含 ${LABEL}：${JSON.stringify(saved)}`,
  ).toBeTruthy();

  expect(row?.api_base).toBe(API_BASE);
  expect(row?.model).toBe(MODEL);
  // 明文密钥绝不进服务端
  expect(String(row?.api_key ?? "")).toBe("");
});
