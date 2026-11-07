// A device's Home and the device rail over real SSH (PRD home-device-rail
// B2, B10, B15, B18, B25, B38, B46, with the rail rework of quick
// device-rail-badges B1, B3, B6). Run with an isolated sshd
// whose port, key and known_hosts are supplied in HIDE_E2E_SSH_*, and whose
// sessions get HIDE_E2E_DEVICE_HOME as HOME, so the device's `~/hide` is made
// there and never in the account's own home. The device's holders, the daemon,
// the helper install and the desktop profile stay private; the agent is the
// fixture's `claude` shim.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startSshPaneFixture, ownSshPaneRunning, type SshRest } from "./ssh-pane-fixture";
import { deviceHome as declaredDeviceHome, proveDeviceHome, stageBuild } from "./device-home";
import { enterWorkspace } from "../../web/e2e/wire";
import { HIDE_CLI, endChild, isolate, nodeOf, relaunch, shellPage, test } from "./fixture";
import { animationsFinished, compositorPresents } from "../../web/e2e/wait";

test.describe.configure({ timeout: 300_000 });
test.skip(!process.env.HIDE_E2E_SSH_PORT || !process.env.HIDE_E2E_DEVICE_HOME, "an isolated SSH server with its own session HOME is required");

const DEVICE = "mini-e2e";
// B46: a long Korean name on the rail tile, the header line and the band.
const DEVICE_LABEL = "연구실 빌드 서버 맥미니";

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

async function openStartPanel(page: Page): Promise<void> {
  await page.keyboard.press("Meta+KeyK");
  await expect(page.locator('[data-palette="Search"] [data-palette-input]')).toBeFocused();
  await page.keyboard.type("Start an agent");
  await page.locator('[data-palette-row="command:start-agent"]').click();
  await expect(page.locator("[data-start-panel]")).toBeVisible();
  await expect(page.locator("[data-start-text]")).toBeFocused();
}

/** Submits the start panel and waits until it closes; a refusal fails the test with its reason. */
async function submitStart(page: Page): Promise<void> {
  await page.locator("[data-start-submit]").click();
  const panel = page.locator("[data-start-panel]");
  const failure = panel.locator("[data-start-failure]");
  await expect
    .poll(async () => ((await failure.count()) > 0 ? `refused: ${await failure.innerText()}` : (await panel.count()) === 0 ? "closed" : "open"), { timeout: 90_000 })
    .toBe("closed");
}

async function chooseTarget(page: Page, option: string): Promise<void> {
  await page.locator("[data-start-panel] [data-start-target]").click();
  await page.locator(option).first().click();
}

/** The window as macOS draws it, in Light and Dark, when a run directory is named. */
async function capture(page: Page, app: ElectronApplication, name: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  const source = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.getMediaSourceId());
  for (const theme of ["dark", "light"] as const) {
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator(`[data-theme-option="${theme}"]`).click();
    await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
    await animationsFinished(page);
    await compositorPresents(page);
    execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.split(":")[1]!, path.join(dir, `${name}-${theme}.png`)]);
  }
}

async function startDeviceHomeRun() {
  const deviceHome = process.env.HIDE_E2E_DEVICE_HOME!;
  const local = await startSshPaneFixture();
  const device = await startSshPaneFixture();
  // The declared SSH HOME must carry the same ownership marker as the other
  // device fixtures before the native node can create holders there.
  if (fs.realpathSync(deviceHome) !== declaredDeviceHome()) {
    throw new Error("HIDE_E2E_DEVICE_HOME and HIDE_E2E_SSH_HOME must name the same private HOME");
  }
  // The device's panes find the fixture's `claude` shim before anything else.
  fs.writeFileSync(path.join(device.root, "home", ".zshenv"), `skip_global_compinit=1\nexport PATH="${path.join(device.root, "bin")}:$PATH"\n`);
  fs.mkdirSync(path.join(deviceHome, ".local", "bin"), { recursive: true });
  // The remote node's login shell and every new pane read this isolated
  // HOME. Its shim reports sessions through this build's real hook binary.
  fs.writeFileSync(path.join(deviceHome, ".zshenv"), [
    "skip_global_compinit=1",
    `export PATH='${path.join(device.root, "bin")}':"$PATH"`,
    `export HIDE_E2E_ROOT='${device.root}'`,
    `export HIDE_E2E_HOOKS='${path.join(path.dirname(HIDE_CLI), "hide-agent-hooks")}'`, "",
  ].join("\n"));
  const run = isolate(local, "device-home");
  run.env.HIDE_HOST_HELPER_ROOT = path.join(run.root, "remote-helper");
  run.env.HIDE_HOST_CLI_DIR = path.join(run.root, "remote-bin");
  run.env.PATH = `${path.join(local.root, "bin")}:${run.env.PATH ?? "/usr/bin:/bin"}`;
  const ssh = path.join(run.env.HOME!, ".ssh");
  fs.mkdirSync(ssh, { recursive: true });
  fs.copyFileSync(process.env.HIDE_E2E_SSH_KNOWN_HOSTS!, path.join(ssh, "known_hosts"));
  fs.writeFileSync(path.join(ssh, "config"), [
    "Host isolated-device", "  HostName 127.0.0.1", `  Port ${process.env.HIDE_E2E_SSH_PORT}`,
    `  User ${os.userInfo().username}`, `  IdentityFile ${process.env.HIDE_E2E_SSH_KEY}`, "  IdentityAgent none",
    `  SetEnv HIDE_MACHINE_ID=${DEVICE}`, "",
  ].join("\n"), { mode: 0o600 });
  proveDeviceHome(run.env, "isolated-device", fs.realpathSync(deviceHome));
  const build = stageBuild(run.root);
  const daemonLog = path.join(run.root, "daemon.log");
  const daemonOutput = fs.openSync(daemonLog, "w");
  const daemon = spawn(path.join(build, "hided"), [], { env: run.env, stdio: ["ignore", daemonOutput, daemonOutput] });
  fs.closeSync(daemonOutput);
  return { deviceHome, local, device, run, daemonLog, daemon };
}

