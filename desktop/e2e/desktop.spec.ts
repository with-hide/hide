// The desktop app end to end: a private Herdr server with two fake agent
// panes, a private hided the app starts through `hide connect`, and the
// real Electron build. Each test owns its state directory and so its daemon.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import "../../web/src/host";
import { chord, commandLabel } from "../../web/e2e/chords";
import { fixtureExecutable } from "../../web/e2e/platform-fixture";
import { herdrTabs } from "../../web/e2e/failure-dump";
import { countSent, enterWorkspace } from "../../web/e2e/wire";
import { toPage } from "../src/main/wirePath";
import { detachedApp, DESKTOP_DIR, HIDE_CLI, hostLog, isolate, launch, relaunch, screenshot, test, type Isolated } from "./fixture";

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => {
  herdr = await startHerdr();
});

test.afterAll(() => {
  herdr?.stop();
});

test.beforeEach(() => {
  run = isolate(herdr, test.info().title.split(":")[0]!);
});

test.afterEach(async () => {
  const info = test.info();
  // The host's log says which discovery step failed; it goes with the failure.
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

async function shellShown(page: Page): Promise<void> {
  await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 30_000 });
}

// @platform: `hide connect` starts hided through the system's processes and local stream, and the window, menu and chords are the system's own set.
test("attach: the app starts hided, shows the shell, and runs the native chords and the menu", { tag: "@platform" }, async () => {
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  // B1: the host's own screen while it looks, then the shell hided serves.
  await shellShown(page);
  expect(new URL(page.url()).hostname).toBe("127.0.0.1");
  expect(run.daemonPid()).not.toBeNull();
  await enterWorkspace(page);
  await expect(page.locator("[data-pane-view]").first()).toBeVisible();
  await expect(page.locator("[data-pane-view] .xterm").first()).toBeVisible();
  await screenshot(page, "desktop-attached");

  // B11: no Node API in the page; the bridge is the host kind and OS, the menu
  // channel, the pane-chord report, the file manager reveal, Add a project's folder
  // picker, the browser displays' views (issue 155), the Factory's and the
  // agents' system notifications with their click back and withdrawal, and
  // macOS's notification permission with its settings page, and the restart of
  // the operator's older Herdr server (PRD herdr-takeover B12).
  expect(
    await page.evaluate(() => ({
      require: typeof (globalThis as { require?: unknown }).require,
      process: typeof (globalThis as { process?: unknown }).process,
      bridge: Object.keys(window.hideHost ?? {}).sort(),
      kind: window.hideHost?.kind,
    })),
  ).toEqual({ require: "undefined", process: "undefined", bridge: ["browser", "editorAvailable", "kind", "notificationPermission", "notify", "onCommand", "onNotificationOpen", "onWindowChrome", "openInEditor", "openNotificationSettings", "openPath", "pickFolder", "platform", "probePaths", "quit", "reportBadge", "reportBindings", "reportLanguage", "restartHerdr", "revealPath", "updates", "withdrawNotifications"], kind: "electron" });
  // sidebar-titlebar-refresh B17, B23: the host says what its frame takes of the shell's top row, traffic lights on macOS only, and the row keeps their room.
  const chrome = await page.evaluate(() => new Promise((resolve) => window.hideHost!.onWindowChrome(resolve)));
  expect(chrome).toEqual({ trafficLights: process.platform === "darwin", fullscreen: false });
  await expect(page.locator("[data-title-row]")).toHaveAttribute("data-traffic-lights", String(process.platform === "darwin"));
  // The sidebar's switch in that row hides the sidebar and, from the same place, brings it back without the chord.
  const sidebarSwitch = page.locator("[data-title-row] [data-sidebar-switch]");
  const switchX = Math.round((await sidebarSwitch.boundingBox())!.x);
  await sidebarSwitch.click();
  await expect(page.locator("nav[data-sidebar]")).toHaveCount(0);
  expect(Math.round((await sidebarSwitch.boundingBox())!.x)).toBe(switchX);
  await screenshot(page, "desktop-sidebar-hidden");
  await sidebarSwitch.click();
  await expect(page.locator("nav[data-sidebar]")).toBeVisible();

  // View > Toggle device rail hides and brings back the rail (quick device-rail-badges B6); the item has no chord.
  await expect(page.locator("[data-device-rail]")).toBeVisible();
  expect(await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("toggle_device_rail")?.accelerator ?? null)).toBeNull();
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("toggle_device_rail")?.click());
  await expect(page.locator("[data-device-rail]")).toHaveCount(0);
  await expect(page.locator("[data-sidebar-device-menu]")).toBeVisible();
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("toggle_device_rail")?.click());
  await expect(page.locator("[data-device-rail]")).toBeVisible();

  // The Agent and View area commands ⌘K no longer carries are Pane menu items with no chord until Settings gives one (PRD cmdk-navigation B24).
  const areaItems = await app.evaluate(({ Menu }) => {
    const menu = Menu.getApplicationMenu();
    return ["focus_next_agent_area", "focus_previous_agent_area", "grow_agent_area", "shrink_agent_area", "focus_next_view_area", "focus_previous_view_area", "grow_view_area", "shrink_view_area"].map((id) => {
      const item = menu?.getMenuItemById(id);
      return { id, label: item?.label ?? null, accelerator: item?.accelerator ?? null };
    });
  });
  expect(areaItems.map((item) => item.label)).toEqual(["Focus next Agent area", "Focus previous Agent area", "Grow Agent area", "Shrink Agent area", "Focus next View area", "Focus previous View area", "Grow View area", "Shrink View area"]);
  expect(areaItems.every((item) => item.accelerator === null)).toBe(true);

  // B9: the shortcuts sheet lists this host's chords, with no "moved for Chrome" note.
  await page.keyboard.press(chord("shortcuts", "electron"));
  const sheet = page.locator("[data-shortcut-sheet]");
  await expect(sheet).toBeVisible();
  await expect(sheet.getByText("desktop app")).toBeVisible();
  await expect(sheet.locator('[data-shortcut="new_tab"] kbd')).toHaveText(commandLabel("new_tab", "electron"));
  await expect(sheet.locator('[data-shortcut="close_tab"] kbd')).toHaveText(commandLabel("close_tab", "electron"));
  await expect(sheet.getByText("moved for Chrome")).toHaveCount(0);
  await screenshot(page, "desktop-shortcut-sheet");
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
});

