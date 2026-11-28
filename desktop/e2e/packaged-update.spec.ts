// A real update of the installed app (PRD auto-update D-28, D-29, B16, B17):
// version N, installed from its own installer, finds version N+1 on a local
// feed, downloads it, and the operator presses Restart in the Help menu; the
// installer of N+1 then replaces N while the engine's panes and the daemon keep
// running. The package lane runs it on Windows and Linux (deb), where both
// builds are made from one package with `desktop/scripts/installers.mjs`; a
// macOS update needs two Developer ID builds and is not run here. It is also
// a poor fit for this test there: an app a terminal or Playwright starts may
// not write its own bundle, so Squirrel asks for an administrator password
// to install, and after Restart it reopens the app from launchd with the
// account's real profile (docs/INSTALL.md, What each system supports).
//
//   HIDE_E2E_UPDATE_APP      the installed N's executable
//   HIDE_E2E_UPDATE_FEED     the folder holding N+1's installer and update file
//   HIDE_E2E_UPDATE_VERSION  N+1
//
// The installers write to this account's real install folder and package
// database, which is what is being tested; a CI runner's account is discarded
// with it. Never run it against the operator's installed app.

import { expect, type ElectronApplication } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import { openProject, splitPane, tabPanes, waitForPaneInput } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { connectStateDir, useControl } from "../../web/e2e/hided-control.mjs";
import { ownedHolders } from "../../web/e2e/pane-holders.mjs";
import { endWindowsProcesses, fixtureExecutable, windowsProcessTree, type WindowsProcess } from "../../web/e2e/platform-fixture";
import { panePrompt } from "../../web/e2e/pane-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { hostLog, isolate, launch, screenshot, shellPage, test } from "./fixture";
import { withProbe, type Probe } from "./terminal-probe";

const executable = process.env.HIDE_E2E_UPDATE_APP;
const feed = process.env.HIDE_E2E_UPDATE_FEED;
const next = process.env.HIDE_E2E_UPDATE_VERSION ?? "";
// No retry: the first attempt changes the installed version, so a retry
// would start from N+1 and test nothing.
test.describe.configure({ timeout: 360_000, retries: 0 });
test.skip(!executable || !feed || !next, "an installed app and a feed with its next version are required");
test.skip(process.platform === "darwin", "a macOS update needs two Developer ID builds");

/** The version the system's package records say is installed. */
function installedVersion(): string {
  if (process.platform === "win32") {
    const script = "Get-ItemProperty 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*' -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'hide' | ForEach-Object { $_.DisplayVersion }";
    return execFileSync("pwsh", ["-NoProfile", "-Command", script], { encoding: "utf8" }).trim();
  }
  return execFileSync("dpkg-query", ["-W", "-f", "${Version}", "hide"], { encoding: "utf8" }).trim();
}