test("a device's Home is made on its first start, the rail follows registration, and a device start lands on that device", async () => {
  const { deviceHome, local, device, run, daemonLog, daemon } = await startDeviceHomeRun();
  let app: ElectronApplication | undefined;
  try {
    await expect.poll(() => run.hide(["status", "--json"]).stdout.includes('"running":true'), { timeout: 30_000 }).toBe(true);
    await local.attach(run);
    // The app attaches to the daemon already running, so the window is read through the main process.
    app = await relaunch(run.env);
    const page = await shellPage(app);
    await enterWorkspace(page, "fixture");
    const nav = page.locator("nav[data-sidebar]");

    // B1: one device, and the rail still shows (This Mac alone, no Inbox); Home has no row (sidebar-titlebar-refresh B11).
    await expect(nav).toHaveAttribute("data-sidebar-rail", "device");
    await expect(page.locator("[data-rail-tile]")).toHaveCount(1);
    await expect(nav.getByRole("button", { name: /^Home\b/ })).toHaveCount(0);
    await capture(page, app, "device-home-one-device");

    // B6: View > Toggle device rail hides the rail, the name becomes the device menu, and the menu item brings it back.
    await app.evaluate(({ Menu }) => Menu.getApplicationMenu()!.getMenuItemById("toggle_device_rail")!.click());
    await expect(nav).toHaveAttribute("data-sidebar-rail", "none");
    await expect(page.locator("[data-sidebar-device-menu]")).toBeVisible();
    await capture(page, app, "device-home-rail-hidden");
    await app.evaluate(({ Menu }) => Menu.getApplicationMenu()!.getMenuItemById("toggle_device_rail")!.click());
    await expect(nav).toHaveAttribute("data-sidebar-rail", "device");

    const state = JSON.parse(fs.readFileSync(path.join(run.env.HIDE_STATE_DIR!, "hided.json"), "utf8")) as { port: number; token: string };
    await sendFrame(page, state, { kind: "register_device", payload: {
      label: DEVICE_LABEL, ssh_alias: "isolated-device", host_consent: true,
    } });

    // B1: the device's tile joins the rail with This Mac still in front and the same screen.
    await expect(page.locator("[data-rail-tile]")).toHaveCount(2);
    await expect(page.locator(`[data-rail-tile="${nodeOf(run.env)}"]`)).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-rail-tile="${DEVICE}"]`)).toHaveAttribute("data-rail-connected", "true", { timeout: 120_000 });
    // A device Home start needs the helper, which installs after the route connects.
    await expect
      .poll(() => fs.readFileSync(daemonLog, "utf8").includes('"kind":"host.ready","target":"mini-e2e"'), { timeout: 120_000 })
      .toBe(true);
    await device.attach(run, DEVICE, deviceHome);
    // Before any Home start the device has no ~/hide (B18).
    expect(fs.existsSync(path.join(deviceHome, "hide"))).toBe(false);

    // B38, B18: a start aimed at the device's Home makes ~/hide there and runs the agent on that device.
    await openStartPanel(page);
    await chooseTarget(page, `[data-start-target-option="home:${DEVICE}"]`);
    await expect(page.locator("[data-start-panel] [data-start-target]")).toHaveAttribute("data-start-target", `home:${DEVICE}`);
    await expect(page.locator("[data-start-panel] [data-agent-kind]")).toHaveAttribute("data-agent-kind", "claude");
    await page.locator("[data-start-text]").fill("홈에서 첫 지시 확인");
    await submitStart(page);
    await expect.poll(() => fs.existsSync(path.join(deviceHome, "hide", ".hide-home.json")), { timeout: 60_000 }).toBe(true);
    for (const guide of ["AGENTS.md", "CLAUDE.md"]) expect(fs.existsSync(path.join(deviceHome, "hide", guide))).toBe(true);
    await expect.poll(() => ownSshPaneRunning(deviceHome, DEVICE, "홈에서 첫 지시 확인")?.info ?? "", { timeout: 60_000 }).toContain("claude");
    const homeAgent = ownSshPaneRunning(deviceHome, DEVICE, "홈에서 첫 지시 확인")!;
    const homeCwd = homeAgent.cwd!;
    expect(homeCwd).toContain(fs.realpathSync(path.join(deviceHome, "hide")));
    // B15, B2: the center, rail and sidebar moved to that device's pane together, under the device band.
    await expect(page.locator(`[data-device-band="${DEVICE}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-rail-tile="${DEVICE}"]`)).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-sidebar-title-name]")).toContainText(DEVICE_LABEL);
    await expect(page.locator('nav[aria-label="Location"]')).toHaveText("~/hide");
    await capture(page, app, "device-home-started");

    // B25: with the device's Home pane in front, the panel aims at that device's Home.
    await openStartPanel(page);
    await expect(page.locator("[data-start-panel] [data-start-target]")).toHaveAttribute("data-start-target", `home:${DEVICE}`);
    // B38: a device checkout start runs there too.
    await chooseTarget(page, `[data-start-target-option^="checkout:${DEVICE}:"][data-start-target-option$="/fixture"]`);
    await page.locator("[data-start-text]").fill("체크아웃 지시 확인");
    await submitStart(page);
    await expect.poll(() => ownSshPaneRunning(deviceHome, DEVICE, "체크아웃 지시 확인")?.info ?? "", { timeout: 60_000 }).toContain("claude");
    await expect(page.locator(`[data-device-band="${DEVICE}"]`)).toBeVisible();

    // The raw shim starts idle; give its Home pane a real detected question for the counts and rail mark.
    const session = `device-home-${process.pid}`;
    await device.send(`\x1b]7770;session=${session}\x07`, homeAgent.pane);
    const homeRow = (rest: unknown) => (rest as SshRest).status?.remote?.find((row) => row.target_id === DEVICE)?.session?.agents.find((row) => row.pane_id === homeAgent.pane);
    await device.control.until((rest) => homeRow(rest)?.session_id === session, "the device's own session report");
    await device.send("\x1b]0;Fixture\x07\x1b[2J\x1b[Hdo you want to proceed?\nbash command\n❯ 1. Yes\n2. No\n", homeAgent.pane);
    await device.control.until((rest) => homeRow(rest)?.state.needs_you, "the device's real permission screen");
    await expect(page.locator("nav[data-sidebar]")).toBeVisible();
    await expect(page.locator("nav[data-sidebar] [data-device-chip]")).toHaveCount(0);
    await expect(page.locator('[data-raised-group="needs_you"]')).toBeVisible();
    await expect(page.locator(`[data-rail-tile="${DEVICE}"] [data-rail-badge]`).first()).toBeVisible();
    await capture(page, app, "device-home-device-agents");
    // B10: removing the device in front moves the front to This Mac; the rail stays with its one tile, and the device keeps ~/hide and its agents.
    await sendFrame(page, state, { kind: "remove_device", payload: { device_id: DEVICE } });
    await expect(page.locator("[data-rail-tile]")).toHaveCount(1, { timeout: 30_000 });
    await expect(nav).toHaveAttribute("data-sidebar-rail", "device");
    await expect(page.locator(`[data-device-band="${DEVICE}"]`)).toHaveCount(0);
    expect(fs.existsSync(path.join(deviceHome, "hide", ".hide-home.json"))).toBe(true);
    expect(ownSshPaneRunning(deviceHome, DEVICE, "홈에서 첫 지시 확인")).not.toBeNull();
  } finally {
    await app?.close();
    await endChild(daemon);
    const keep = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (keep && fs.existsSync(daemonLog)) fs.copyFileSync(daemonLog, path.join(keep, "device-home-daemon.log"));
    // The core's diagnostic log, where each step of a start is recorded.
    const diagnostics = path.join(run.env.HIDE_STATE_DIR!, "Logs");
    if (keep && fs.existsSync(diagnostics)) fs.cpSync(diagnostics, path.join(keep, "device-home-diagnostics"), { recursive: true });
    run.cleanup();
    local.stop();
    device.stop();
  }
});