// @platform: the native accelerator and menu bridge each open a terminal on every system.
test("New tab: the native chord and menu each open one terminal directly", { tag: "@platform" }, async () => {
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  const last = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, last);
  await shellShown(page);
  await enterWorkspace(page);
  const tabs = page.locator("[data-agent-tab-bar] [role=tab][data-tab]");
  const start = page.locator("[data-agent-tab-start]");
  await expect(page.locator("[data-pane-view]").first()).toBeVisible();
  const before = await tabs.count();
  const nativeTabCount = () => (herdrTabs(herdr) as { tabs: string[] }[]).reduce((sum, workspace) => sum + workspace.tabs.length, 0);
  const beforeNative = nativeTabCount();
  const area = page.locator("[data-agent-area-id]");
  const areaId = await area.getAttribute("data-agent-area-id");
  const request = {
    workspace: { device_id: expect.any(String), path: toPage(path.join(herdr.root, "fixture")) },
    group_id: null, action: "new_tab", terminal: true, request_id: expect.any(String),
  };

  // New tab opens a shell in the keyboard's Agent area without a start form.
  await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();
  await page.keyboard.press(chord("new_tab", "electron"));
  await expect(tabs).toHaveCount(before + 1);
  await expect(start).toHaveCount(0);
  const selected = area.locator('[data-agent-tab-bar] [role=tab][aria-selected="true"]');
  await expect(selected).not.toHaveAttribute("data-tab", /^draft:/);
  const first = await selected.getAttribute("data-tab");
  await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", first!);
  await expect.poll(() => sent.get("agent_group.new_tab")).toBe(1);
  expect(last.get("agent_group.new_tab")).toMatchObject({ ...request, area_id: areaId });
  const firstRequest = last.get("agent_group.new_tab")!.request_id;
  await expect.poll(nativeTabCount).toBe(beforeNative + 1);

  // The browser-only chord creates no tab; the native menu adds exactly one more terminal.
  await page.keyboard.press(chord("new_tab"));
  await app.evaluate(({ Menu }) => Menu.getApplicationMenu()?.getMenuItemById("new_tab")?.click());
  await expect(tabs).toHaveCount(before + 2);
  await expect(start).toHaveCount(0);
  await expect(selected).not.toHaveAttribute("data-tab", first!);
  await expect(selected).not.toHaveAttribute("data-tab", /^draft:/);
  const second = await selected.getAttribute("data-tab");
  await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", second!);
  await expect.poll(() => sent.get("agent_group.new_tab")).toBe(2);
  expect(last.get("agent_group.new_tab")).toMatchObject(request);
  expect(last.get("agent_group.new_tab")!.request_id).not.toBe(firstRequest);
  expect(sent.get("create_tab") ?? 0).toBe(0);
  await expect.poll(nativeTabCount).toBe(beforeNative + 2);
  await expect(page.locator("[data-pane-view]")).toBeVisible();
  await screenshot(page, "desktop-new-tab-terminals");
});

