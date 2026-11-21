// The packaged app as an operator installs it, with its window: its own
// Electron host, the `hide` CLI and daemon in its resources (on Windows run
// from the build's runtime copy under the private home; PRD auto-update
// D-10), and the web shell that daemon embeds, against a private HOME, state
// and engine. The package lane runs it on Windows, where `smoke-package.mjs`
// starts the same package without a window and no other job draws the
// packaged one. HIDE_E2E_PACKAGED_APP names the packaged executable (`hide.exe`
// in the install folder on Windows, `hide.app/Contents/MacOS/hide` on macOS),
// never the operator's installed copy.

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { openProject, withControl } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { fixtureExecutable } from "../../web/e2e/platform-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { hostLog, isolate, launch, screenshot, shellPage, test } from "./fixture";

const executable = process.env.HIDE_E2E_PACKAGED_APP;
test.describe.configure({ timeout: 180_000 });
test.skip(!executable, "a packaged app is required");

/** The packaged resources folder: `Contents/Resources` on macOS, `resources` beside the executable elsewhere. */
function packagedResources(file: string): string {
  return process.platform === "darwin" ? path.resolve(file, "..", "..", "Resources") : path.join(path.dirname(file), "resources");
}

test("the packaged app opens its window on the shell its own daemon serves", async () => {
  const file = fs.realpathSync.native(executable!);
  if (file.startsWith("/Applications/")) throw new Error("HIDE_E2E_PACKAGED_APP must name a build, never the operator's installed app");
  const resources = packagedResources(file);
  // The package starts its own daemon, so none runs before the launch: an empty engine, whose project is made once that daemon answers.
  const engine = await startEngine();
  const run = isolate(engine, "packaged");
  let app: ElectronApplication | null = null;
  try {
    // The host finds the package's own CLI, whose `hide connect` starts the
    // hided beside it; with no UI folder named, that daemon serves the shell
    // it embeds. On Windows that CLI is the runtime copy's, which the host
    // makes before it starts anything.
    if (process.platform === "win32") delete run.env.HIDE_CLI_PATH;
    else run.env.HIDE_CLI_PATH = path.join(resources, fixtureExecutable("hide"));
    delete run.env.HIDED_UI_DIR;
    // Every Windows profile has its Start Menu; Electron finds it with SHGetKnownFolderPath, which
    // fails for a missing folder, so the fixture's private profile needs one as a real profile has.
    const startMenu = process.platform === "win32" ? path.join(run.env.APPDATA!, "Microsoft", "Windows", "Start Menu", "Programs") : null;
    if (startMenu) fs.mkdirSync(startMenu, { recursive: true });
    ({ app } = await launch(run.env, { executablePath: file }));
    const page = await shellPage(app);
    expect(fs.realpathSync.native(await app.evaluate(() => process.resourcesPath))).toBe(fs.realpathSync.native(resources));
    expect(run.daemonPid()).not.toBeNull();
    if (process.platform === "win32") {
      const runtimeRoot = path.join(run.env.USERPROFILE!, ".hide", "runtime");
      const copies = fs.readdirSync(runtimeRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory());
      expect(copies).toHaveLength(1);
      const runtimeDir = path.join(runtimeRoot, copies[0]!.name);
      expect(fs.statSync(path.join(runtimeDir, "hide.exe")).isFile()).toBe(true);
      // The Windows-home contract preserves the suffix and uses ~/ before it.
      const parts = runtimeDir.split(path.sep);
      const loggedDir = /^[A-Za-z]:$/.test(parts[0]!) && parts[1]?.toLowerCase() === "users" ? `~/${parts.slice(3).join(path.sep)}` : runtimeDir;
      const prepared = hostLog(run.env).find((line) => line.event === "runtime.prepared");
      expect(prepared?.dir).toBe(loggedDir);
      expect(hostLog(run.env).find((line) => line.event === "cli.resolved")?.path).toBe(`${loggedDir}${path.sep}hide.exe`);
    }
    // Launch builds Electron's notification presenter on Windows, which makes the Start Menu
    // shortcut a toast is attributed through, named from the executable's product name, before
    // any notification shows.
    if (startMenu) await expect.poll(() => fs.existsSync(path.join(startMenu, "hide.lnk")), { timeout: 30_000 }).toBe(true);
    const { port, token } = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    await withControl({ origin: `http://127.0.0.1:${port}`, token }, (control) => openProject(control, path.join(engine.root, "fixture"), "fixture"));
    await enterWorkspace(page, "fixture");
    await expect(page.locator("[data-pane-view] .xterm").first()).toBeVisible();
    await screenshot(page, "packaged-window");
  } catch (error) {
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    engine.stop();
  }
});
