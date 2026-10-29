import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { fitWindow, isolate, launchShell, test } from "./fixture";

// B16: native presentation of a real failed cold restore in both themes,
// with a long Korean cwd and a narrow window. OS keyboard input is not claimed.
// @platform: Herdr's native folder becomes a wire path before the page renders it.
test("failed terminals remain legible and keyboard reachable in a narrow native window", { tag: "@platform" }, async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "restoration");
  let app: ElectronApplication | null = null;
  try {
    const folder = path.join(herdr.root, "fixture", "복구할-작업-폴더-이름이-긴-경우에도-경로를-확인합니다");
    fs.mkdirSync(folder);
    const made = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", folder, "--label", "복구 작업", "--no-focus"]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } };
    await herdr.restart(() => fs.renameSync(folder, `${folder}-moved`));
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    await fitWindow(app, { width: 900, height: 720 });
    const identity = await app.evaluate(({ BrowserWindow }) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error("expected exactly one candidate window");
      return { pid: process.pid, executable: process.execPath, window: windows[0]!.getMediaSourceId() };
    });
    await enterWorkspace(page);
    await page.locator(`[data-tab="${made.result.tab.tab_id}"]`).click();
    const failed = page.locator("[data-pane-restore-failed]");
    await expect(failed).toHaveCount(1);
    await expect(failed).toContainText(toPage(folder));
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.writeFileSync(path.join(evidence, "native-restoration-identity.json"), JSON.stringify({ ...identity, daemonPid: run.daemonPid(), state: run.env.HIDE_STATE_DIR, socket: herdr.socket }));
    for (const theme of ["dark", "light"]) {
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator("[data-interface-language]").click();
      await page.locator('[data-language-option="ko"]').click();
      // The closing select hands the keyboard back to its trigger from a
      // timer; wait for it, or that late hand-back takes the slider's End.
      await expect(page.locator("[data-interface-language]")).toBeFocused();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      const fontSize = page.locator('[data-font-size] [role="slider"]');
      await fontSize.focus();
      await page.keyboard.press("End");
      await expect(fontSize).toHaveAttribute("aria-valuenow", "17");
      await page.keyboard.press("Escape");
      await expect(page.locator("html")).toHaveAttribute("lang", "ko");
      await expect(failed).toContainText("터미널");
      expect(await failed.evaluate((node) => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(0);
      const close = failed.getByRole("button");
      await close.focus();
      await expect(close).toBeFocused();
      // Portable renderer evidence. The run's manual native observation
      // captures this candidate through the OS separately.
      if (evidence) await page.screenshot({ path: path.join(evidence, `restoration-renderer-ko-${theme}.png`) });
    }
    await page.keyboard.press("Enter");
    await expect(failed).toHaveCount(0);
  } finally { await app?.close(); run.cleanup(); herdr.stop(); }
});