test("links: external links open in the default browser and the window stays on the daemon", async () => {
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  await shellShown(page);
  const origin = new URL(page.url()).origin;
  await app.evaluate(({ shell }) => {
    const opened: string[] = [];
    (globalThis as { opened?: string[] }).opened = opened;
    shell.openExternal = async (url: string) => {
      opened.push(url);
    };
  });
  // B10: a new window never opens; its URL goes to the default browser.
  expect(await page.evaluate(() => window.open("https://example.com/window") === null)).toBe(true);
  // A navigation off the daemon origin is refused and handed over the same way.
  await page.evaluate(() => {
    location.href = "https://example.com/navigate";
  });
  await expect.poll(() => app!.evaluate(() => (globalThis as { opened?: string[] }).opened)).toEqual([
    "https://example.com/window",
    "https://example.com/navigate",
  ]);
  // Playwright keeps waiting on the refused navigation, so the page is read directly.
  expect(await page.evaluate(() => [location.origin, !!document.querySelector("[data-main-screen], [data-workspace-screen]")])).toEqual([origin, true]);
});

// @platform: Closing the last window keeps the app on macOS and ends it elsewhere (`window-all-closed` in `index.ts`); hided outlives the app on every system.
test("lifetime: a second launch brings the first back, closing the last window keeps the app on macOS and ends it elsewhere, and hided stays for the next launch", { tag: "@platform" }, async () => {
  ({ app } = await launch(run.env));
  let page = await app.firstWindow();
  await shellShown(page);
  const pid = run.daemonPid();
  expect(pid).not.toBeNull();
  // Issue 232: the host heard the e2e switch; the fixture's focus guard fails the test if the app still came forward.
  expect(hostLog(run.env).find((line) => line.event === "host.start")?.show_inactive).toBe(true);

  // B6: a second launch on the same profile exits and the first comes forward
  // (under e2e, shown again without taking the keyboard; the first instance's switch decides that).
  const electronBinary = fs.readFileSync(path.join(DESKTOP_DIR, "node_modules", "electron", "path.txt"), "utf8");
  // Playwright starts the first instance without Chromium's sandbox, which a CI runner on Linux cannot provide; the second must match.
  const sandbox = process.platform === "linux" ? ["--no-sandbox"] : [];
  const second = spawn(path.join(DESKTOP_DIR, "node_modules", "electron", "dist", electronBinary), [...sandbox, DESKTOP_DIR], { env: run.env, stdio: "ignore" });
  const [code, signal] = await new Promise<[number | null, NodeJS.Signals | null]>((resolve) => second.once("exit", (exitCode, exitSignal) => resolve([exitCode, exitSignal])));
  expect(code, `the second launch ended by signal ${signal}`).toBe(0);
  await expect.poll(() => hostLog(run.env).some((line) => line.event === "host.reopen" && line.trigger === "second-instance")).toBe(true);
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);

  // B7: closing the last window keeps the app on macOS, and a Dock click (activate) brings a window back;
  // on every other system the app ends with its last window, and the next launch attaches to the same hided.
  const first = app.process();
  const closing = app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.close());
  if (process.platform === "darwin") {
    await closing;
    await expect.poll(() => app!.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(0);
    const reopened = app.waitForEvent("window");
    await app.evaluate(({ app: electronApp }) => electronApp.emit("activate"));
    page = await reopened;
    await shellShown(page);
  } else {
    // The app ends with its window, so the connection can drop before the answer returns.
    await closing.catch(() => undefined);
    if (first.exitCode === null && first.signalCode === null) await new Promise((resolve) => first.once("exit", resolve));
    app = await relaunch(run.env);
  }
  expect(run.daemonPid()).toBe(pid);

  // B8: the size and position come back on the next launch.
  // Fitted inside this machine's primary work area (a CI runner's display is
  // smaller than a workstation's), and the expectation is what the system actually
  // placed, since the window manager may still adjust a requested rect.
  const bounds = await app.evaluate(({ BrowserWindow, screen }) => {
    const area = screen.getPrimaryDisplay().workArea;
    const window = BrowserWindow.getAllWindows()[0]!;
    // eslint-disable-next-line hide-e2e/window-size-through-fixture -- the rect the next launch restores is this check's subject, set inside the work area.
    window.setBounds({
      x: area.x + 20,
      y: area.y + 20,
      width: Math.min(1000, area.width - 40),
      height: Math.min(700, area.height - 40),
    });
    return window.getNormalBounds();
  });

  // B5: quitting the app leaves the daemon; the next launch attaches to it.
  await app.close();
  app = null;
  expect(run.daemonPid()).toBe(pid);
  app = await relaunch(run.env);
  expect(run.daemonPid()).toBe(pid);
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.getBounds())).toEqual(bounds);
  // First launch, the reopened window (bounds written when the first closed), then the relaunch.
  expect(hostLog(run.env).filter((line) => line.event === "window.bounds").map((line) => line.source)).toEqual(["default", "stored", "stored"]);
});

