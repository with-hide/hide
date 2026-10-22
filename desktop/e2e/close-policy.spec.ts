import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { enterWorkspace, showExplorer } from "../../web/e2e/wire";
import { fitWindow, focusPage, isolate, launch, NEEDS_FOCUS, test } from "./fixture";

type Session = { herdr: HerdrFixture; app: ElectronApplication; page: Page; capture: (name: string) => void };

/** A fresh Herdr, hided and app in the fixture workspace with the Explorer shown; always torn down. */
async function inWorkspace(label: string, body: (session: Session) => Promise<void>): Promise<void> {
  const herdr = await startHerdr();
  const run = isolate(herdr, label);
  let app: ElectronApplication | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.md"), "# Notes\n\n한글과 English\n");
    fs.writeFileSync(path.join(herdr.root, "fixture", "page.html"), '<!doctype html><meta charset="utf-8"><title>Close page</title><h1>Browser keyboard owner</h1><input aria-label="Page input">');
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    const identity = await app.evaluate(({ BrowserWindow }) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error("expected exactly one candidate window");
      const window = windows[0]!;
      return { pid: process.pid, executable: process.execPath, window: window.getMediaSourceId() };
    });
    await fitWindow(app, { width: 1024, height: 1000 });
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    await enterWorkspace(page);
    // The window is a CI runner's screen wide on every machine; zoomed out,
    // the lane over the agents leaves both panes in sight.
    await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.webContents.setZoomFactor(0.6));
    if (evidence) fs.writeFileSync(path.join(evidence, `${label}-identity.json`), JSON.stringify({ ...identity, daemonPid: run.daemonPid(), state: run.env.HIDE_STATE_DIR, socket: herdr.socket }, null, 2));
    const capture = (name: string) => {
      if (evidence && process.platform === "darwin") {
        const id = identity.window.split(":")[1]!;
        execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", id, path.join(evidence, `${name}.png`)]);
      }
    };
    await showExplorer(page);
    await body({ herdr, app, page, capture });
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    herdr.stop();
  }
}

test("Command W closes the keyboard's display or pane, never its tab", async () => {
  await inWorkspace("close-policy", async ({ herdr, page, capture }) => {
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    const editor = page.locator("[data-editor-body] .cm-content");
    await expect(editor).toBeVisible();
    await editor.click();
    capture("native-close-view-before");
    await page.keyboard.press("Meta+KeyW");
    await expect(editor).toHaveCount(0);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    // The last view closed leaves the Explorer alone in its column (PRD lane-two-columns B12).
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "none");
    capture("native-close-view-tree");
    await page.locator('[data-explorer-row$="/notes.md"]').focus();
    await page.keyboard.press("Meta+KeyW");
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await page.keyboard.press("Meta+KeyE");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await page.locator(`[data-terminal-host="${herdr.panes[1]}"]`).click();
    await page.keyboard.press("Meta+KeyW");
    await expect(page.locator(`[data-pane-view="${herdr.panes[1]}"]`)).toHaveCount(0, { timeout: 15_000 });
    await expect(page.locator(`[data-pane-view="${herdr.panes[0]}"]`)).toBeVisible();
    capture("native-close-pane-survivor");
    await page.locator(`[data-terminal-host="${herdr.panes[0]}"]`).click();
    await page.keyboard.press("Meta+KeyW");
    await expect(page.locator(`[data-tab="${herdr.tab}"]`)).toHaveCount(0);
    await expect(page.locator("[data-pane-view]")).toHaveCount(0);
  });
});

test("Command W from the app menu closes a browser page that holds the keyboard, never the agents behind it", { tag: NEEDS_FOCUS }, async () => {
  await inWorkspace("close-policy-page", async ({ app, page }) => {
    // Browser pages live in native child contents, outside the shell DOM.
    await page.locator('[data-explorer-row$="/page.html"]').click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-browser"]').click();
    await expect(page.locator("[data-browser-slot]")).toBeVisible();
    // A page holds the keyboard only in the key window, so this test's window comes to the front.
    await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children.some((child) =>
      (child as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Close page")), { message: "the page has loaded" }).toBe(true);
    await focusPage(app, { title: "Close page" });
    // The native application menu uses the same Command W command while a
    // child page owns focus; it must not close the agents behind that page.
    await app.evaluate(({ Menu }) => {
      const item = Menu.getApplicationMenu()!.getMenuItemById("close_tab")!;
      item.click();
    });
    await expect(page.locator("[data-browser-slot]")).toHaveCount(0);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
  });
});
