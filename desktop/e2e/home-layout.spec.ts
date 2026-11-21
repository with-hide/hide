// The one-time move of an older daemon layout into ~/.hide under a private HOME.
// The staged app exercises the default state path without operator resources.

import { expect } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { stageBuild } from "./device-home";
import { HIDE_CLI, isolate, nodeOf, relaunch, screenshot, shellPage, test } from "./fixture";
import { startEngine } from "../../web/e2e/engine-fixture";
import { captureNativeWindow } from "./native-window";

test.describe.configure({ timeout: 240_000 });
test.skip(process.platform !== "darwin", "the staged bundle uses the macOS layout");

/**
 * A bundle around the staged build so its daemon applies the install kit.
 * It carries what the packaged app carries (`stageBuild`).
 */
function stageBundle(root: string): string {
  const build = stageBuild(root);
  const contents = path.join(root, "Hide.app", "Contents");
  fs.mkdirSync(path.join(contents, "MacOS"), { recursive: true });
  fs.renameSync(build, path.join(contents, "Resources"));
  fs.writeFileSync(path.join(contents, "Info.plist"), "<plist><dict><key>CFBundleExecutable</key><string>hide</string></dict></plist>\n");
  fs.writeFileSync(path.join(contents, "MacOS", "hide"), `#!/bin/sh\nexec ${JSON.stringify(process.execPath)} "$@"\n`, { mode: 0o755 });
  return path.join(contents, "Resources");
}

function json(command: string, args: string[], env: Record<string, string>): Record<string, unknown> {
  const run = spawnSync(command, args, { env, encoding: "utf8", timeout: 60_000 });
  const line = run.stdout.trim().split("\n").at(-1) ?? "";
  try { return JSON.parse(line) as Record<string, unknown>; } catch { throw new Error(`${command} ${args.join(" ")}: exit ${run.status}\n${run.stdout}\n${run.stderr}`); }
}

const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };

test("the first connect of a new app moves the old layout into ~/.hide once", async () => {
  const engine = await startEngine();
  const run = isolate(engine, "hl");
  const home = run.env.HOME!;
  const env: Record<string, string> = { ...run.env };
  for (const key of ["HIDE_STATE_DIR", "XDG_STATE_HOME"]) delete env[key];
  const resources = stageBundle(run.root);
  const appHide = path.join(resources, "hide");
  const legacyState = path.join(home, ".local", "state", "hide");
  let oldPid = 0;
  let newPid = 0;
  try {
    // An older build's daemon, running from the legacy folder with the operator's state beside it. It
    // runs outside a bundle, so it runs no kit pass.
    const old = json(HIDE_CLI, ["connect"], { ...env, HIDE_STATE_DIR: legacyState });
    oldPid = Number(old.pid);
    fs.writeFileSync(path.join(legacyState, "operator-file.txt"), "kept");
    const hostId = fs.readFileSync(path.join(legacyState, "host-id"), "utf8");
    // The labels era's leftovers on this Mac, and the shared parents that stay.
    fs.mkdirSync(path.join(home, ".local", "state", "hide-plugin-upgrade", "20260920T110346Z"), { recursive: true });
    fs.mkdirSync(path.join(home, ".local", "share", "hide", "agent-context-labels"), { recursive: true });
    fs.writeFileSync(path.join(home, ".local", "share", "hide", "agent-context-labels", "watcher.out"), "x");

    const connected = json(appHide, ["connect"], env);
    expect(connected.ok, JSON.stringify(connected)).toBe(true);
    newPid = Number(connected.pid);
    const moved = path.join(home, ".hide", "state");

    // B2, B3: one folder, the old daemon gone, one new daemon from ~/.hide/state.
    expect(newPid).not.toBe(oldPid);
    await expect.poll(() => alive(oldPid), { timeout: 10_000 }).toBe(false);
    expect(fs.existsSync(legacyState)).toBe(false);
    expect(fs.readFileSync(path.join(moved, "operator-file.txt"), "utf8")).toBe("kept");
    expect(fs.readFileSync(path.join(moved, "host-id"), "utf8")).toBe(hostId);
    expect(json(appHide, ["status", "--json"], env).pid).toBe(newPid);

    // B9: the labels era's folders are gone; ~/.local/state and ~/.local/share stay.
    await expect.poll(() => fs.existsSync(path.join(home, ".local", "share", "hide")), { timeout: 30_000 }).toBe(false);
    expect(fs.existsSync(path.join(home, ".local", "state", "hide-plugin-upgrade"))).toBe(false);
    expect(fs.statSync(path.join(home, ".local", "state")).isDirectory()).toBe(true);
    expect(fs.statSync(path.join(home, ".local", "share")).isDirectory()).toBe(true);

    // B4: a second connect attaches to the same daemon and moves nothing.
    expect(json(appHide, ["connect"], env).pid).toBe(newPid);
    expect(fs.existsSync(legacyState)).toBe(false);

    // B26: This Mac's row in Settings > Devices, in this checkout's window attached to that daemon.
    const app = await relaunch({ ...run.env, HIDE_STATE_DIR: moved, HIDE_CLI_PATH: appHide });
    try {
      const page = await shellPage(app);
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="devices"]').click();
      const movedNode = nodeOf({ HIDE_STATE_DIR: moved });
      await page.locator(`[data-device-menu="${movedNode}"]`).click();
      await page.locator(`[data-device-details="${movedNode}"]`).click();
      await expect(page.locator(`[data-kit-part="${movedNode}:coordination_retirement:installed"]`)).toBeVisible();
      await screenshot(page, "home-layout-this-mac-kit");
      await captureNativeWindow(app, "coordination-retirement-native", { state: moved, home, retirement: "installed" });
    } finally {
      await app.close().catch(() => undefined);
    }
  } finally {
    spawnSync(appHide, ["stop"], { env, timeout: 20_000 });
    if (oldPid && alive(oldPid)) spawnSync(HIDE_CLI, ["stop"], { env: { ...env, HIDE_STATE_DIR: legacyState }, timeout: 20_000 });
    run.cleanup();
    engine.stop();
  }
});