// @platform: The CLI is `hide.exe` on Windows and `hide` elsewhere, and the host looks for it in the system's own places (`cli.ts`).
test("failure: a missing CLI shows its reason and Retry attaches once it exists", { tag: "@platform" }, async () => {
  const cli = path.join(run.root, "bin", fixtureExecutable("hide"));
  ({ app } = await launch({ ...run.env, HIDE_CLI_PATH: cli }));
  const page = await app.firstWindow();
  // B2, B3: one screen, the reason category, and Retry; the tried path is logged.
  await expect(page.locator("#reason")).toHaveText("The hide command was not found.", { timeout: 20_000 });
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await screenshot(page, "desktop-cli-missing");
  expect(hostLog(run.env).find((line) => line.event === "cli.missing")?.tried).toBe(cli);
  // PRD first-run-onboarding B2: Copy diagnostics puts the build, the reason and the log's place on the clipboard and says so,
  // or, when the clipboard refused, shows that text to select by hand.
  // The clipboard is the machine's, so what it held is put back.
  const held = await app.evaluate(({ clipboard }) => clipboard.readText());
  try {
    await page.getByRole("button", { name: "Copy diagnostics" }).click();
    await expect(page.getByRole("button", { name: "Copied" }).or(page.locator("#diagnostics", { hasText: "reason: cli_missing" }))).toBeVisible();
    const copied = hostLog(run.env).find((line) => line.event === "status.diagnostics_copied" || line.event === "status.diagnostics_copy_failed");
    expect(copied?.reason).toBe("cli_missing");
    if (copied?.event === "status.diagnostics_copied") expect(await app.evaluate(({ clipboard }) => clipboard.readText())).toMatch(/^hide .+\nreason: cli_missing\nlog: .+desktop\.log$/);
  } finally {
    await app.evaluate(({ clipboard }, text) => clipboard.writeText(text), held);
  }
  // The refused form and the connecting line (B1) are what the page draws from its address, read here without a second launch.
  await page.evaluate(() => (location.hash = "failed=cli_missing&diagnostics=reason%3A+cli_missing"));
  await expect(page.locator("#diagnostics")).toHaveText("reason: cli_missing");
  await expect(page.getByRole("button", { name: "Copy diagnostics" })).toBeVisible();
  await page.evaluate(() => (location.hash = "connecting"));
  await expect(page.getByRole("status")).toHaveText("Starting Hide…");
  await expect(page.locator("#failed")).toBeHidden();
  await page.evaluate(() => (location.hash = "failed=cli_missing"));
  fs.mkdirSync(path.dirname(cli), { recursive: true });
  if (process.platform === "win32") {
    // A link needs a privilege the account may lack, and `hide connect` starts the `hided` beside it, so both programs are copied.
    for (const name of ["hide", "hided"]) fs.copyFileSync(path.join(path.dirname(HIDE_CLI), fixtureExecutable(name)), path.join(path.dirname(cli), fixtureExecutable(name)));
  } else {
    fs.symlinkSync(HIDE_CLI, cli);
  }
  await page.getByRole("button", { name: "Retry" }).click();
  await shellShown(page);
});

