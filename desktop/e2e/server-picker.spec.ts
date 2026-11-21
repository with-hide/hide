import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { openProject, withControl } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { chord } from "../../web/e2e/chords";
import { enterWorkspace } from "../../web/e2e/wire";
import { openServerButton, startServer } from "../../web/e2e/server-fixture";
import { assertIsolated, isolate, launch, shellPage, test } from "./fixture";
import { compositorPresents } from "../../web/e2e/wait";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });
// Test-only candidate selection: a packaged executable runs its own bundled
// host and daemon. Absent this path, CI exercises the current desktop build.
const executablePath = process.env.HIDE_E2E_PACKAGED_APP;

test("native fixture refuses shared runtime state before launch", async () => {
  const engine = await startEngine();
  const run = isolate(engine, "state-guard");
  try {
    expect(() => assertIsolated(run.env)).not.toThrow();
    expect(() => assertIsolated({ ...run.env, HIDE_STATE_DIR: "" })).toThrow("HIDE_STATE_DIR");
  } finally {
    // This case launches no process and never invokes a daemon/kit command.
    run.cleanup();
    engine.stop();
  }
});

test("server picker in a background native window", async () => {
  // The app starts its own daemon, packaged or not, so none runs before the launch: an empty engine,
  // whose project is made once that daemon answers, over the repository and servers made beforehand.
  const engine = await startEngine();
  const run = isolate(engine, "server-picker");
  let app: ElectronApplication | null = null;
  const servers = [];
  try {
    if (executablePath) {
      run.env.HIDE_CLI_PATH = path.resolve(executablePath, "../../Resources/hide");
      delete run.env.HIDED_UI_DIR;
    }
    const cwd = fs.realpathSync(path.join(engine.root, "fixture"));
    execFileSync("git", ["init", "-q"], { cwd });
    execFileSync("git", ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "Fixture"], { cwd });
    servers.push(await startServer(cwd), await startServer(cwd, "::1"));
    const foreign = path.join(engine.root, "foreign-project");
    fs.mkdirSync(foreign);
    servers.push(await startServer(foreign, "::1", servers[0]!.port));
    ({ app } = await launch(run.env, { executablePath }));
    const page = await shellPage(app);
    const { port, token } = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    await withControl({ origin: `http://127.0.0.1:${port}`, token }, (control) => openProject(control, cwd, "fixture"));
    await enterWorkspace(page, "fixture");
    const candidate = await app.evaluate(({ BrowserWindow }) => ({ pid: process.pid, executable: process.execPath, resources: process.resourcesPath, source: BrowserWindow.getAllWindows()[0]!.getMediaSourceId(), windows: BrowserWindow.getAllWindows().length }));
    expect(candidate.windows).toBe(1);
    const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (dir) fs.writeFileSync(path.join(dir, "native-candidate.json"), JSON.stringify(candidate, null, 2));
    const shot = async (name: string) => {
      if (!dir) return;
      // The DOM assertion precedes the macOS compositor presenting the frame.
      // Match the other native capture fixtures before reading the exact window.
      await compositorPresents(page);
      execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", candidate.source.split(":")[1]!, path.join(dir, `${name}.png`)]);
    };
    // Settings by its chord, as an operator with the keyboard in the terminal would.
    const theme = async (value: "light" | "dark") => {
      await page.keyboard.press(chord("settings", "electron"));
      await expect(page.locator('[data-settings="true"]')).toBeVisible();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${value}"]`).click();
      await page.keyboard.press("Escape");
      await expect(page.locator("html")).toHaveClass(new RegExp(value));
    };
    for (const value of ["dark", "light"] as const) {
      await theme(value);
      const globe = openServerButton(page);
      await expect(globe).toHaveCount(1);
      // The globe's name says how many listeners the core knows; with two it
      // opens the picker rather than a page.
      await expect(globe).toHaveAccessibleName("Open server, 2 running", { timeout: 20_000 });
      await globe.click();
      await expect(page.locator("[data-server-port]")).toHaveCount(2);
      await shot(`native-server-picker-${value}`);
      await page.getByRole("button", { name: `127.0.0.1:${servers[0]!.port}`, exact: true }).click();
      await expect.poll(() => app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children.map((view) => (view as unknown as { webContents?: Electron.WebContents }).webContents?.getURL()))).toContain(`http://127.0.0.1:${servers[0]!.port}/`);
      await expect(globe).toHaveCount(1);
      await globe.click();
      await expect(page.locator("[data-server-port]")).toHaveCount(2);
      await page.getByRole("button", { name: `[::1]:${servers[1]!.port}`, exact: true }).click();
      await expect(page.locator("[data-browser-address]")).toHaveValue(`[::1]:${servers[1]!.port}`);
      await expect.poll(() => app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children.some((view) => (view as unknown as { webContents?: Electron.WebContents }).webContents?.getTitle() === "Workspace preview"))).toBe(true);
      await expect.poll(() => app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children.map((view) => (view as unknown as { webContents?: Electron.WebContents }).webContents?.getURL()))).toContain(`http://[::1]:${servers[1]!.port}/`);
      await shot(`native-server-opened-${value}`);
    }
  } finally {
    await app?.close();
    for (const server of servers) server.child.kill();
    run.cleanup();
    engine.stop();
  }
});
