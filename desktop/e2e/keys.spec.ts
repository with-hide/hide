// The desktop app's keys end to end: the operator's macOS pane chords, which
// hided brings across from the removed native app's state once, and the held-modifier
// cycles, whose commit waits for the modifier's release. A private engine, a
// private hided and Electron app per test.

import { expect, type ElectronApplication, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { focusTab, newTab, openProject, splitPane, withControl } from "../../web/e2e/daemon-actions";
import { startEngine } from "../../web/e2e/engine-fixture";
import { startAgentIn } from "../../web/e2e/fixture-agents";
import "../../web/src/host";
import { countSent, enterWorkspace, showProjectRow } from "../../web/e2e/wire";
import { connectHome, hostLog, isolate, launchShell, screenshot, startStack, test, type Isolated } from "./fixture";
import { quietFor } from "../../web/e2e/wait";

let run: Isolated;
let stopHome: (() => void) | null = null;
let app: ElectronApplication | null = null;

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus && stopHome) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  stopHome?.();
  stopHome = null;
});

/** A menu item's accelerator as the app menu holds it now. */
function accelerator(id: string): Promise<string | null> {
  return app!.evaluate(({ Menu }, itemId) => Menu.getApplicationMenu()?.getMenuItemById(itemId)?.accelerator ?? null, id);
}

/** After the one expected event, a quiet moment in which no second one arrives. */
async function exactlyOnce(sent: Map<string, number>, kind: string, expected: number, page: Page): Promise<void> {
  await expect.poll(() => sent.get(kind) ?? 0).toBe(expected);
  await quietFor(page, 600, "no second event follows the one counted");
  expect(sent.get(kind) ?? 0).toBe(expected);
}

test("pane chords: the macOS app's set comes across, runs once, and Settings edits it", async () => {
  // The operator's native-app state on the day of the decision, with the fields
  // that app kept around the one hided reads.
  // hided reads that state when its core starts, so the home's daemon starts after it is written.
  const engine = await startEngine();
  run = isolate(engine, test.info().title.split(":")[0]!);
  stopHome = () => { try { run.cleanup(); } finally { engine.stop(); } };
  const nativeState = path.join(run.env.HOME!, "Library", "Application Support", "hide", "state.json");
  fs.mkdirSync(path.dirname(nativeState), { recursive: true });
  fs.writeFileSync(
    nativeState,
    JSON.stringify({
      schema_version: 1,
      expanded_paths: [],
      selected_path: null,
      selected_pane_id: null,
      pet_visible: false,
      shortcut_bindings: {
        close_pane: "command+shift+w",
        split_down: "command+shift+d",
        split_right: "command+d",
        toggle_zoom: "command+option+return",
        increase_text_size: "command+=",
        decrease_text_size: "command+-",
        reset_text_size: "command+0",
      },
    }),
  );
  const daemon = await connectHome(run);
  const fixtureDir = path.join(engine.root, "fixture");
  fs.mkdirSync(fixtureDir, { recursive: true });
  await withControl(daemon, async (control) => {
    const made = await openProject(control, fixtureDir, "fixture");
    await splitPane(control, made.tab, made.folder);
  });
  const launched = await launchShell(run.env);
  app = launched.app;
  const page = launched.page;
  const lastSent = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, lastSent);
  await page.reload();
  await enterWorkspace(page);
  await expect(page.locator("[data-pane-view]")).toHaveCount(2);

  // The menu shows the imported chord once the shell reports the set.
  await expect.poll(() => accelerator("toggle_zoom")).toBe("Alt+Command+Return");
  expect(await accelerator("split_right")).toBe("Command+D");

  // ⌥⌘↩ zooms exactly once; the default ⇧⌘↩ is no longer a chord here.
  await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();
  await page.keyboard.press("Meta+Shift+Enter");
  await page.keyboard.press("Meta+Alt+Enter");
  await expect(page.locator("[data-canvas]").first()).toHaveAttribute("data-zoomed", "true");
  await exactlyOnce(sent, "toggle_zoom", 1, page);

  // The ⌘/ sheet lists it.
  await page.keyboard.press("Meta+Slash");
  const sheet = page.locator("[data-shortcut-sheet]");
  await expect(sheet.locator('[data-shortcut="toggle_zoom"] kbd')).toHaveText("⌥⌘↩");
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);

  // Settings > Shortcuts edits the same set, in the macOS set's text form.
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="shortcuts"]').click();
  await expect(page.locator('[data-shortcut-effective="toggle_zoom"]')).toHaveText("⌥⌘↩");
  // The numbered chords are fixed, and each row says which key to hold to see its numbers.
  const fixed = page.locator('[data-settings-group="numbered-chords"]');
  await expect(fixed).toContainText("Hold ⌘ to see the numbers");
  await expect(fixed).toContainText("Hold ⌥ to see the numbers");
  await expect(fixed).toContainText("Hold ⌥⌘ to see the numbers");
  await page.locator('[data-shortcut-record="split_right"]').click();
  await page.keyboard.press("Meta+Alt+KeyR");
  await expect(page.locator('[data-shortcut-effective="split_right"]')).toHaveText("⌥⌘R");
  const saved = lastSent.get("ui_state_update")?.shortcut_bindings as Record<string, string> | undefined;
  expect(saved).toMatchObject({ split_right: "command+option+r", toggle_zoom: "command+option+return" });
  expect(lastSent.get("ui_state_update")?.browser_shortcut_bindings ?? {}).toEqual({});
  await expect.poll(() => accelerator("split_right")).toBe("Alt+Command+R");
  await screenshot(page, "desktop-shortcut-settings");
});

