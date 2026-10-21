// The exact candidate window, with private PTYs and renderer-addressed input.
import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { labelAgent, startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { compositorPresents } from "../../web/e2e/wait";
import { captureElectronWindow } from "./electron-window";
import { fitWindow, isolate, launchShell, test } from "./fixture";

const paneView = (page: Page, id: string) => page.locator(`[data-pane-view="${id}"]`);

test("pane moves render and cancel in a narrow candidate with long titles and large text", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "pane-move");
  let app: ElectronApplication | undefined;
  try {
    const source = herdr.panes[0], target = herdr.panes[1];
    labelAgent(herdr, source, { task: "로그인 화면과 예외 처리의 진행 상황 확인하기" });
    labelAgent(herdr, target, { task: "Review authentication changes" });
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 920, height: 760 });
    await enterWorkspace(page, "fixture");
    for (const id of [source, target]) await expect(paneView(page, id)).toHaveAttribute("data-transport", "controlling");
    await expect(paneView(page, source).locator("[data-pane-move-title]")).toContainText("로그인", { timeout: 30_000 });
    await expect(paneView(page, target).locator("[data-pane-move-title]")).toContainText("Review");
    const facts = { build: "worktree debug candidate", daemonPid: run.daemonPid(), socket: herdr.socket, state: run.env.HIDE_STATE_DIR, input: "Playwright renderer", provider: "controlled fixture CLI", window: "sole candidate, shown inactive" };
    for (const [theme, language] of [["dark", "ko"], ["light", "en"]] as const) {
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      const size = page.locator("[data-font-size] [role=slider]");
      await size.focus();
      await page.keyboard.press("End");
      await expect(size).toHaveAttribute("aria-valuenow", await size.getAttribute("aria-valuemax") ?? "");
      await page.keyboard.press("Escape");
      // The visible gap after the title participates in the same first-row handle.
      const title = paneView(page, source).locator("[data-pane-move-title]");
      const box = (await title.boundingBox())!;
      await page.mouse.move(box.x + box.width - 2, box.y + box.height / 2);
      await page.mouse.down();
      const destination = (await paneView(page, target).boundingBox())!;
      await page.mouse.move(destination.x + destination.width * 0.9, destination.y + destination.height / 2, { steps: 6 });
      await expect(page.locator('[data-pane-move-target="pane"]')).toBeVisible();
      await compositorPresents(page);
      await captureElectronWindow(app, `pane-move-${theme}-preview`, { ...facts, theme, language });
      await page.keyboard.press("Escape");
      await page.mouse.up();
      await expect(page.locator('[data-pane-move-target="pane"]')).toHaveCount(0);
      await paneView(page, source).locator(`[data-pane-menu="${source}"]`).focus();
      await page.keyboard.press("Enter");
      await expect(page.locator('[data-menu-item="pane_move"]')).toBeVisible();
      await page.keyboard.press("Escape");
    }
    const from = (await paneView(page, source).locator("[data-pane-move-title]").boundingBox())!;
    const to = (await paneView(page, target).boundingBox())!;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(to.x + to.width / 2, to.y + to.height * 0.9, { steps: 6 });
    await page.mouse.up();
    await expect.poll(async () => {
      const a = await paneView(page, source).boundingBox(), b = await paneView(page, target).boundingBox();
      return Boolean(a && b && a.y > b.y);
    }).toBe(true);
    await expect(page.locator("[data-pane-move-status]")).toHaveCount(0);
    await captureElectronWindow(app, "pane-move-completed", facts);
  } finally {
    try { await app?.close(); }
    finally { try { run.cleanup(); } finally { herdr.stop(); } }
  }
});