test("failure: a state folder another node owns names the file and starts nothing", async () => {
  // PRD core-host-node B2: the daemon refuses the folder and the screen says which file.
  const marker = path.join(run.env.HIDE_STATE_DIR!, "node.json");
  fs.mkdirSync(path.dirname(marker), { recursive: true });
  fs.writeFileSync(marker, JSON.stringify({ version: 1, node: "another-node" }));
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  await expect(page.locator("#reason")).toHaveText("hided could not start because of this file, which was left unchanged.", { timeout: 20_000 });
  // The daemon names the file as it resolved its state folder, which may spell the temporary root differently.
  await expect(page.locator("#file")).toHaveText(/\/state\/node\.json$/);
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await screenshot(page, "desktop-state-refused");
  expect(run.daemonPid()).toBeNull();
  expect(JSON.parse(fs.readFileSync(marker, "utf8"))).toEqual({ version: 1, node: "another-node" });
});

test("failure: a core newer than this app names its machine and both builds and asks for the app to be updated", async () => {
  // PRD core-host-node-move B11: a stand-in hide answers as a node whose core runs a newer build; the core is left as it is.
  const cli = path.join(run.root, "bin", "hide");
  fs.mkdirSync(path.dirname(cli), { recursive: true });
  const answer = { ok: false, reason: "core_newer", detail: "newer", machine: "Mac mini", core: "0.4.2 (3f1c2a9)", app: "0.4.1 (b520d6e)" };
  fs.writeFileSync(cli, `#!/bin/sh\necho '${JSON.stringify(answer)}'\nexit 2\n`, { mode: 0o755 });
  ({ app } = await launch({ ...run.env, HIDE_CLI_PATH: cli }));
  const page = await app.firstWindow();
  await expect(page.locator("h1")).toHaveText("Update this app", { timeout: 20_000 });
  await expect(page.locator("#reason")).toHaveText(
    "The core on Mac mini runs a newer build than this app, so they did not connect. The core keeps running. Replace hide.app with the newer build, then open it again.",
  );
  await expect(page.locator("#builds")).toHaveText("core 0.4.2 (3f1c2a9) · this app 0.4.1 (b520d6e)");
  await expect(page.locator("#file")).toBeHidden();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await screenshot(page, "desktop-core-newer");
  expect(hostLog(run.env).find((line) => line.event === "discovery.failed")).toMatchObject({ reason: "core_newer", machine: "Mac mini", core: "0.4.2 (3f1c2a9)", app: "0.4.1 (b520d6e)" });
});