/** Serves the feed folder over loopback HTTP, as GitHub's release downloads serve a release. */
async function serveFeed(dir: string): Promise<{ url: string; close: () => Promise<void> }> {
  const server = http.createServer((request, response) => {
    const name = decodeURIComponent(new URL(request.url ?? "/", "http://feed").pathname.slice(1));
    const file = path.join(dir, name);
    if (name.includes("/") || name.includes("\\") || !fs.statSync(file, { throwIfNoEntry: false })?.isFile()) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { "Content-Length": fs.statSync(file).size });
    fs.createReadStream(file).pipe(response);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return { url: `http://127.0.0.1:${port}/`, close: () => new Promise((resolve) => server.close(() => resolve())) };
}

/**
 * A `pkexec` that asks no one: the runner's account may use sudo without a
 * password, and no polkit agent runs to answer pkexec's prompt. The updater
 * installs a deb with the first of gksudo, kdesudo, pkexec or sudo it finds.
 */
function sudoShim(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-update-sudo-"));
  fs.writeFileSync(path.join(dir, "pkexec"), '#!/bin/sh\n[ "$1" = --disable-internal-agent ] && shift\nexec sudo -n "$@"\n', { mode: 0o755 });
  return dir;
}

/** A live process running the installed app's executable, with its arguments on Linux. */
type AppProcess = { pid: number; args?: string };

/**
 * The live processes running the installed app's executable. A process the
 * system still lists after it has ended is left out: on Windows the process
 * table names it for a moment while a handle to it is open (`HasExited` reads
 * its handle), and on Linux one waiting to be reaped is listed as
 * `[name] <defunct>`, which does not match.
 */
function appProcesses(app: string): AppProcess[] {
  if (process.platform === "win32") {
    const script = "Get-Process hide -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:HIDE_E2E_INSTALLED_APP -and -not $_.HasExited } | ForEach-Object { $_.Id }";
    const listed = execFileSync("pwsh", ["-NoProfile", "-Command", script], { encoding: "utf8", env: { ...process.env, HIDE_E2E_INSTALLED_APP: app } });
    return listed.split(/\r?\n/).filter(Boolean).map((pid) => ({ pid: Number(pid) }));
  }
  return execFileSync("ps", ["-eo", "pid=,args="], { encoding: "utf8" }).split("\n").flatMap((line) => {
    const match = /^\s*(\d+) (.*)$/.exec(line);
    if (!match) return [];
    const args = match[2]!;
    return args === app || args.startsWith(`${app} `) ? [{ pid: Number(match[1]), args }] : [];
  });
}

/** Electron's relauncher on Linux, which waits for the old app to exit and then starts the new one, is not the app. */
const isRelauncher = (listed: AppProcess): boolean => listed.args?.includes(" --type=relauncher") ?? false;

/**
 * Ends every process of the installed app, which after the update is the new
 * version the update opened and no test handle owns, and returns once none
 * is left.
 *
 * On Windows each one's tree is ended too: the reopened app runs with
 * Explorer's environment, not the test's, so its `hide connect` starts a
 * daemon in the runner's own home. `hide-processes` ends each process and
 * waits for its end on the process's own handle, not on a listing.
 */
async function endReopenedApp(app: string): Promise<void> {
  if (process.platform === "win32") {
    const owned = new Map<string, WindowsProcess>();
    for (const listed of appProcesses(app)) {
      for (const item of windowsProcessTree(listed.pid)) owned.set(`${item.id}:${item.created}`, item);
    }
    endWindowsProcesses([...owned.values()], path.dirname(app));
    return;
  }
  for (const listed of appProcesses(app)) {
    try { process.kill(listed.pid, "SIGKILL"); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
    }
  }
  await expect.poll(() => appProcesses(app).map((listed) => `${listed.pid} ${listed.args}`), {
    message: "every process of the reopened app ended", timeout: 5_000, // wait-ok: the killed processes leave the process table
  }).toEqual([]);
}

test("the installed app updates to the next version and the panes and daemon keep running", async () => {
  const file = fs.realpathSync.native(executable!);
  if (file.startsWith("/Applications/")) throw new Error("HIDE_E2E_UPDATE_APP must name a CI install, never the operator's installed app");
  const before = installedVersion();
  expect(before).not.toBe(next);
  const served = await serveFeed(path.resolve(feed!));
  const engine = await startEngine();
  const run = isolate(engine, "update");
  const shim = process.platform === "linux" ? sudoShim() : null;
  let app: ElectronApplication | null = null;
  // An app left running fails a test that passed; after a failure it is
  // logged, so the test's own error is the one reported.
  let failed = false;
  let leftRunning: unknown = null;
  try {
    if (process.platform === "win32") delete run.env.HIDE_CLI_PATH;
    else run.env.HIDE_CLI_PATH = path.join(path.dirname(file), "resources", fixtureExecutable("hide"));
    delete run.env.HIDED_UI_DIR;
    run.env.HIDE_UPDATE_FEED_URL = served.url;
    if (shim) run.env.PATH = `${shim}${path.delimiter}${run.env.PATH ?? ""}`;
    if (process.platform === "win32") fs.mkdirSync(path.join(run.env.APPDATA!, "Microsoft", "Windows", "Start Menu", "Programs"), { recursive: true });
    ({ app } = await launch(run.env, { executablePath: file }));
    const page = await shellPage(app);
    const daemon = run.daemonPid();
    expect(daemon).not.toBeNull();
    // Two shells in one tab of the fixture's folder, made through the app's daemon as the web shell makes them.
    const stateDir = run.env.HIDE_STATE_DIR!;
    const tab = await useControl(await connectStateDir(stateDir), async (control) => {
      const made = await openProject(control, path.join(engine.root, "fixture"), "fixture");
      const panes = [made.pane, await splitPane(control, made.tab, made.folder)];
      await withProbe(page);
      await enterWorkspace(page, "fixture");
      for (const pane of panes) {
        await waitForPaneInput(control, pane);
        await expect.poll(() => page.evaluate(id => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), pane), {
          message: `the shell in ${pane} ready before updating`, timeout: 20_000,
        }).toMatch(panePrompt(engine.shell));
      }
      if (process.platform !== "win32") {
        await expect.poll(() => ownedHolders(stateDir).filter(holder => {
          if (!holder.program) return false;
          try { process.kill(holder.program, 0); return true; }
          catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
        }).length, { message: "both holders and their shell processes ready before updating", timeout: 20_000 }).toBe(panes.length);
      }
      return made.tab;
    });
    // The panes the daemon lists, and off Windows the holder processes that run them.
    const panesNow = async () => {
      const reading = await connectStateDir(stateDir);
      try {
        // The first snapshot holds every layout, so an empty answer is a pane that is gone, not one not yet drawn.
        const listed = await reading.until((rest) => tabPanes(rest, tab).join(",") || "none", `the panes of ${tab}`);
        const holders = process.platform !== "win32" ? JSON.stringify(ownedHolders(stateDir)) : "";
        return `${listed};${holders}`;
      } finally { reading.close(); }
    };
    const panes = await panesNow();

    // The first look comes 15 s after launch; then the download (B15), and the ↑ button, its card opened once and Help's dot.
    const trigger = page.locator("[data-help-menu-trigger]");
    await expect(trigger.locator("[data-help-update-dot]")).toBeVisible({ timeout: 180_000 });
    await expect(trigger).toHaveAttribute("aria-label", /Update ready/);
    await expect(page.locator(`[data-update-button="${next}"]`)).toBeVisible();
    await expect(page.locator(`[data-update-card="${next}"]`)).toBeVisible();
    // Help opens with the update; its Restart now is the one pressed here.
    await trigger.click();
    await expect(page.locator(`[data-help-update-block="${next}"]`)).toContainText(`hide ${next} is ready`);
    const restart = page.locator(`[data-help-update="${next}"]`);
    await expect(restart).toHaveText("Restart now");
    await screenshot(page, "update-ready");
    expect(hostLog(run.env).map((line) => line.event)).toEqual(expect.arrayContaining(["update.check", "update.found", "update.ready"]));

    const closed = app.waitForEvent("close", { timeout: 120_000 });
    await restart.click();
    await closed;
    app = null;

    // The installer of N+1 replaced N (B16) and nothing the core runs ended (D-19).
    await expect.poll(installedVersion, { timeout: 180_000, intervals: [1_000] }).toBe(next);
    // Restart opens the new version once it is installed (B16). It is not
    // the app this test launched: on Windows the installer has Explorer start
    // it (electron-builder's NSIS `StartApp`), and Explorer does so on its own
    // time, sometimes after the installer has exited; on Linux Electron's
    // relauncher starts it once the old app has exited. It is ended below
    // only once it is there, or it would start after the end and stay.
    await expect.poll(() => appProcesses(file).filter((listed) => !isRelauncher(listed)).length, {
      message: `hide ${next} opened after the update`, timeout: 60_000, intervals: [1_000], // wait-ok: the update starts the new version
    }).toBeGreaterThan(0);
    expect(run.daemonPid()).toBe(daemon);
    expect(await panesNow()).toBe(panes);
    expect(hostLog(run.env).map((line) => line.event)).toContain("update.install");
  } catch (error) {
    failed = true;
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    try { await endReopenedApp(file); } catch (error) {
      if (failed) console.log(`ending the reopened app failed: ${error instanceof Error ? error.message : String(error)}`);
      else leftRunning = error;
    }
    if (shim) fs.rmSync(shim, { recursive: true, force: true });
    await served.close();
    run.cleanup();
    engine.stop();
  }
  if (leftRunning) throw leftRunning;
});
