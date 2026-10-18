// S2 flows on an isolated pinned Herdr (PRD web-shell-pivot-s2 B16): two
// checkouts, three tabs, two splits, one registration, one refusal, plus
// the shortcut sheet, zoom, a pane close and a divider drag. Every command
// runs against a private server; the operator's Herdr is never touched.
// Each test is one contract and starts from `startFlow`'s own stack, so a
// failure in one never takes another contract's assertions down with it.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import crypto from "node:crypto";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { altScreenProgram, bytesReceived } from "./alt-screen-program";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { projectRow } from "./overview-entry";
import { FACTORY_OPEN } from "../src/factory/launch";
import { countSent, screenshot, showProjectRow } from "./wire";
import { chord, mod, SYSTEM } from "./chords";

/**
 * Waits until the shell has drawn `paneId` as the focused pane and kept it
 * for a second. Herdr confirms a focus later than the core moves it, and
 * on a slow runner the confirmation of one request can reach the core after
 * the next request went out, naming the earlier pane for a moment; keys
 * typed in that moment follow it.
 */
async function settledFocus(page: Page, paneId: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  let held = 0;
  while (held < 5) {
    if (Date.now() > deadline) throw new Error(`focus did not settle on ${paneId}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
    const focused = await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view");
    held = focused === paneId ? held + 1 : 0;
  }
}

/**
 * Copies what the focused pane has selected with the system's terminal copy
 * chord: ⌘C on macOS, the browser's own copy, and Ctrl+Shift+C on Windows and
 * Linux, which the pane answers itself. Both raise the same `copy` event the
 * pane's handler answers.
 */
async function copy(page: Page): Promise<void> {
  await page.keyboard.press(mod("KeyC"));
}

async function screen(page: Page): Promise<string> {
  return page.evaluate(() => window.__hideProbe?.screenText() ?? "");
}

/** The pane Herdr itself reports as focused. */
function herdrFocused(herdr: HerdrFixture): string {
  return (herdr.run(["pane", "current"]) as { result: { pane: { pane_id: string } } }).result.pane.pane_id;
}

test.describe.configure({ timeout: 90_000 });

type Flow = {
  herdr: HerdrFixture;
  beta: { result: { workspace: { workspace_id: string }; tab: { tab_id: string } } };
  /** The first checkout's three tabs, the fixture's own first tab first. */
  tabs: string[];
  /** The first checkout's row in the Projects sidebar. */
  firstRow: ReturnType<Page["locator"]>;
  sent: Map<string, number>;
  lastSent: Map<string, Record<string, unknown>>;
  /** Every `terminal.panes` list the daemon sent, oldest first, each pane id with its transport state. */
  terminalLists: Map<string, string>[];
  stop: () => void;
};

/**
 * Records each `terminal.panes` list the page receives. The core sends the
 * whole list whenever it changes, and the shell disposes a parked terminal
 * whose pane is missing from it, so one list without a retained pane is a
 * renderer thrown away even when the next list brings the pane back.
 */
function recordTerminalLists(page: Page): Map<string, string>[] {
  const lists: Map<string, string>[] = [];
  page.on("websocket", (ws) => {
    ws.on("framereceived", (frame) => {
      const text = String(frame.payload);
      if (!text.includes('"terminal":{')) return;
      const panes = (JSON.parse(text) as { payload?: { rest?: { terminal?: { panes?: { pane_id: string; transport_state: string }[] } } } }).payload?.rest?.terminal?.panes;
      if (panes) lists.push(new Map(panes.map((pane) => [pane.pane_id, pane.transport_state])));
    });
  });
  return lists;
}

/** The lists received since `from` that lack one of `panes` or carry it released. */
function listsDropping(lists: Map<string, string>[], from: number, panes: string[]): string[] {
  return lists.slice(from).flatMap((list, index) => {
    const dropped = panes.filter((pane) => !list.has(pane) || list.get(pane) === "released");
    return dropped.length ? [`list ${from + index}: ${dropped.join(",")}`] : [];
  });
}

const attachedPanes = (page: Page) => page.evaluate(() => window.__hideProbe!.attachedPanes().sort());
const liveTerminals = (page: Page) => page.evaluate(() => window.__hideProbe!.liveTerminals().sort());
const paneText = (page: Page, paneId: string) => page.evaluate((id) => window.__hideProbe!.paneText(id), paneId);

/**
 * A second checkout and two more tabs in the first, all without focus, then
 * the page opened on the first checkout: three tabs, its first tab split
 * into two Claude agent panes. A Workspace opens with the Explorer beside its
 * agents; the room keeps the split panes wide enough that typed lines do not wrap.
 * `tabLabels` names the tabs added to the first; without `ownerMark` its
 * workspace is not its owner, so the first tab Hide creates there opens one.
 */
async function startFlow(page: Page, { tabLabels = ["second", "third"], ownerMark = true } = {}): Promise<Flow> {
  await page.setViewportSize({ width: 1680, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  const stop = () => {
    daemon?.stop();
    herdr.stop();
  };
  try {
    fs.mkdirSync(path.join(herdr.root, "beta"), { recursive: true });
    const beta = herdr.run([
      "workspace", "create", "--cwd", path.join(herdr.root, "beta"), "--label", "beta", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
    ]) as Flow["beta"];
    const tabs = [herdr.tab];
    for (const label of tabLabels) {
      const made = herdr.run([
        "tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", label, "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
      ]) as { result: { tab: { tab_id: string } } };
      tabs.push(made.result.tab.tab_id);
    }
    // Checkout owner D-11/B13: this existing plain-folder workspace owns
    // the tabs this flow adds. An unmarked workspace would correctly make
    // Hide open a new owner instead (covered by checkout-owner.spec.ts).
    // Owner identity uses the core's wire path, including on Windows.
    const folder = toPage(fs.realpathSync(path.join(herdr.root, "fixture")));
    const owner = crypto.createHash("sha256").update(`local\0${folder}`).digest("hex").slice(0, 32);
    if (ownerMark) execFileSync(herdr.bin, ["workspace", "report-metadata", herdr.workspace, "--source", "e2e-owner", "--token", `hide_owner=${owner}`], { env: herdr.env, timeout: 30_000 });
    daemon = await startHided(herdr);
    const lastSent = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, lastSent);
    const terminalLists = recordTerminalLists(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);

    // Two checkouts in the Projects sidebar. Which one the core focuses at
    // boot is its own choice, so the flow starts by choosing the first.
    // beta runs no agent, so unless it is in front it waits under No agents.
    await showProjectRow(page, page.locator("[data-project]", { hasText: "beta" }).locator("[data-checkout]").first());
    await expect(page.locator("[data-checkout]")).toHaveCount(2);
    // The core names projects by directory, not by Herdr's label or id.
    const firstRow = page.locator("[data-project]", { hasText: "fixture" }).locator("[data-checkout]").first();
    await firstRow.click();
    await expect(firstRow).toHaveAttribute("aria-current", "true");
    await expect(page.locator("[data-tab]")).toHaveCount(tabs.length);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    return { herdr, beta, tabs, firstRow, sent, lastSent, terminalLists, stop };
  } catch (error) {
    stop();
    throw error;
  }
}

/**
 * ⌘D on the first tab's split: a third pane running the fixture shell, which
 * the core focuses. Returns once the shell has its prompt, the keyboard, and Herdr's confirmation.
 */
async function splitShell(page: Page, flow: Flow): Promise<string> {
  await expect.poll(() => screen(page), { timeout: 15_000 }).toContain("claude");
  const focusedPane = () => page.evaluate(() => window.__hideProbe?.paneId() ?? null);
  const paneIds = () => page.locator("[data-pane-view]").evaluateAll((views) => views.map((view) => view.getAttribute("data-pane-view")!));
  const before = await paneIds();
  await page.keyboard.press(chord("split_right"));
  await expect(page.locator("[data-pane-view]")).toHaveCount(3);
  await expect.poll(() => flow.sent.get("create_pane")).toBe(1);
  // The split's new pane is the one the grid gained.
  const created = (await paneIds()).filter((id) => !before.includes(id));
  expect(created).toHaveLength(1);
  const shellPaneId = created[0];
  // The core publishes that pane's layout first and follows Herdr's focus onto
  // it in the update after, so wait for exactly that pane to hold the focus.
  await expect.poll(focusedPane).toBe(shellPaneId);
  const shellPane = page.locator(`[data-pane-view="${shellPaneId}"]`);
  await expect(shellPane).toHaveAttribute("data-transport", /connected|controlling|idle/, { timeout: 15_000 });
  await expect.poll(() => screen(page), { timeout: 15_000 }).toContain("fixture %");
  // Herdr confirms a focus later than the core moves it; the next focus_pane
  // must not go out before Herdr has named the shell.
  await expect.poll(() => herdrFocused(flow.herdr), { timeout: 10_000 }).toBe(shellPaneId);
  await shellPane.locator(".xterm-helper-textarea").focus();
  return shellPaneId;
}

// @platform: The checkout rows and tabs a platform's Herdr creates, owned by the core's wire path (a Windows drive path included), and the Overview round trip.
test("switching checkouts is one focus_checkout, and a plain folder is a row of the Overview over its Workspace", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, beta, firstRow, sent } = flow;
    const betaProject = page.locator("[data-project]", { hasText: "beta" });
    await screenshot(page, "s2-projects-two-checkouts");
    const focusEvents = sent.get("focus_checkout") ?? 0;

    // Switching checkouts is one focus_checkout; the tab bar and center follow.
    const betaRow = await showProjectRow(page, betaProject.locator("[data-checkout]").first());
    await betaRow.click();
    await expect(betaRow).toHaveAttribute("aria-current", "true");
    await expect(page.locator("[data-tab]")).toHaveCount(1);
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", beta.result.tab.tab_id);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    await expect.poll(() => sent.get("focus_checkout")).toBe(focusEvents + 1);

    // A plain folder's sidebar row opens its checkout; the Overview over it
    // lists the folder on Projects (PRD overview-device-tasks B47), and
    // Escape returns to the Workspace under it.
    await firstRow.click();
    await expect(await projectRow(page, "fixture")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", herdr.tab);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
  } finally {
    flow.stop();
  }
});

// @platform: ⌥T and ⌥` are chords the platform's browser keeps or moves, the new tab's label is the process name Herdr reads on that platform, and a tab closes through its Herdr.
test("a tab switch mounts the new tab, ⌥T is one create_tab, ⌥` walks the recent agent panes, and the strip reorders locally and closes a tab", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, tabs, sent } = flow;
    // The fixture's first tab runs two Claude agents; the one focused is the agent pane the keyboard was last in.
    const lastAgentPane = (await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"))!;

    // Tab switch: the previous tab's instances are gone, the new one's mounted.
    await page.locator(`[data-tab="${tabs[1]}"]`).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tabs[1]);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const secondTabShell = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    await expect.poll(() => sent.get("agent_layout.focus")).toBe(1);
    await expect.poll(() => screen(page), { timeout: 15_000 }).toContain("fixture %");

    // ⌥T is one create_tab; the new tab is active with the core's next label.
    const nextLabel = (await page.locator("[data-new-agent-tab]").first().getAttribute("aria-label"))!.replace("New tab ", "");
    await page.keyboard.press(chord("new_tab"));
    await expect(page.locator("[data-tab]")).toHaveCount(4);
    // The fourth tab belongs to the marked owner, not a second workspace.
    await expect.poll(() => (herdr.run(["tab", "list", "--workspace", herdr.workspace]) as { result: { tabs: unknown[] } }).result.tabs.length).toBe(4);
    // The label is Herdr's: the strip shows an automatic label only until
    // Herdr reports the pane's process, which then names the tab.
    const labelled = () => (herdr.run(["api", "snapshot"]) as { result: { snapshot: { tabs: { tab_id: string; label?: string }[] } } }).result.snapshot.tabs.find((tab) => tab.label === nextLabel)?.tab_id;
    await expect.poll(labelled).toBeTruthy();
    await expect(page.locator("[data-tab][aria-selected=true]")).toHaveAttribute("data-tab", labelled()!);
    await expect.poll(() => sent.get("create_tab")).toBe(1);

    // ⌥` in the Agent area walks the recent agent panes. The new tab and the
    // second one run plain shells, so the first candidate is the agent pane
    // left in the first tab.
    const paneFocusEvents = sent.get("focus_pane") ?? 0;
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    await expect(page.locator("[data-cycle=agents] [aria-selected=true]")).toHaveAttribute("data-cycle-row", lastAgentPane);
    await expect(page.locator(`[data-cycle=agents] [data-cycle-row="${secondTabShell}"]`)).toHaveCount(0);
    await page.keyboard.up("Alt");
    await expect(page.locator("[data-cycle]")).toHaveCount(0);
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", herdr.tab);
    await expect.poll(() => sent.get("focus_pane")).toBe(paneFocusEvents + 1);

    // Agent areas own local order (B7/D19): one move changes the strip without a Herdr reorder.
    const secondTab = page.locator(`[data-tab="${tabs[1]}"]`);
    const firstTab = page.locator(`[data-tab="${herdr.tab}"]`);
    const from = (await secondTab.boundingBox())!;
    const to = (await firstTab.boundingBox())!;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 - 30, from.y + from.height / 2, { steps: 4 });
    // Stay inside the tab, clear of the sidebar's centered resize grab.
    await page.mouse.move(to.x + to.width / 4, to.y + to.height / 2, { steps: 8 });
    await expect(page.locator('[data-agent-drop="bar"]')).toBeVisible();
    await page.mouse.up();
    await expect.poll(() => sent.get("agent_layout.move")).toBe(1);
    expect(sent.get("reorder_tab") ?? 0).toBe(0);
    await expect.poll(() => page.locator("[data-tab]").first().getAttribute("data-tab"), { timeout: 10_000 }).toBe(tabs[1]);

    // The tab close control closes the visible tab (idle panes need no confirmation).
    await page.locator(`[data-tab="${tabs[2]}"]`).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tabs[2]);
    await page.locator(`[data-tab="${tabs[2]}"] button`).click();
    await expect.poll(() => sent.get("close_tab")).toBe(1);
    await expect(page.locator("[data-tab]")).toHaveCount(3);
    await expect(page.locator(`[data-tab="${tabs[2]}"]`)).toHaveCount(0);
    await page.locator(`[data-tab="${herdr.tab}"]`).click();
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
  } finally {
    flow.stop();
  }
});

