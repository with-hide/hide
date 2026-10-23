// The words the host draws itself (the app menu, the connection screen) in each
// language, on the real Electron build against a private Herdr and hided.

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { HIDE_CLI, hostLog, isolate, launch, relaunch, screenshot, test, type Isolated } from "./fixture";

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => {
  herdr = await startHerdr();
});

test.afterAll(() => {
  herdr?.stop();
});

test.beforeEach(() => {
  run = isolate(herdr, test.info().title.split(":")[0]!);
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

const storedChoice = () => path.join(run.env.HIDE_DESKTOP_USER_DATA_DIR!, "interface-language.json");

function store(language: string | null): void {
  fs.mkdirSync(run.env.HIDE_DESKTOP_USER_DATA_DIR!, { recursive: true });
  fs.writeFileSync(storedChoice(), JSON.stringify({ schema: 1, interface_language: language }));
}

const menuLabels = (target: ElectronApplication) =>
  target.evaluate(({ Menu }) => ({
    // The first menu carries the app's own name, which is not a translation.
    top: Menu.getApplicationMenu()?.items.slice(1).map((item) => item.label) ?? [],
    newTab: Menu.getApplicationMenu()?.getMenuItemById("new_tab")?.label ?? null,
  }));

// Expected texts are written out here, apart from the catalogs they come from.
const EXPECTED = {
  en: { lang: "en", top: ["File", "Edit", "View", "Pane", "Window", "Help"], newTab: "New tab", failed: "Can't connect to hided", reason: "The hide command was not found.", retry: "Retry" },
  ko: { lang: "ko", top: ["파일", "편집", "보기", "터미널", "윈도우", "도움말"], newTab: "새 탭", failed: "hided에 연결할 수 없어요", reason: "hide 명령을 찾지 못했어요.", retry: "다시 시도" },
  "zh-CN": { lang: "zh-CN", top: ["文件", "编辑", "视图", "窗格", "窗口", "帮助"], newTab: "新建标签页", failed: "无法连接 hided", reason: "未找到 hide 命令。", retry: "重试" },
  ja: { lang: "ja", top: ["ファイル", "編集", "表示", "ペイン", "ウィンドウ", "ヘルプ"], newTab: "新しいタブ", failed: "hidedに接続できません", reason: "hideコマンドが見つかりませんでした。", retry: "再試行" },
} as const;

for (const language of ["en", "ko", "zh-CN", "ja"] as const) {
  test(`stored-${language}: the connection screen and the menu use the last confirmed language before any daemon answers`, async () => {
    store(language);
    ({ app } = await launch({ ...run.env, HIDE_CLI_PATH: path.join(run.root, "bin", "hide") }));
    const page = await app.firstWindow();
    const expected = EXPECTED[language];
    await expect(page.locator("#reason")).toHaveText(expected.reason, { timeout: 20_000 });
    await expect(page.locator("h1")).toHaveText(expected.failed);
    await expect(page.getByRole("button", { name: expected.retry })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("lang", expected.lang);
    expect(await menuLabels(app)).toEqual({ top: expected.top, newTab: expected.newTab });
    await screenshot(page, `desktop-status-${language}`);
  });
}

test("unset: with no stored choice the system language decides, and an unsupported one reads English", async () => {
  ({ app } = await launch({ ...run.env, HIDE_DESKTOP_SYSTEM_LANGUAGE: "ja-JP", HIDE_CLI_PATH: path.join(run.root, "bin", "hide") }));
  const page = await app.firstWindow();
  await expect(page.locator("#reason")).toHaveText(EXPECTED.ja.reason, { timeout: 20_000 });
  expect((await menuLabels(app)).newTab).toBe(EXPECTED.ja.newTab);
  await app.close();
  app = null;
  ({ app } = await launch({ ...run.env, HIDE_DESKTOP_SYSTEM_LANGUAGE: "fr-FR", HIDE_CLI_PATH: path.join(run.root, "bin", "hide") }));
  await expect((await app.firstWindow()).locator("#reason")).toHaveText(EXPECTED.en.reason, { timeout: 20_000 });
  expect((await menuLabels(app)).newTab).toBe(EXPECTED.en.newTab);
});

test("choice: the language chosen in Settings relabels the menu, is kept for the next launch, and system returns to the OS language", async () => {
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  await enterWorkspace(page, "fixture");
  expect((await menuLabels(app)).newTab).toBe(EXPECTED.en.newTab);
  await page.locator("[data-open-settings]").click();
  await expect(page.locator("[data-interface-language]")).toBeEnabled();
  await page.locator("[data-interface-language]").click();
  await page.locator('[data-language-option="ko"]').click();
  await expect.poll(async () => (await menuLabels(app!)).newTab).toBe(EXPECTED.ko.newTab);
  expect((await menuLabels(app)).top).toEqual(EXPECTED.ko.top);
  expect(JSON.parse(fs.readFileSync(storedChoice(), "utf8"))).toEqual({ schema: 1, interface_language: "ko" });
  await screenshot(page, "desktop-menu-language-ko");

  // The next launch, with no hide to ask, still draws ko.
  await app.close();
  app = null;
  ({ app } = await launch({ ...run.env, HIDE_CLI_PATH: path.join(run.root, "bin", "hide") }));
  await expect((await app.firstWindow()).locator("#reason")).toHaveText(EXPECTED.ko.reason, { timeout: 20_000 });
  await app.close();
  app = null;

  // Back to the system language (en-US here), which the host resolves on its own.
  app = await relaunch({ ...run.env, HIDE_CLI_PATH: HIDE_CLI });
  const shell = await app.firstWindow();
  await enterWorkspace(shell, "fixture");
  await shell.locator("[data-open-settings]").click();
  await shell.locator("[data-interface-language]").click();
  await shell.locator('[data-language-option="system"]').click();
  await expect.poll(async () => (await menuLabels(app!)).newTab).toBe(EXPECTED.en.newTab);
  expect(JSON.parse(fs.readFileSync(storedChoice(), "utf8"))).toEqual({ schema: 1, interface_language: null });
});
