import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { closePane, newTab, preparePaneInput, splitPane, withControl } from "../../web/e2e/daemon-actions";
import { endPaneHolders } from "../../web/e2e/pane-holders.mjs";
import { endWindowsProcesses, windowsProcessTree } from "../../web/e2e/platform-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { fitWindow, launchShell, startStack, test } from "./fixture";

// B16: native presentation of a real failed cold restore in both themes,
// with a long Korean cwd and a narrow window. OS keyboard input is not claimed.
// @platform: the engine's native folder becomes a wire path before the page renders it.
// The engine opens a pane whose folder moved away in the home folder with a notice, and
// marks a pane failed only when its shell cannot start, so the shell is the one thing that
// is missing when the engine's panes are made again.
test("failed terminals remain legible and keyboard reachable in a narrow native window", { tag: "@platform" }, async () => {
  test.skip(true, "own-restore-checkout 노드 대기: cold restore moves the nested folder's pane out of its original checkout");
  // Windows starts the integrated PowerShell, which no SHELL setting makes unstartable.
  test.skip(process.platform === "win32", "the engine's restore fails only for a shell that cannot start, which no setting does for Windows' integrated PowerShell");
  const stack = await startStack("restoration", { agents: false });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | null = null;
  try {
    const folder = path.join(fixture.root, "fixture", "복구할-작업-폴더-이름이-긴-경우에도-경로를-확인합니다");
    fs.mkdirSync(folder);
    // A tab whose only pane runs in that folder.
    const tab = await withControl(daemon, async (control) => {
      const made = await newTab(control, fixture, "복구 작업");
      // No window is open yet to report the panes' sizes. Finish each
      // attachment before the next geometry operation, as the view would.
      await preparePaneInput(control, made.pane);
      const pane = await splitPane(control, made.tab, toPage(folder));
      await preparePaneInput(control, pane);
      await closePane(control, made.pane);
      // Only this pane belongs to the cold-restore scenario. The missing
      // shell would fail every other saved pane too.
      for (const initial of fixture.panes) await closePane(control, initial);
      return made.tab;
    });
    // The engine's panes end and are made again from what was saved, as a reboot does.
    expect(run.hide(["stop"]).status).toBe(0);
    endPaneHolders(daemon.stateDir, process.platform === "win32" ? { endTree: (pid: number) => endWindowsProcesses(windowsProcessTree(pid)) } : {});
    run.env.SHELL = path.join(run.root, "no-such-shell");
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
    await page.locator(`[data-tab="${tab}"]`).click();
    const failed = page.locator("[data-pane-restore-failed]");
    await expect(failed).toHaveCount(1);
    await expect(failed).toContainText(toPage(folder));
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.writeFileSync(path.join(evidence, "native-restoration-identity.json"), JSON.stringify({ ...identity, daemonPid: run.daemonPid(), state: run.env.HIDE_STATE_DIR }));
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
  } finally { await app?.close(); stack.stop(); }
});