// @platform: A split, a divider resize and a pane close through the platform's Herdr and the shell it runs.
test("⌘D splits the focused pane, a divider drag sends one resize_pane, and ⌥⇧W closes the focused idle pane without a confirmation", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { sent } = flow;
    await splitShell(page, flow);
    await expect(page.locator("[data-split]")).toHaveCount(2);
    await screenshot(page, "s2-two-splits");

    // A divider drag sends one resize_pane on release and the grid follows Herdr.
    const outer = page.locator("[data-split=right]").first();
    const divider = outer.locator("> [data-divider]");
    const box = (await divider.boundingBox())!;
    const before = await outer.evaluate((el) => (el as HTMLElement).style.gridTemplateColumns);
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2, { steps: 6 });
    await expect(page.locator("[data-resize-guide]")).toHaveCount(1);
    await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2, { steps: 6 });
    await page.mouse.up();
    await expect(page.locator("[data-resize-guide]")).toHaveCount(0);
    await expect.poll(() => sent.get("resize_pane")).toBe(1);
    await expect
      .poll(() => outer.evaluate((el) => (el as HTMLElement).style.gridTemplateColumns), { timeout: 10_000 })
      .not.toBe(before);


    // ⌥⇧W closes the focused idle pane; Herdr's new geometry redraws the grid,
    // and the closed pane's terminal is disposed, not parked (D-05).
    const closingPane = (await page.evaluate(() => window.__hideProbe?.paneId()))!;
    await page.keyboard.press(chord("close_pane"));
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await expect.poll(() => sent.get("close_pane")).toBe(1);
    await expect(page.locator("[data-confirm-close]")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.liveTerminals() ?? [])).not.toContain(closingPane);
  } finally {
    flow.stop();
  }
});

