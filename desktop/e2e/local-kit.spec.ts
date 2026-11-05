// This Mac's install kit from a real app bundle (PRD device-parity B1, B2,
// B7, B10): the bundle's own daemon, started with a private HOME, installs
// every part there at launch and writes nothing when it launches again.
// HIDE_E2E_APP names a packaged hide.app (`pnpm --dir desktop package`),
// never the operator's /Applications copy; the window is this checkout's
// desktop host, attached to that daemon.

import { expect, type ElectronApplication } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { openProject } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { connectStateDir } from "../../web/e2e/hided-control.mjs";
import { enterWorkspace } from "../../web/e2e/wire";
import {
  claudeSettings, codexDaemonWritten, codexHooks, readSettings, seedAgentFiles,
} from "./device-home";
import { hostLog, isolate, nodeOf, relaunch, screenshot, shellPage, test } from "./fixture";

test.describe.configure({ timeout: 300_000 });
test.skip(!process.env.HIDE_E2E_APP, "a packaged hide.app is required");

const PARTS = ["cli", "claude_code_hook", "codex_hook", "coordination_retirement"];

type Applied = { kind?: string; device_id?: string; components?: { id: string; state: string; reason: string | null }[] };

function applied(log: string, node: string): Applied[] {
  return fs.readFileSync(log, "utf8").split("\n").filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line) as Applied)
    .filter((line) => line.kind === "apply.completed" && line.device_id === node);
}

test("the app's daemon installs this Mac's kit into its HOME at launch and changes nothing the next time", async () => {
  const bundle = fs.realpathSync(process.env.HIDE_E2E_APP!);
  if (bundle.startsWith("/Applications/")) throw new Error("HIDE_E2E_APP must name a build, never the operator's installed app");
  const resources = path.join(bundle, "Contents", "Resources");
  const local = await startEngine();
  const run = isolate(local, "lk");
  // The release daemon and the host's discovery CLI must come from this bundle.
  run.env.HIDE_CLI_PATH = path.join(resources, "hide");
  const home = run.env.HOME!;
  const original = seedAgentFiles(home);
  const daemonLog = path.join(run.root, "daemon.log");
  const startDaemon = (): ChildProcess => {
    const output = fs.openSync(daemonLog, "a");
    const child = spawn(path.join(resources, "hided"), [], { env: run.env, stdio: ["ignore", output, output] });
    fs.closeSync(output);
    return child;
  };
  fs.writeFileSync(daemonLog, "");
  let daemon = startDaemon();
  let app: ElectronApplication | undefined;
  const errors: unknown[] = [];
  try {
    await expect.poll(() => run.hide(["status", "--json"]).stdout.includes('"running":true'), { timeout: 30_000 }).toBe(true);
    // The project the window opens below, made through the bundle's daemon as the web shell makes it.
    const control = await connectStateDir(run.env.HIDE_STATE_DIR!);
    try { await openProject(control, path.join(local.root, "fixture"), "fixture"); } finally { control.close(); }
    // B1: the first launch installs every part, asking nothing.
    await expect.poll(() => applied(daemonLog, nodeOf(run.env)).length, { timeout: 120_000 }).toBeGreaterThan(0);
    expect(applied(daemonLog, nodeOf(run.env))[0]!.components).toEqual(PARTS.map((id) => expect.objectContaining({ id, state: "installed" })));
    expect(fs.readlinkSync(path.join(home, ".local", "bin", "hide"))).toBe(path.join(resources, "hide"));
    // PRD settings-cleanup D-14: the kit reads Codex's daemon setting and never turns it off on its own.
    expect(codexDaemonWritten(home)).toBe(false);
    for (const [file, before] of [[claudeSettings(home), original.claude], [codexHooks(home), original.codex]] as const) {
      const now = readSettings(file);
      expect(JSON.stringify(now.hooks)).toContain(path.join(resources, "hide-agent-hooks"));
      expect(now.hooks.SessionStart).toEqual(expect.arrayContaining(before.hooks.SessionStart!));
      expect({ ...now, hooks: undefined }).toEqual({ ...before, hooks: undefined });
    }

    app = await relaunch(run.env);
    const page = await shellPage(app);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    await expect(page.locator('[data-settings="true"]')).toBeVisible();
    await page.locator('[data-settings-tab="devices"]').click();
    // PRD settings-cleanup B54: the healthy kit is on request in Connection details, never on the row.
    await page.locator(`[data-device-menu="${nodeOf(run.env)}"]`).click();
    await page.locator(`[data-device-details="${nodeOf(run.env)}"]`).click();
    for (const id of PARTS) await expect(page.locator(`[data-kit-part="${nodeOf(run.env)}:${id}:installed"]`)).toBeVisible();
    await screenshot(page, "this-mac-kit-installed");
    await page.keyboard.press("Escape");
    await expect(page.locator(`[data-device-details-dialog="${nodeOf(run.env)}"]`)).toHaveCount(0);
    await expect(page.locator(`[data-kit-reinstall="${nodeOf(run.env)}"]`)).toHaveCount(0);
    await page.locator('[data-settings-tab="agents"]').click();
    await expect(page.locator('[data-settings-tab="agents"]')).toHaveAttribute("data-state", "active");
    await screenshot(page, "this-mac-hooks");
    await app.close();
    app = undefined;

    // B10, engineering rule 11: the next launch finds every part current and writes nothing.
    const written = [claudeSettings(home), codexHooks(home)].map((file) => fs.statSync(file).mtimeMs);
    expect(run.hide(["stop"]).status).toBe(0);
    await expect.poll(() => daemon.exitCode !== null || daemon.signalCode !== null, { timeout: 30_000 }).toBe(true);
    daemon = startDaemon();
    await expect.poll(() => applied(daemonLog, nodeOf(run.env)).length, { timeout: 120_000 }).toBe(2);
    expect(applied(daemonLog, nodeOf(run.env))[1]!.components).toEqual(PARTS.map((id) => expect.objectContaining({ id, state: "installed" })));
    expect([claudeSettings(home), codexHooks(home)].map((file) => fs.statSync(file).mtimeMs)).toEqual(written);
  } catch (error) {
    errors.push(error);
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    console.log(fs.readFileSync(daemonLog, "utf8"));
  } finally {
    try { await app?.close(); } catch (error) { errors.push(error); }
    try { run.cleanup(); } catch (error) { errors.push(error); }
    try {
      if (daemon.exitCode === null && daemon.signalCode === null) daemon.kill("SIGTERM");
    } catch (error) { errors.push(error); }
    try { local.stop(); } catch (error) { errors.push(error); }
  }
  if (errors.length === 1) throw errors[0];
  if (errors.length) throw new AggregateError(errors, "private kit fixture failed; original and cleanup errors are retained");
});