test("cycles: ⌃Tab and ⌥Tab commit once, on releasing the held modifier", async () => {
  // Two more tabs in the fixture Workspace, each running an agent, since the
  // Agent area's ⌃Tab walks agent panes, and two more Projects.
  const stack = await startStack("cycles");
  const { fixture } = stack;
  run = stack.run;
  stopHome = stack.stop;
  const tabs = [fixture.tab];
  for (const label of ["second", "third"]) {
    const made = await withControl(stack.daemon, (control) => newTab(control, fixture, label));
    tabs.push(made.tab);
    // Each of those tabs runs an agent, started as the operator starts one in a shell.
    await startAgentIn(fixture, stack.daemon, made.pane);
  }
  // Two more Projects, and the first tab shown again, as it was before they were made.
  await withControl(stack.daemon, async (control) => {
    for (const name of ["beta", "gamma"]) {
      fs.mkdirSync(path.join(fixture.root, name), { recursive: true });
      await openProject(control, path.join(fixture.root, name), name);
    }
    await focusTab(control, fixture, fixture.tab);
  });
  const launched = await launchShell(run.env);
  app = launched.app;
  const page = launched.page;
  const sent = countSent(page);
  await page.reload();
  await enterWorkspace(page, "fixture");
  const canvas = page.locator("[data-canvas]").first();
  const cycleRow = (kind: string) => page.locator(`[data-cycle=${kind}] [aria-selected=true]`);

  // Recent order third, second, first: the first tab's focused pane is current.
  const paneOf = new Map<string, string>();
  for (const tab of [tabs[2]!, tabs[1]!, tabs[0]!]) {
    await page.locator(`[data-tab="${tab}"]`).click();
    await expect(canvas).toHaveAttribute("data-canvas", tab);
    paneOf.set(tab, (await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"))!);
  }

  // ⌃Tab twice with ⌃ held walks two Agent panes back; releasing ⌃ commits one focus_pane.
  let focused = sent.get("focus_pane") ?? 0;
  await page.keyboard.down("Control");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await expect(cycleRow("agents")).toHaveAttribute("data-cycle-row", paneOf.get(tabs[2]!)!);
  expect(sent.get("focus_pane") ?? 0).toBe(focused);
  await page.keyboard.up("Control");
  await expect(page.locator("[data-cycle]")).toHaveCount(0);
  await expect(canvas).toHaveAttribute("data-canvas", tabs[2]!);
  await exactlyOnce(sent, "focus_pane", focused + 1, page);

  // ⌃⇧Tab walks back toward the start: from third, two forward then one
  // back is first.
  focused = sent.get("focus_pane") ?? 0;
  await page.keyboard.down("Control");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(cycleRow("agents")).toHaveAttribute("data-cycle-row", paneOf.get(tabs[0]!)!);
  await page.keyboard.up("Control");
  await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);
  await exactlyOnce(sent, "focus_pane", focused + 1, page);

  // Sidebar focus also opens recent Agent panes. It has no pane origin,
  // so the first chord selects the most recent pane, then walks the list.
  const sidebarOverview = page.locator('[data-sidebar-overview]');
  await sidebarOverview.focus();
  await expect(sidebarOverview).toBeFocused();
  focused = sent.get("focus_pane") ?? 0;
  await page.keyboard.down("Control");
  await page.keyboard.press("Tab");
  await expect(cycleRow("agents")).toHaveAttribute("data-cycle-row", paneOf.get(tabs[0]!)!);
  await page.keyboard.press("Tab");
  await expect(cycleRow("agents")).toHaveAttribute("data-cycle-row", paneOf.get(tabs[2]!)!);
  await screenshot(page, "desktop-recent-agents-from-sidebar");
  const captureDir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (captureDir) {
    const candidate = await app!.evaluate(({ BrowserWindow }) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error("expected one isolated candidate window");
      return { pid: process.pid, windowId: windows[0]!.getMediaSourceId().split(":")[1]! };
    });
    fs.writeFileSync(path.join(captureDir, "desktop-recent-agents-candidate.json"), JSON.stringify(candidate));
    execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", candidate.windowId, path.join(captureDir, "desktop-recent-agents-from-sidebar-native.png")]);
  }
  expect(sent.get("focus_pane") ?? 0).toBe(focused);
  await page.keyboard.up("Control");
  await expect(canvas).toHaveAttribute("data-canvas", tabs[2]!);
  await exactlyOnce(sent, "focus_pane", focused + 1, page);

  // Recent Projects gamma, beta, fixture: fixture is current.
  const checkout = (name: string) => page.locator("[data-project]", { hasText: name }).locator("[data-checkout]").first();
  for (const name of ["gamma", "beta", "fixture"]) {
    await (await showProjectRow(page, checkout(name))).click({ position: { x: 12, y: 8 } });
    await expect(checkout(name)).toHaveAttribute("aria-current", "true");
  }

  // ⌥Tab twice walks to gamma; releasing ⌥ commits one focus_tab, back on
  // the tab gamma was last used on.
  focused = sent.get("focus_tab") ?? 0;
  await page.keyboard.down("Alt");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await expect(cycleRow("projects")).toBeVisible();
  expect(sent.get("focus_tab") ?? 0).toBe(focused);
  await page.keyboard.up("Alt");
  await expect(page.locator("[data-cycle]")).toHaveCount(0);
  await expect(checkout("gamma")).toHaveAttribute("aria-current", "true");
  await exactlyOnce(sent, "focus_tab", focused + 1, page);

  // ⌥⇧Tab walks backward: from gamma, one back past the end is beta.
  focused = sent.get("focus_tab") ?? 0;
  await page.keyboard.down("Alt");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.up("Alt");
  await expect(checkout("beta")).toHaveAttribute("aria-current", "true");
  await exactlyOnce(sent, "focus_tab", focused + 1, page);
});