// @platform: Wheel and click input over a real PTY through the platform's Herdr and its shell: the bytes its shim reads, and the SGR report is the only thing it gets.
test("a wheel over a shell pane is one terminal_scroll per batch, and a click is one terminal_click answered by the SGR report", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, sent, lastSent } = flow;
    // The new pane runs the fixture shell. A wheel over it is one
    // terminal_scroll per batch and the core's viewport follows (B19).
    const shellPane = page.locator('[data-pane-view][data-focused="true"]');
    const shellPaneId = await splitShell(page, flow);
    await page.keyboard.type("seq 1 100\n");
    await expect.poll(() => screen(page), { timeout: 15_000 }).toMatch(/100\s+fixture %/);
    const shellBox = (await shellPane.boundingBox())!;
    await page.mouse.move(shellBox.x + shellBox.width / 2, shellBox.y + shellBox.height / 2);
    const keysBeforeWheel = sent.get("key") ?? 0;
    await page.mouse.wheel(0, -120);
    await expect.poll(() => sent.get("terminal_scroll")).toBe(1);
    expect(lastSent.get("terminal_scroll")).toMatchObject({ direction: "up", modifiers: 0 });
    expect(lastSent.get("terminal_scroll")!.lines as number).toBeGreaterThan(1);
    await expect.poll(() => screen(page), { timeout: 10_000 }).not.toMatch(/100\s+fixture %/);
    await page.mouse.wheel(0, 400);
    await expect.poll(() => sent.get("terminal_scroll")).toBe(2);
    expect(lastSent.get("terminal_scroll")).toMatchObject({ direction: "down" });
    await expect.poll(() => screen(page), { timeout: 10_000 }).toMatch(/100\s+fixture %/);
    // ⌥ + wheel is the browser's, not Herdr's.
    await page.keyboard.down("Alt");
    await page.mouse.wheel(0, -120);
    await page.keyboard.up("Alt");
    await expect.poll(() => sent.get("terminal_scroll")).toBe(2);
    // The wheel never became key bytes: xterm did not get to turn it into
    // cursor keys (which would walk the shell's history).
    expect(sent.get("key") ?? 0).toBe(keysBeforeWheel);

    // A single click in a pane is one terminal_click with the pressed cell
    // (B20). The core answers it with the SGR report a mouse-tracking
    // program asked for, and that report is the only thing the PTY gets:
    // the shim logs every byte it reads, and no key event went out, so
    // xterm wrote no mouse report of its own.
    const agentPane = herdr.panes[1];
    // The cell comes from xterm's screen element, exactly cols by rows; the
    // host around it is larger by the fit's remainder, and a cell read from
    // the host drifts toward the far edge (the delta is printed so a run
    // shows it: the last row of a 41-row pane lands a row off).
    const agentHost = (await page.locator(`[data-pane-view="${agentPane}"] [data-terminal]`).boundingBox())!;
    const agentScreen = (await page.locator(`[data-pane-view="${agentPane}"] .xterm-screen`).boundingBox())!;
    const agentGrid = (await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, agentPane))!;
    const lastRowFromHost = Math.floor((agentScreen.y + (agentGrid.rows - 0.5) * (agentScreen.height / agentGrid.rows) - agentHost.y) / (agentHost.height / agentGrid.rows));
    console.log(
      `cell grid: host ${agentHost.width}x${agentHost.height}, screen ${agentScreen.width}x${agentScreen.height}, ${agentGrid.cols}x${agentGrid.rows}; ` +
        `the last row's centre read from the host is row ${lastRowFromHost}`,
    );
    const cell = { column: 4, row: 2 };
    const focusBefore = sent.get("focus_pane") ?? 0;
    const keysBeforeClick = sent.get("key") ?? 0;
    const inputBefore = fs.existsSync(herdr.inputLogs[1]) ? fs.statSync(herdr.inputLogs[1]).size : 0;
    await page.mouse.click(
      agentScreen.x + (cell.column + 0.5) * (agentScreen.width / agentGrid.cols),
      agentScreen.y + (cell.row + 0.5) * (agentScreen.height / agentGrid.rows),
    );
    await expect.poll(() => sent.get("terminal_click")).toBe(1);
    expect(lastSent.get("terminal_click")).toEqual({ pane_id: agentPane, column: cell.column, row: cell.row, modifiers: 0 });
    await expect.poll(() => sent.get("focus_pane")).toBe(focusBefore + 1);
    // Herdr confirms a focus later than the core moves it, and a confirmation
    // still in flight when the next focus_pane goes out is followed as a
    // move of Herdr's own (ARCHITECTURE.md), which would flip keyboard focus
    // back mid-typing on a slow runner; each focus waits for Herdr's word.
    await expect.poll(() => herdrFocused(herdr), { timeout: 10_000 }).toBe(agentPane);
    await settledFocus(page, agentPane);
    const report = `\x1b[<0;${cell.column + 1};${cell.row + 1}M\x1b[<0;${cell.column + 1};${cell.row + 1}m`;
    await expect
      .poll(() => (fs.existsSync(herdr.inputLogs[1]) ? fs.readFileSync(herdr.inputLogs[1], "latin1").slice(inputBefore) : ""), { timeout: 10_000 })
      .toBe(report);
    expect(sent.get("key") ?? 0).toBe(keysBeforeClick);

    // Two operator focus changes are two focus_pane events; the focus the
    // shell moves to follow the snapshot is never reported back.
    const shellView = page.locator(`[data-pane-view="${shellPaneId}"]`);
    await shellView.locator(".xterm-helper-textarea").focus();
    await expect(shellView).toHaveAttribute("data-focused", "true");
    await expect.poll(() => herdrFocused(herdr), { timeout: 10_000 }).toBe(shellPaneId);
    await settledFocus(page, shellPaneId);
    expect(sent.get("focus_pane")).toBe(focusBefore + 2);
  } finally {
    flow.stop();
  }
});

