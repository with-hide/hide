// A real update of the installed app (PRD auto-update D-28, D-29, B16, B17):
// version N, installed from its own installer, finds version N+1 on a local
// feed, downloads it, and the operator presses Restart in the Help menu; the
// installer of N+1 then replaces N while the Herdr server and the daemon keep
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
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { fixtureExecutable } from "../../web/e2e/platform-fixture";
import { hostLog, isolate, launch, screenshot, shellPage, test } from "./fixture";

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

/** Ends the app the installer reopened, which no test handle owns. */
function endReopenedApp(app: string): void {
  if (process.platform === "win32") {
    const script = `Get-Process hide -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq '${app.replace(/'/g, "''")}' } | Stop-Process -Force`;
    execFileSync("pwsh", ["-NoProfile", "-Command", script], { stdio: "inherit" });
  } else {
    try {
      execFileSync("pkill", ["-f", `^${app}( |$)`], { stdio: "inherit" });
    } catch {
      // pkill exits 1 when nothing matched: the app did not reopen.
    }
  }
}

test("the installed app updates to the next version and the Herdr server and daemon keep running", async () => {
  const file = fs.realpathSync.native(executable!);
  if (file.startsWith("/Applications/")) throw new Error("HIDE_E2E_UPDATE_APP must name a CI install, never the operator's installed app");
  const before = installedVersion();
  expect(before).not.toBe(next);
  const served = await serveFeed(path.resolve(feed!));
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "update");
  const shim = process.platform === "linux" ? sudoShim() : null;
  let app: ElectronApplication | null = null;
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
    const panes = JSON.stringify(herdr.run(["pane", "list"]));

    // The first look comes 15 s after launch; then the download (B15).
    const trigger = page.locator("[data-help-menu-trigger]");
    await expect(trigger.locator("[data-help-update-dot]")).toBeVisible({ timeout: 180_000 });
    await expect(trigger).toHaveAttribute("aria-label", /Update ready/);
    await trigger.click();
    const restart = page.locator(`[data-help-update="${next}"]`);
    await expect(restart).toHaveText(new RegExp(`hide ${next.replace(/\./g, "\\.")} ready.*Restart`));
    await screenshot(page, "update-ready");
    expect(hostLog(run.env).map((line) => line.event)).toEqual(expect.arrayContaining(["update.check", "update.found", "update.ready"]));

    const closed = app.waitForEvent("close", { timeout: 120_000 });
    await restart.click();
    await closed;
    app = null;

    // The installer of N+1 replaced N (B16) and nothing the core runs ended (D-19).
    await expect.poll(installedVersion, { timeout: 180_000, intervals: [1_000] }).toBe(next);
    expect(run.daemonPid()).toBe(daemon);
    expect(JSON.stringify(herdr.run(["pane", "list"]))).toBe(panes);
    expect(hostLog(run.env).map((line) => line.event)).toContain("update.install");
  } catch (error) {
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    endReopenedApp(file);
    if (shim) fs.rmSync(shim, { recursive: true, force: true });
    await served.close();
    run.cleanup();
    herdr.stop();
  }
});