test("failure: an attached node whose core update failed shows both builds, and Retry is one more connection", async () => {
  // PRD core-host-node-move B10, B20: the node's health names the failed update and the host leaves the shell for its page.
  const builds = { machine: "Mac mini", core: "0.4.1 (b520d6e)", app: "0.4.2 (3f1c2a9)" };
  const node = http.createServer((request, response) => {
    if (request.url === "/health") {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ role: "node", core_link: "waiting", core_link_reason: "update_failed", builds }));
      return;
    }
    response.setHeader("content-type", "text/html");
    response.end("<!doctype html><title>shell</title>");
  });
  await new Promise<void>((resolve) => node.listen(0, "127.0.0.1", resolve));
  try {
    const port = (node.address() as AddressInfo).port;
    const cli = path.join(run.root, "bin", "hide");
    const runs = path.join(run.root, "connect-runs");
    fs.mkdirSync(path.dirname(cli), { recursive: true });
    const answer = { ok: true, url: `http://127.0.0.1:${port}/#token=t`, port, pid: process.pid };
    fs.writeFileSync(cli, `#!/bin/sh\necho run >> '${runs}'\necho '${JSON.stringify(answer)}'\n`, { mode: 0o755 });
    ({ app } = await launch({ ...run.env, HIDE_CLI_PATH: cli }));
    const page = await app.firstWindow();
    await expect(page.locator("h1")).toHaveText("The core was not updated", { timeout: 20_000 });
    await expect(page.locator("#reason")).toHaveText("The core on Mac mini is still running its previous build. Retry, or update Hide on Mac mini.");
    await expect(page.locator("#builds")).toHaveText("core 0.4.1 (b520d6e) · this app 0.4.2 (3f1c2a9)");
    await screenshot(page, "desktop-update-failed");
    const connects = () => fs.readFileSync(runs, "utf8").trim().split("\n").length;
    expect(connects()).toBe(1);
    await page.getByRole("button", { name: "Retry" }).click();
    // Retry is one connection, which this node answers with the same failure; nothing connects again on its own.
    await expect.poll(connects).toBe(2);
    await expect(page.locator("h1")).toHaveText("The core was not updated", { timeout: 20_000 });
    expect(hostLog(run.env).filter((line) => line.event === "discovery.failed" && line.reason === "update_failed")).toHaveLength(2);
    expect(connects()).toBe(2);
  } finally {
    node.close();
  }
});