// @platform: A full-screen program on the platform's PTY, which answers a wheel with the alternate screen's own keys.
test("a wheel over an alternate-screen program is one terminal_scroll and no key event, and q reaches it", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, sent, lastSent } = flow;
    const shellPane = page.locator('[data-pane-view][data-focused="true"]');
    await splitShell(page, flow);
    // The pager fixture draws the alternate screen and logs every byte the
    // PTY hands it, which is what says whether a wheel or a key reached it.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-alt-"));
    herdr.afterStop(() => fs.rmSync(dir, { recursive: true, force: true }));
    const log = path.join(dir, "bytes.log");
    await page.keyboard.type(`${altScreenProgram(dir, log).command}\n`);
    await expect.poll(() => screen(page), { timeout: 15_000 }).toMatch(/^1\s/);
    expect(bytesReceived(log).length).toBe(0);
    const shellBox = (await shellPane.boundingBox())!;
    await page.mouse.move(shellBox.x + shellBox.width / 2, shellBox.y + shellBox.height / 2);
    const keysBeforeWheel = sent.get("key") ?? 0;
    await page.mouse.wheel(0, 240);
    await expect.poll(() => sent.get("terminal_scroll")).toBe(1);
    expect(lastSent.get("terminal_scroll")).toMatchObject({ direction: "down" });
    await page.mouse.wheel(0, -240);
    await expect.poll(() => sent.get("terminal_scroll")).toBe(2);
    expect(sent.get("key") ?? 0).toBe(keysBeforeWheel);
    await page.keyboard.press("q");
    await expect.poll(() => sent.get("key")).toBe(keysBeforeWheel + 1);
    // Herdr answers a wheel batch over an alternate screen without mouse
    // tracking with one cursor key (down, then up), whatever the batch's
    // lines; q is the one key event the page sent. These are the only bytes
    // the PTY got, in this order.
    await expect.poll(() => bytesReceived(log).toString("latin1"), { timeout: 15_000 }).toBe("\x1b[B\x1b[Aq");
    // The program left the alternate screen on q and the shell's prompt is back.
    await expect.poll(() => screen(page), { timeout: 15_000 }).toMatch(/fixture %\s*$/);
  } finally {
    flow.stop();
  }
});

