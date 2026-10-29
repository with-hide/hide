// `hide <path>` from a terminal on this Mac: the folder becomes a project
// through the same registration Add a project sends, and the app's window,
// hidden here, comes back showing it in place of All projects or an Overview.

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { chord } from "../../web/e2e/chords";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { hostLog, isolate, launch, test } from "./fixture";

const windowVisible = (app: ElectronApplication) =>
  app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.isVisible() ?? false);

const hideWindow = (app: ElectronApplication) =>
  app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.hide());

test("hide <path> adds a folder as a project, brings it forward again, and calls the window back", async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, "open-path");
  let app: ElectronApplication | undefined;
  try {
    const home = run.env.HOME!;
    const beta = path.join(home, "projects", "beta");
    fs.mkdirSync(path.join(beta, "src"), { recursive: true });
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await expect(page.locator("[data-main-screen], [data-workspace-screen]")).toBeVisible({ timeout: 30_000 });
    const projects = page.locator("[data-project-list]");
    await expect(projects).not.toContainText("beta");

    // A folder no project holds is added, and the window that was out of sight is shown again.
    await hideWindow(app);
    await expect.poll(() => windowVisible(app!)).toBe(false);
    const added = run.hide([beta]);
    expect(added.stderr).toBe("");
    expect(added.status).toBe(0);
    expect(added.stdout).toMatch(/^Added beta \(.+\/projects\/beta\) as a project and brought it forward in Hide\./);
    await expect.poll(() => windowVisible(app!)).toBe(true);
    await expect(projects).toContainText("beta", { timeout: 20_000 });
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    expect(hostLog(run.env).some((line) => line.event === "come_forward.shown")).toBe(true);
    const shots = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (shots) await page.screenshot({ path: path.join(shots, "open-path-added.png") });

    // A folder inside it is the same project: brought forward, never added
    // twice, and an Overview opened over it closes so the project shows.
    await page.keyboard.press(chord("overview", "electron"));
    await expect(page.locator("[data-overview-modal]")).toBeVisible();
    await hideWindow(app);
    const again = run.hide(["open", path.join(beta, "src")]);
    expect(again.status).toBe(0);
    expect(again.stdout).toMatch(/^Brought beta \(.+\/projects\/beta\) forward in Hide\./);
    await expect.poll(() => windowVisible(app!)).toBe(true);
    await expect(page.locator("[data-overview-modal]")).toHaveCount(0);
    await expect(workspace).toBeVisible();
    await expect(projects.getByText("beta", { exact: true })).toHaveCount(1);

    // A path with nothing there fails with what to do, and nothing is added.
    const missing = run.hide([path.join(home, "projects", "gone")]);
    expect(missing.status).not.toBe(0);
    expect(missing.stderr).toContain("no folder is there; check the path");
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});
