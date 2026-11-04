// A device gets the same install kit as this Mac (PRD device-parity B13,
// B16, B8, D-26, B22, B23, B25): over a real SSH connection to an isolated
// sshd whose sessions get a private HOME (desktop/e2e/device-home.ts), with a
// private daemon, desktop profile and two engine fixtures, one standing in
// for the device's own.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startSshPaneFixture, type SshPaneFixture } from "./ssh-pane-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import {
  claudeSettings, codexHooks, deviceBridges, deviceHome,
  codexAutostart, codexDaemonAnswers, codexDaemonWritten, proveDeviceHome, startCodexDaemon, readSettings, resetDeviceHome, stageBuild, writeSshConfig, type AgentSettings,
} from "./device-home";
import { endChild, hostLog, isolate, relaunch, screenshot, shellPage, test } from "./fixture";

test.describe.configure({ timeout: 300_000 });
test.skip(!process.env.HIDE_E2E_SSH_PORT, "an isolated SSH server is required");

const DEVICE = "ssh-kit";
const ALIAS = "isolated-kit";
const SECOND_DEVICE = "ssh-kit-second";
const SECOND_ALIAS = "isolated-kit-second";
const PARTS = ["cli", "claude_code_hook", "codex_hook", "coordination_retirement"];

function quote(value: string): string { return `'${value.replaceAll("'", "'\\''")}'`; }

type DaemonEvent = {
  kind?: string; device_id?: string; target?: string; reason?: string; outcome?: unknown;
  components?: { id: string; state?: string; outcome?: unknown; reason?: string | null }[];
  upload?: { sent: number; reused: number; missing: string[] };
};

function daemonEvents(log: string): DaemonEvent[] {
  return fs.readFileSync(log, "utf8").split("\n").filter((line) => line.startsWith("{")).map((line) => JSON.parse(line) as DaemonEvent);
}

async function sendFrame(page: Page, state: { port: number; token: string }, frame: Record<string, unknown>): Promise<void> {
  await page.evaluate(async ({ port, token, frame }) => {
    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
      const timer = setTimeout(() => reject(new Error("frame timed out")), 10_000);
      ws.onerror = () => { clearTimeout(timer); reject(new Error("frame socket failed")); };
      ws.onopen = () => ws.send(JSON.stringify({ token, schema_version: 2 }));
      ws.onmessage = () => {
        ws.send(JSON.stringify({ schema_version: 2, ...frame }));
        clearTimeout(timer); ws.close(); resolve();
      };
    });
  }, { ...state, frame });
}

type DeviceRun = Awaited<ReturnType<typeof startDeviceRun>>;

/**
 * A private daemon started from a staged build, two engine fixtures (this
 * Mac's and the device's), the isolated sshd reached under `aliases`,
 * and the device's private HOME proved and reset.
 */
async function startDeviceRun(name: string, aliases: string[]) {
  const home = deviceHome();
  const local = await startSshPaneFixture();
  const device = await startSshPaneFixture();
  const run = isolate(local, name);
  const bridge = deviceBridges(home);
  const helper = path.join(run.root, "device-helper");
  const cliDir = path.join(run.root, "device-bin");
  run.env.HIDE_HOST_HELPER_ROOT = helper;
  run.env.HIDE_HOST_CLI_DIR = cliDir;
  writeSshConfig(run.env.HOME!, aliases);
  proveDeviceHome(run.env, aliases[0]!, home);
  const original = resetDeviceHome(home);
  const build = stageBuild(run.root);
  const daemonLog = path.join(run.root, "daemon.log");
  const daemonOutput = fs.openSync(daemonLog, "w");
  const daemon = spawn(path.join(build, "hided"), [], { env: run.env, stdio: ["ignore", daemonOutput, daemonOutput] });
  fs.closeSync(daemonOutput);
  await expect.poll(() => run.hide(["status", "--json"]).stdout.includes('"running":true'), { timeout: 30_000 }).toBe(true);
  await local.attach(run);
  return { home, local, device, run, bridge, helper, cliDir, original, daemonLog, daemon };
}

async function stopDeviceRun(setup: DeviceRun): Promise<void> {
  // The daemon's records go beside the screenshots when a run keeps them.
  const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (evidence) fs.copyFileSync(setup.daemonLog, path.join(evidence, `${path.basename(setup.run.root)}-daemon.jsonl`));
  setup.run.cleanup();
  try {
    await endChild(setup.daemon);
  } finally {
    setup.device.stop();
    setup.local.stop();
  }
}

/** The files Herdr's integrations for Claude Code and Codex leave in an account's home. */
function herdrIntegrations(home: string): string[] {
  return [path.join(home, ".claude", "hooks", "herdr-agent-state.sh"), path.join(home, ".codex", "herdr-agent-state.sh")];
}