// @platform: Selection and the system's copy chord (⌘C, Ctrl+Shift+C) with the clipboard's own line endings.
test("a drag selects locally without a click, and its copy joins wrapped lines and keeps a shared margin's indentation", { tag: "@platform" }, async ({ page, context }) => {
  const flow = await startFlow(page);
  try {
    const { sent } = flow;
    // A drag selects locally and is not a click. Its copy is what a native
    // terminal gives (B20): no padding to the column edge, the wrapped line
    // joined back into one, and the real line ends kept.
    const shellPaneId = await splitShell(page, flow);
    const shellView = page.locator(`[data-pane-view="${shellPaneId}"]`);
    const shellGrid = (await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, shellPaneId))!;
    const wrapped = "w".repeat(shellGrid.cols + 7);
    // The fixture selects ComSpec on Windows and zsh on Unix.
    await page.keyboard.type(process.platform === "win32"
      ? `cls&echo:${wrapped}&echo:short&echo:&echo:  in&echo:    deeper&echo:end\n`
      : `clear; echo ${wrapped}; echo short; echo; echo '  in'; echo '    deeper'; echo end\n`);
    await expect.poll(() => screen(page), { timeout: 15_000 }).toMatch(/^w+\s*\n\s*w+\s*\n\s*short\s*\n\s*\n\s+in\s*\n\s+deeper\s*\n\s*end/);
    const shellScreen = (await shellView.locator(".xterm-screen").boundingBox())!;
    const shellCell = (column: number, row: number) => ({
      x: shellScreen.x + (column + 0.5) * (shellScreen.width / shellGrid.cols),
      y: shellScreen.y + (row + 0.5) * (shellScreen.height / shellGrid.rows),
    });
    const clicksBeforeDrag = sent.get("terminal_click") ?? 0;
    // Dragged from the empty row back to the first cell: the split's divider
    // grab area covers the pane's first column, so the press starts inside.
    const pressAt = shellCell(shellGrid.cols - 2, 3);
    const dragTo = shellCell(0, 0);
    await page.mouse.move(pressAt.x, pressAt.y);
    await page.mouse.down();
    await page.mouse.move(dragTo.x - shellScreen.width / shellGrid.cols, dragTo.y, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneSelection(id) ?? null, shellPaneId)).toBe(`${wrapped}\nshort\n`);
    expect(sent.get("terminal_click") ?? 0).toBe(clicksBeforeDrag);
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await copy(page);
    // The Windows text clipboard represents line endings as CRLF.
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(`${wrapped}${os.EOL}short${os.EOL}`);
    // Lines that share a margin lose it and keep their relative indentation.
    const indentFrom = shellCell(shellGrid.cols - 2, 5);
    const indentTo = shellCell(0, 4);
    await page.mouse.move(indentFrom.x, indentFrom.y);
    await page.mouse.down();
    await page.mouse.move(indentTo.x - shellScreen.width / shellGrid.cols, indentTo.y, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneSelection(id) ?? null, shellPaneId)).toBe("in\n  deeper");
    await copy(page);
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(`in${os.EOL}  deeper`);
  } finally {
    flow.stop();
  }
});

// @platform: Zoom through the platform's Herdr, whose wider PTY the zoomed pane follows.
test("of three panes the focused one is outlined, and zoom hides the other two behind a chip", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { sent } = flow;
    const shellPaneId = await splitShell(page, flow);
    const shellPane = page.locator(`[data-pane-view="${shellPaneId}"]`);

    // Of three panes, the one holding the keyboard is outlined, and only it.
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(1);
    await expect(shellPane.locator("[data-pane-focus-outline]")).toBeVisible();

    await page.keyboard.press(chord("toggle_zoom"));
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "true");
    await expect.poll(() => sent.get("toggle_zoom")).toBe(1);
    // Zoom keeps the input perimeter and names the two panes it hides.
    await expect(page.locator("[data-pane-zoom]")).toHaveAttribute("data-pane-zoom", "2");
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(1);
    await expect(shellPane.locator("[data-pane-focus-outline]")).toBeVisible();
    await screenshot(page, "s2-zoom-chip");
    await page.locator("[data-pane-zoom]").click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "false");
    await expect.poll(() => sent.get("toggle_zoom")).toBe(2);
    await expect(page.locator('[data-pane-zoom]:not([data-pane-zoom="false"])')).toHaveCount(0);
  } finally {
    flow.stop();
  }
});