test("discovery: a Finder-style PATH still lets a new daemon run installed tools", async () => {
  // launchd's bare PATH, no override, no worktree build beside it. Playwright runs the app
  // unpackaged, so the login-shell step is skipped here; cli.test.ts covers its place in the order.
  const bare = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
  const env: Record<string, string> = { ...run.env, PATH: bare.join(":") };
  delete env.HIDE_CLI_PATH;
  const appDir = detachedApp(run.root);
  const installed = path.join(env.HOME!, ".local", "bin", "hide");
  fs.mkdirSync(path.dirname(installed), { recursive: true });
  fs.symlinkSync(HIDE_CLI, installed);
  const gh = path.join(path.dirname(installed), "gh");
  fs.writeFileSync(gh, "#!/bin/sh\nprintf 'fixture-gh\\n'\n", { mode: 0o755 });
  // The unpackaged host still looks beside its app folder for a worktree build; there is none.
  const before = [...["debug", "release"].map((profile) => path.join(run.root, "target", profile, "hide")), ...bare.map((dir) => path.join(dir, "hide"))];
  const resolved = () => hostLog(env).filter((line) => line.event === "cli.resolved");

  ({ app } = await launch(env, { appDir }));
  await shellShown(await app.firstWindow());
  const daemon = run.daemonPid();
  expect(daemon).not.toBeNull();
  const processInfo = spawnSync("/bin/ps", ["eww", "-p", String(daemon), "-o", "command="], { encoding: "utf8" });
  expect(processInfo.status).toBe(0);
  const daemonPath = /(?:^|\s)PATH=(.*?)(?=\s[A-Za-z_][A-Za-z_0-9]*=|$)/s.exec(processInfo.stdout)?.[1];
  expect(daemonPath).toBeTruthy();
  expect(spawnSync("gh", ["--version"], { env: { PATH: daemonPath }, encoding: "utf8" }).stdout).toBe("fixture-gh\n");
  expect(resolved().at(-1)).toMatchObject({ source: "well-known", path: installed, tried: [...before, installed].join(":") });
  const remembered = path.join(env.HIDE_DESKTOP_USER_DATA_DIR!, "cli-path.json");
  expect(JSON.parse(fs.readFileSync(remembered, "utf8"))).toEqual({ schema: 1, path: installed });
  await app.close();

  app = await relaunch(env, { appDir });
  expect(resolved().at(-1)).toMatchObject({ source: "remembered", path: installed, tried: [...before, installed].join(":") });
});

// @platform: A daemon ending and the next one starting, seen through the system's processes and local stream.
test("reattach: a daemon that dies shows the shell's disconnected state, and the app follows the next one", { tag: "@platform" }, async () => {
  ({ app } = await launch(run.env));
  const page = await app.firstWindow();
  await shellShown(page);
  const first = new URL(page.url()).origin;
  // B4: the shell's own disconnected state; the host reloads nothing.
  run.hide(["stop"]);
  await expect(page.locator("[data-connection]")).toBeVisible({ timeout: 20_000 });
  expect(new URL(page.url()).origin).toBe(first);
  await screenshot(page, "desktop-daemon-lost");
  // Another client starts hided again, on a new port with a new token.
  expect(run.hide(["connect"]).status).toBe(0);
  await expect.poll(() => new URL(page.url()).origin, { timeout: 20_000 }).not.toBe(first);
  await shellShown(page);
  await expect(page.locator("[data-connection]")).toHaveCount(0);
});

test("uncaught: an exception and a rejection nothing in the host caught go to its log without a dialog, the app keeps running, and quit finishes", async () => {
  ({ app } = await launch(run.env));
  await shellShown(await app.firstWindow());
  run.allowHostUncaught("planted");
  // A message can quote the daemon URL's token or a page's address; neither reaches the log.
  const secret = "0123456789abcdef0123456789abcdef";
  await app.evaluate((_electron, token) => {
    setTimeout(() => { throw new Error(`planted exception token=${token} at https://example.test/inbox?id=1`); }, 0);
    void Promise.reject(new Error("planted rejection"));
  }, secret);
  const uncaught = () => hostLog(run.env).filter((line) => line.event === "host.uncaught");
  await expect.poll(() => uncaught().map((line) => line.kind).sort()).toEqual(["uncaughtException", "unhandledRejection"]);
  const exception = uncaught().find((line) => line.kind === "uncaughtException")!;
  expect(exception.message).toBe("planted exception token=[redacted] at https://[address]");
  expect(String(exception.stack)).toContain("planted exception token=[redacted]");
  expect(uncaught().find((line) => line.kind === "unhandledRejection")?.message).toBe("planted rejection");
  const logged = JSON.stringify(hostLog(run.env));
  expect(logged).not.toContain(secret);
  expect(logged).not.toContain("example.test");
  // As with Electron's default, the app keeps running after either.
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length)).toBe(1);
  // Electron's dialog would hold the quit until someone answered it (issue 675).
  const host = app.process();
  await app.close();
  app = null;
  expect(host.exitCode).toBe(0);
});