/** Hide's command for `event`, as the kit wrote it into an agent runtime's file. */
function hideCommand(settings: AgentSettings, event: string, hooks: string): string {
  const command = settings.hooks[event]?.flatMap((group) => group.hooks).find((hook) => hook.command.includes(hooks))?.command;
  expect(command, `no Hide ${event} hook`).toBeTruthy();
  return command!;
}

/** The file with every entry naming Hide's hook binary taken out, as an operator deleting it by hand would. */
function withoutHide(settings: AgentSettings, hooks: string): AgentSettings {
  const events = Object.fromEntries(Object.entries(settings.hooks)
    .map(([event, groups]) => [event, groups
      .map((group) => ({ ...group, hooks: group.hooks.filter((hook) => !hook.command.includes(hooks)) }))
      .filter((group) => group.hooks.length > 0)])
    .filter(([, groups]) => groups.length > 0));
  return { ...settings, hooks: events };
}

/** Runs a shell line in the device's first pane and returns its exit status and output. */
async function inDevicePane(device: SshPaneFixture, line: string, label: string): Promise<{ status: number; stdout: string }> {
  return device.command(line, label);
}

test("a device gets this Mac's kit, keeps a part the operator removed out until Reinstall, and gives the kit back on removal", async () => {
  const setup = await startDeviceRun("kit", [ALIAS]);
  const { home, device, run, bridge, helper, cliDir, original, daemonLog } = setup;
  let app: ElectronApplication | undefined;
  try {
    app = await relaunch(run.env);
    const page = await shellPage(app);
    await enterWorkspace(page, "fixture");
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };

    // B13: allowing the helper is the only question; the connection installs every part.
    await sendFrame(page, state, { kind: "register_device", payload: {
      id: DEVICE, label: "SSH kit fixture", ssh_alias: ALIAS, host_consent: true,
    } });
    const applied = () => daemonEvents(daemonLog).filter((line) => line.kind === "apply.completed" && line.device_id === DEVICE);
    await expect.poll(() => applied().length, { timeout: 120_000 }).toBeGreaterThan(0);
    await device.attach(run, DEVICE);
    expect(applied()[0]!.components).toEqual(PARTS.map((id) => expect.objectContaining({ id, state: "installed" })));

    // B16: every part names the helper root's `current`, which outlives a build.
    const current = path.join(fs.realpathSync(helper), "current");
    const hooks = path.join(current, "hide-agent-hooks");
    expect(fs.readlinkSync(path.join(cliDir, "hide"))).toBe(path.join(current, "hide"));
    for (const [file, before] of [[claudeSettings(home), original.claude], [codexHooks(home), original.codex]] as const) {
      const now = readSettings(file);
      hideCommand(now, "SessionStart", hooks);
      // Another tool's entries and settings stay as they were.
      expect(now.hooks.SessionStart).toEqual(expect.arrayContaining(before.hooks.SessionStart!));
      expect({ ...now, hooks: undefined }).toEqual({ ...before, hooks: undefined });
    }
    // The kit puts in no Herdr integration on a device (PRD engine-bundle-4-herdr-removal D-2).
    for (const file of herdrIntegrations(home)) expect(fs.existsSync(file), file).toBe(false);
    // D-14: the kit reads Codex's daemon setting and never turns it off on its own.
    expect(codexDaemonWritten(home)).toBe(false);

    await page.locator("[data-open-settings]").click();
    await expect(page.locator('[data-settings="true"]')).toBeVisible();
    await page.locator('[data-settings-tab="devices"]').click();
    // PRD settings-cleanup B54: a healthy kit is not on the row; Connection details lists every part.
    await page.locator(`[data-device-menu="${DEVICE}"]`).click();
    await page.locator(`[data-device-details="${DEVICE}"]`).click();
    for (const id of PARTS) await expect(page.locator(`[data-kit-part="${DEVICE}:${id}:installed"]`)).toBeVisible();
    await screenshot(page, "device-kit-installed");
    await page.keyboard.press("Escape");
    await expect(page.locator(`[data-device-details-dialog="${DEVICE}"]`)).toHaveCount(0);
    await expect(page.locator(`[data-kit-problem="${DEVICE}"]`)).toHaveCount(0);
    await expect(page.locator(`[data-kit-reinstall="${DEVICE}"]`)).toHaveCount(0);
    await page.locator('[data-settings-tab="agents"]').click();
    await expect(page.locator('[data-settings-tab="agents"]')).toHaveAttribute("data-state", "active");
    await screenshot(page, "agents-hooks-per-machine");

    // B25, B26: the hook the kit installed runs in a device pane and names that device's Workspace, with no Memory there.
    const session = await inDevicePane(device, `printf '{}' | HIDE_STATE_DIR=${quote(path.dirname(bridge))} sh -c ${quote(hideCommand(readSettings(claudeSettings(home)), "SessionStart", hooks))}`, "device-session-start");
    expect(session.status).toBe(0);
    const context = (JSON.parse(session.stdout) as { hookSpecificOutput: { additionalContext: string } }).hookSpecificOutput.additionalContext;
    expect(context).toContain("Hide Workspace control is available for this session's checkout");
    expect(context).not.toContain("hide-memory-context");

    // D-26, B8: an entry the operator deleted stays deleted until Reinstall, which puts back only that part.
    fs.writeFileSync(claudeSettings(home), `${JSON.stringify(withoutHide(readSettings(claudeSettings(home)), hooks), null, 2)}\n`);
    await page.locator('[data-settings-tab="devices"]').click();
    await expect(page.locator(`[data-kit-problem="${DEVICE}"]`)).toContainText("Claude Code hook", { timeout: 60_000 });
    await expect(page.locator(`[data-kit-reinstall="${DEVICE}"]`)).toBeVisible();
    await screenshot(page, "device-kit-removed-part");
    expect(readSettings(claudeSettings(home))).toEqual(original.claude);
    const reinstalls = applied().length;
    await page.locator(`[data-kit-reinstall="${DEVICE}"]`).click();
    await expect(page.locator(`[data-kit-problem="${DEVICE}"]`)).toHaveCount(0, { timeout: 60_000 });
    await expect(page.locator(`[data-kit-reinstall="${DEVICE}"]`)).toHaveCount(0);
    await page.locator(`[data-device-menu="${DEVICE}"]`).click();
    await page.locator(`[data-device-details="${DEVICE}"]`).click();
    await expect(page.locator(`[data-kit-part="${DEVICE}:claude_code_hook:installed"]`)).toBeVisible({ timeout: 60_000 });
    await page.keyboard.press("Escape");
    expect(applied().length).toBeGreaterThan(reinstalls);
    hideCommand(readSettings(claudeSettings(home)), "SessionStart", hooks);

    // PRD codex-daemon-apply B3, B11: the operator's confirmed turn-off reaches the device's helper,
    // which turns autostart off and stops the daemon running there, in one answer.
    startCodexDaemon(home);
    const answered = () => daemonEvents(daemonLog).filter((line) => line.kind === "codex_daemon_off.answered" && line.device_id === DEVICE);
    await sendFrame(page, state, { kind: "codex_daemon_disable", payload: { device_id: DEVICE } });
    await expect.poll(() => answered().length, { timeout: 60_000 }).toBe(1);
    expect(answered()[0]!.outcome).toEqual({ state: "done" });
    expect(codexAutostart(home)).toBe(false);
    expect(codexDaemonAnswers(home)).toBe(false);

    // B22, B23: removal says what comes off and what stays, then takes only Hide's parts off.
    await page.locator(`[data-device-menu="${DEVICE}"]`).click();
    await page.locator(`[data-device-remove="${DEVICE}"]`).click();
    await expect(page.locator(`[data-device-remove-confirm="${DEVICE}"]`)).toBeVisible();
    await expect(page.locator(`[data-device-remove-kit="${DEVICE}"]`)).toContainText("records in ~/.hide stay");
    await screenshot(page, "device-remove-confirm");
    const beforeRemoval = daemonEvents(daemonLog).length;
    await page.locator('[data-device-remove-go="true"]').click();
    await expect.poll(() => daemonEvents(daemonLog).slice(beforeRemoval)
      .find((line) => line.device_id === DEVICE && /^device\.kit_(removed|left)$/.test(line.kind ?? ""))?.kind, { timeout: 60_000 }).toBe("device.kit_removed");
    expect(readSettings(claudeSettings(home))).toEqual(original.claude);
    expect(readSettings(codexHooks(home))).toEqual(original.codex);
    for (const file of herdrIntegrations(home)) expect(fs.existsSync(file), file).toBe(false);
    expect(fs.lstatSync(path.join(cliDir, "hide"), { throwIfNoEntry: false })).toBeUndefined();
    expect(fs.existsSync(helper)).toBe(false);
  } catch (error) {
    console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
    console.log(fs.readFileSync(daemonLog, "utf8"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    await stopDeviceRun(setup);
  }
});

// A device an older Hide ran carries that Hide's helper build, which
// `current` leads to, and the bridge folder its gone pane service left. The
// version-mismatch install points `current` at this build, which runs as
// `hided`, and removes the older build; the device's pane service removes the
// dead bridge folder when it starts.
test("a device an older Hide ran keeps only this build and no dead bridge folder", async () => {
  const setup = await startDeviceRun("upgrade", [ALIAS]);
  const { run, bridge, helper, daemonLog } = setup;
  const OLD_BUILD = "0123456789abcdef";
  fs.mkdirSync(path.join(helper, OLD_BUILD), { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(helper, OLD_BUILD, "hide-host-helper"), "#!/bin/sh\nexit 1\n", { mode: 0o700 });
  fs.symlinkSync(OLD_BUILD, path.join(helper, "current"));
  const leftover = path.join(bridge, "bridge-older-hide");
  fs.mkdirSync(leftover, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(leftover, "bootstrap.sock"), "");
  const long = new Date(Date.now() - 10 * 60_000);
  fs.utimesSync(leftover, long, long);
  let app: ElectronApplication | undefined;
  try {
    app = await relaunch(run.env);
    const page = await shellPage(app);
    await enterWorkspace(page, "fixture");
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    await sendFrame(page, state, { kind: "register_device", payload: {
      id: DEVICE, label: "SSH kit fixture", ssh_alias: ALIAS, host_consent: true,
    } });
    await expect.poll(() => daemonEvents(daemonLog).some((line) => line.kind === "apply.completed" && line.device_id === DEVICE), { timeout: 120_000 }).toBe(true);

    const builds = fs.readdirSync(helper).filter((name) => name !== "current");
    expect(builds).toHaveLength(1);
    expect(builds[0]).not.toBe(OLD_BUILD);
    expect(fs.readlinkSync(path.join(helper, "current"))).toBe(builds[0]);
    const installed = fs.readdirSync(path.join(helper, builds[0]!));
    expect(installed).toContain("hided");
    expect(installed).not.toContain("hide-host-helper");
    await expect.poll(() => fs.existsSync(leftover), { timeout: 60_000 }).toBe(false);
  } catch (error) {
    console.log(fs.readFileSync(daemonLog, "utf8"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    await stopDeviceRun(setup);
  }
});

// Two registrations that reach one account (two SSH aliases of one
// machine) connect together when the daemon starts: both upload the same
// build into one helper root and run the kit on one HOME at once. Removing
// one leaves the account's kit for the other; removing the last takes it off.
test("two registrations of one device account connect at once and share its kit until the last is removed", async () => {
  const setup = await startDeviceRun("kit2", [ALIAS, SECOND_ALIAS]);
  const { home, run, helper, cliDir, original, daemonLog } = setup;
  let app: ElectronApplication | undefined;
  try {
    app = await relaunch(run.env);
    const page = await shellPage(app);
    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    const both = [[DEVICE, ALIAS], [SECOND_DEVICE, SECOND_ALIAS]] as const;
    await Promise.all(both.map(([id, alias]) => sendFrame(page, state, { kind: "register_device", payload: {
      id, label: id, ssh_alias: alias, host_consent: true,
    } })));
    for (const [id] of both) {
      await expect.poll(() => daemonEvents(daemonLog).find((line) => line.kind === "apply.completed" && line.device_id === id)?.components, { timeout: 120_000 })
        .toEqual(PARTS.map((part) => expect.objectContaining({ id: part, state: "installed" })));
      const ready = daemonEvents(daemonLog).filter((line) => line.kind === "host.ready" && line.target === id);
      expect(ready.map((line) => line.upload?.missing)).toEqual(ready.map(() => []));
    }
    expect(daemonEvents(daemonLog).filter((line) => line.kind === "host.failed")).toEqual([]);
    const current = path.join(fs.realpathSync(helper), "current");
    expect(fs.readlinkSync(path.join(cliDir, "hide"))).toBe(path.join(current, "hide"));

    await sendFrame(page, state, { kind: "remove_device", payload: { device_id: DEVICE } });
    await expect.poll(() => daemonEvents(daemonLog).find((line) => line.device_id === DEVICE && /^device\.kit_/.test(line.kind ?? ""))?.reason, { timeout: 30_000 })
      .toBe("another registered device reaches the same account on that machine");
    expect(fs.readlinkSync(path.join(cliDir, "hide"))).toBe(path.join(current, "hide"));
    expect(JSON.stringify(readSettings(claudeSettings(home)))).toContain(path.join(current, "hide-agent-hooks"));

    await sendFrame(page, state, { kind: "remove_device", payload: { device_id: SECOND_DEVICE } });
    await expect.poll(() => daemonEvents(daemonLog).find((line) => line.device_id === SECOND_DEVICE && /^device\.kit_/.test(line.kind ?? ""))?.kind, { timeout: 60_000 })
      .toBe("device.kit_removed");
    expect(readSettings(claudeSettings(home))).toEqual(original.claude);
    expect(readSettings(codexHooks(home))).toEqual(original.codex);
    expect(fs.existsSync(helper)).toBe(false);
  } catch (error) {
    console.log(fs.readFileSync(daemonLog, "utf8"));
    throw error;
  } finally {
    await app?.close().catch(() => undefined);
    await stopDeviceRun(setup);
  }
});