// @platform: The terminal menu's pane commands run through the platform's Herdr, and Zoom pane takes its wider PTY.
test("a right-click focuses its pane and opens the terminal menu, whose Zoom pane fills the canvas", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, sent } = flow;
    const shellPaneId = await splitShell(page, flow);
    const shellPane = page.locator(`[data-pane-view="${shellPaneId}"]`);
    const agentPane = herdr.panes[1];

    // A right-click selects its pane, but the menu holds the keyboard until
    // it closes, so no canvas carries input emphasis while it is open.
    const agentView = page.locator(`[data-pane-view="${agentPane}"]`);
    await agentView.locator("[data-terminal-host]").click({ button: "right", position: { x: 40, y: 40 } });
    await expect(page.getByRole("menu")).toBeVisible();
    await expect(agentView).toHaveAttribute("data-focused", "true");
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect.poll(() => herdrFocused(herdr), { timeout: 10_000 }).toBe(agentPane);
    await settledFocus(page, agentPane);
    await expect(agentView.locator("[data-pane-focus-outline]")).toBeVisible();
    await shellPane.locator(".xterm-helper-textarea").focus();
    await expect.poll(() => herdrFocused(herdr), { timeout: 10_000 }).toBe(shellPaneId);
    await settledFocus(page, shellPaneId);

    // A right-click in the terminal opens the editing, layout and pane menu.
    await shellPane.locator("[data-terminal-host]").click({ button: "right", position: { x: 40, y: 40 } });
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    for (const item of ["Paste", "Select all", "Find", "Split right", "Split down", "Zoom terminal", "Copy details"]) {
      await expect(menu.getByRole("menuitem", { name: item, exact: false }).first()).toBeVisible();
    }
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(0);
    await screenshot(page, "s2-terminal-menu");
    await menu.getByRole("menuitem", { name: /^Zoom terminal/ }).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "true");
    await expect.poll(() => sent.get("toggle_zoom")).toBe(1);
    // The zoomed pane takes the whole canvas, not just its own split cell,
    // and Herdr's wider PTY is what its terminal_resize follows.
    const canvasBox = (await page.locator("[data-canvas]").boundingBox())!;
    const zoomedBox = (await page.locator('[data-pane-view][data-focused="true"]').boundingBox())!;
    expect(Math.abs(zoomedBox.width - canvasBox.width)).toBeLessThan(2);
    expect(Math.abs(zoomedBox.height - canvasBox.height)).toBeLessThan(2);
    expect(Math.abs(zoomedBox.x - canvasBox.x)).toBeLessThan(2);
    await screenshot(page, "s2-zoomed");
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "false");
    await expect(page.locator("[data-pane-view]")).toHaveCount(3);
  } finally {
    flow.stop();
  }
});

// @platform: The shortcut sheet marks the chords a platform's browser keeps (⌘⇧M and the like), and ⌘F reaches the agent's own find through the platform's PTY.
test("the shortcut sheet marks the moved chords and ⌘F reaches the agent's own find, each without a stray key or find bar", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, sent } = flow;
    // ⌘/ opens the sheet from the registry with the moved chords marked.
    // The pane keeps keyboard focus under the sheet, and the Escape that
    // closes it is the shell's alone: no ESC byte reaches the program.
    await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
    const keysBeforeSheet = sent.get("key") ?? 0;
    await page.keyboard.press(chord("shortcuts"));
    await expect(page.locator("[data-shortcut-sheet]")).toBeVisible();
    // The complete command set includes the three direct Overview/sidebar commands
    // and, once the Factory opens (src/factory/launch.ts), Open Factory (software-factory-ui B2).
    // The two numbered families each fold into one row and have no browser chord;
    // they never carry a Chrome move note (electron-digit-shortcuts-hints B3).
    // The two Explorer path copies and its Cut, Copy and Paste are rows too,
    // and so are the sidebar's Projects | Tasks switch (sidebar-task-lens D-19, D-32)
    // and Show task list (task-split-checklist D-06).
    await expect(page.locator("[data-shortcut]")).toHaveCount(FACTORY_OPEN ? 55 : 54);
    for (const title of ["Overview", "Focus sidebar", "Add to backlog"]) await expect(page.locator("[data-shortcut-sheet]")).toContainText(title);
    // Add to backlog moves in a macOS browser, where Chrome keeps ⌘⇧M for its profiles; Alt+Shift+M is free on Windows and Linux.
    // Assert the exact moved commands, including the unchanged platform exceptions.
    const movedCommands = [
      "new_tab", "close_tab", "reopen_closed_tab", "recent_area_tab", "previous_recent_area_tab", "toggle_sidebar_lens",
      ...(SYSTEM === "mac" ? ["close_pane", "settings", "add_backlog", "toggle_lane_prs"] : []),
    ];
    const movedRows = page.locator("[data-shortcut]").filter({ hasText: "moved for Chrome" });
    await expect(movedRows).toHaveCount(movedCommands.length);
    expect(await movedRows.evaluateAll((rows) => rows.map((row) => row.getAttribute("data-shortcut")).sort())).toEqual(movedCommands.sort());
    await screenshot(page, "s2-shortcut-sheet");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-shortcut-sheet]")).toHaveCount(0);
    expect(sent.get("key") ?? 0).toBe(keysBeforeSheet);

    // ⌘F is intercepted from Chrome. The focused pane is an agent with its
    // own find and no history in Herdr, so the core is asked where the search
    // goes and the agent's search takes it; no find bar opens.
    await expect.poll(() => screen(page), { timeout: 15_000 }).toContain("claude");
    await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
    const findsBefore = sent.get("pane_find_open") ?? 0;
    const heard = () => herdr.inputLogs.map((file) => (fs.existsSync(file) ? fs.readFileSync(file, "latin1") : "")).join("");
    const heardBefore = heard().split("\x0f/").length;
    await page.keyboard.press(chord("find_in_pane"));
    await expect.poll(() => sent.get("pane_find_open") ?? 0).toBe(findsBefore + 1);
    // The agent hears its keys before the typing below, which would
    // otherwise interleave with them.
    await expect.poll(() => heard().split("\x0f/").length, { timeout: 10_000 }).toBe(heardBefore + 1);
    await expect(page.locator("[data-find-bar]")).toHaveCount(0);
  } finally {
    flow.stop();
  }
});

/** The panes of the last five distinct tabs in `shown`, the tabs in the order they were shown (ARCHITECTURE.md, the attach window). */
function lastFiveShown(shown: string[], tabPanes: Map<string, string[]>): string[] {
  const recent = [...new Set([...shown].reverse())].slice(0, 5);
  return recent.flatMap((tab) => tabPanes.get(tab)!).sort();
}

test("creating a sixth tab keeps the four retained tabs, and two revisit cycles restore every evicted tab", async ({ page }) => {
  // Five tabs in a folder whose workspace is not its owner, so the sixth,
  // created here, opens the owner workspace: Herdr answers with the new
  // pane before the core knows its workspace (I3 of the interaction audit).
  const flow = await startFlow(page, { tabLabels: ["second", "third", "fourth", "fifth"], ownerMark: false });
  try {
    // The fixture's first tab holds two panes, every other one pane.
    // Each pane carries its own text, typed while its tab was in front.
    const tabPanes = new Map<string, string[]>([[flow.herdr.tab, [...flow.herdr.panes].sort()]]);
    const typed = new Map<string, string>();
    const mark = async (paneId: string, text: string) => {
      typed.set(paneId, `${typed.get(paneId) ?? ""}${text}`);
      await page.locator(`[data-pane-view="${paneId}"] textarea`).focus();
      await page.keyboard.type(text);
      await expect.poll(() => paneText(page, paneId)).toContain(text);
    };
    await mark((await page.evaluate(() => window.__hideProbe!.paneId()))!, "keep-t1");
    const visit = async (tab: string) => {
      await page.locator(`[data-tab="${tab}"]`).click();
      await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab);
      for (const paneId of tabPanes.get(tab) ?? []) await expect(page.locator(`[data-pane-view="${paneId}"]`)).toHaveAttribute("data-transport", "controlling");
    };
    for (const tab of flow.tabs.slice(1)) {
      await visit(tab);
      const paneId = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
      tabPanes.set(tab, [paneId]);
      await mark(paneId, `keep-${tab}`);
    }
    // The fifth tab is in front: its terminal must be the same one after the sixth tab.
    const warmPane = tabPanes.get(flow.tabs[4])![0];
    const warmTextarea = await page.locator(`[data-pane-view="${warmPane}"] textarea`).elementHandle();
    const shown = [...flow.tabs];
    await expect.poll(() => attachedPanes(page)).toEqual(lastFiveShown(shown, tabPanes));
    await expect.poll(() => liveTerminals(page)).toEqual(lastFiveShown(shown, tabPanes));

    // The sixth tab releases the first tab's two panes and nothing else: no
    // list on the way drops one of the four retained tabs, and the released
    // panes keep their entries, marked released.
    const retained = shown.slice(1).flatMap((tab) => tabPanes.get(tab)!);
    const before = flow.terminalLists.length;
    await page.locator("[data-new-agent-tab]").first().click();
    await expect(page.locator("[data-tab]")).toHaveCount(6);
    await expect(page.locator("[data-pane-view]")).not.toHaveAttribute("data-pane-view", warmPane);
    await expect(page.locator("[data-pane-view]")).toHaveAttribute("data-transport", "controlling");
    await expect.poll(() => screen(page)).toContain("fixture %");
    const sixth = (await page.locator("[data-canvas]").getAttribute("data-canvas"))!;
    tabPanes.set(sixth, [(await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!]);
    await mark(tabPanes.get(sixth)![0], "keep-sixth");
    const order = [...flow.tabs, sixth];
    shown.push(sixth);
    await expect.poll(() => attachedPanes(page)).toEqual(lastFiveShown(shown, tabPanes));
    await expect.poll(() => liveTerminals(page)).toEqual(lastFiveShown(shown, tabPanes));
    expect(listsDropping(flow.terminalLists, before, retained)).toEqual([]);
    expect(flow.herdr.panes.map((paneId) => flow.terminalLists.at(-1)!.get(paneId))).toEqual(["released", "released"]);
    await visit(flow.tabs[4]);
    expect(await warmTextarea!.evaluate((element, id) => element === document.querySelector(`[data-pane-view="${id}"] textarea`), warmPane)).toBe(true);
    shown.push(flow.tabs[4]);

    // Two full cycles through the six tabs in strip order: each visit keeps
    // the last five shown, an evicted tab comes back with its text from
    // Herdr's frame, and every revisited pane takes new ordered input.
    for (const cycle of [1, 2]) {
      for (const tab of order) {
        await visit(tab);
        shown.push(tab);
        await expect.poll(() => attachedPanes(page)).toEqual(lastFiveShown(shown, tabPanes));
        await expect.poll(() => liveTerminals(page)).toEqual(lastFiveShown(shown, tabPanes));
        for (const paneId of tabPanes.get(tab)!) {
          if (typed.has(paneId)) await expect.poll(() => paneText(page, paneId)).toContain(typed.get(paneId)!);
        }
        await mark(tabPanes.get(tab)![0], `-c${cycle}`);
      }
    }
    const firstInput = flow.herdr.inputLogs[flow.herdr.panes.indexOf(tabPanes.get(flow.herdr.tab)![0])];
    if (firstInput) await expect.poll(() => fs.readFileSync(firstInput, "utf8")).toContain("-c1-c2");
  } finally {
    flow.stop();
  }
});

test("switching to another checkout and back keeps the first checkout's retained terminals", async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const retained: string[] = [...flow.herdr.panes];
    for (const tab of flow.tabs.slice(1)) {
      await page.locator(`[data-tab="${tab}"]`).click();
      await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab);
      await expect(page.locator("[data-pane-view]")).toHaveAttribute("data-transport", "controlling");
      retained.push((await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!);
    }
    const warmPane = retained.at(-1)!;
    await page.locator(`[data-pane-view="${warmPane}"] textarea`).focus();
    await page.keyboard.type("checkout-warm");
    await expect.poll(() => paneText(page, warmPane)).toContain("checkout-warm");
    await expect.poll(() => attachedPanes(page)).toEqual([...retained].sort());
    const warmTextarea = await page.locator(`[data-pane-view="${warmPane}"] textarea`).elementHandle();

    // Four tabs are shown at most, so every pane stays attached, and the
    // first checkout's terminals stay parked while the other is in front.
    const before = flow.terminalLists.length;
    const betaRow = await showProjectRow(page, page.locator("[data-project]", { hasText: "beta" }).locator("[data-checkout]").first());
    await betaRow.click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", flow.beta.result.tab.tab_id);
    await expect(page.locator("[data-pane-view]")).toHaveAttribute("data-transport", "controlling");
    const betaPane = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    const all = [...retained, betaPane].sort();
    await expect.poll(() => attachedPanes(page)).toEqual(all);
    await expect.poll(() => liveTerminals(page)).toEqual(all);
    expect(listsDropping(flow.terminalLists, before, retained)).toEqual([]);

    // Coming back finds the same terminal with its text, not a new one.
    await flow.firstRow.click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", flow.tabs[2]);
    await expect(page.locator(`[data-pane-view="${warmPane}"]`)).toHaveAttribute("data-transport", "controlling");
    expect(await warmTextarea!.evaluate((element, id) => element === document.querySelector(`[data-pane-view="${id}"] textarea`), warmPane)).toBe(true);
    expect(await paneText(page, warmPane)).toContain("checkout-warm");
    expect(listsDropping(flow.terminalLists, before, retained)).toEqual([]);
  } finally {
    flow.stop();
  }
});

// @platform: Typed echo through the platform's PTY, a terminal parked across a tab switch, and a dropped socket that comes back to the platform's hided and Herdr.
test("typed text echoes and parks across a tab switch, and a dropped socket comes back with the tab bar and splits", { tag: "@platform" }, async ({ page }) => {
  const flow = await startFlow(page);
  try {
    const { herdr, tabs, lastSent } = flow;
    // Typed text echoes in the focused pane.
    await expect.poll(() => screen(page), { timeout: 15_000 }).toContain("claude");
    await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
    await page.keyboard.type("s2-echo-4b2e");
    await expect.poll(() => screen(page), { timeout: 10_000 }).toContain("s2-echo-4b2e");

    // Leaving the tab parks its terminals instead of disposing them (D-05):
    // they are still live while away, and coming back shows the last frame
    // in the same tick as the click, before any frame could arrive.
    const echoPane = (await page.evaluate(() => window.__hideProbe?.paneId()))!;
    await page.locator(`[data-tab="${tabs[1]}"]`).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tabs[1]);
    expect(await page.evaluate(() => window.__hideProbe?.liveTerminals() ?? [])).toContain(echoPane);
    expect(await page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", echoPane)).toContain("s2-echo-4b2e");
    const backAtOnce = await page.evaluate(
      ({ tab, id }) => {
        document.querySelector<HTMLElement>(`[data-tab="${tab}"]`)!.click();
        return window.__hideProbe?.paneText(id) ?? "";
      },
      { tab: herdr.tab, id: echoPane },
    );
    expect(backAtOnce).toContain("s2-echo-4b2e");
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", herdr.tab);
    await expect(page.locator(`[data-pane-view="${echoPane}"]`)).toHaveAttribute("data-transport", /connected|controlling|idle/);
    await expect(page.locator(`[data-pane-view="${echoPane}"] [data-terminal]`)).toHaveCount(1);
    // Coming back is a fit on the same size, not a request for a full frame.
    await expect.poll(() => lastSent.get("terminal_viewport")?.new_view).toBe(false);

    // A dropped socket comes back with the tab bar and splits from the core (B14).
    await page.evaluate(() => window.__hideProbe?.dropSocket());
    await expect(page.locator("[data-connection]")).toHaveText(/reconnecting/, { timeout: 15_000 });
    await expect(page.locator("[data-connection]")).toHaveCount(0, { timeout: 15_000 });
    await expect(page.locator("[data-tab]")).toHaveCount(3);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await expect(page.locator("[data-split]")).toHaveCount(1);
    await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
    await page.keyboard.type("after-reconnect-1d7c");
    await expect.poll(() => screen(page), { timeout: 10_000 }).toContain("after-reconnect-1d7c");
    await screenshot(page, "s2-after-reconnect");
  } finally {
    flow.stop();
  }
});
