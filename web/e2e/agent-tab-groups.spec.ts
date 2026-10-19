import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, declareParent, printLinesCommand } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { waitForTerminal } from "./group-start-helper";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, showTool } from "./wire";
import { chord } from "./chords";
import { quietFor } from "./wait";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });
async function box(node: Locator) { const rect = await node.boundingBox(); if (!rect) throw new Error("Missing drag target"); return rect; }
async function drag(page: Page, tab: Locator, target: { x: number; y: number }, during?: () => Promise<void>) {
  await tab.scrollIntoViewIfNeeded();
  const rect = await box(tab);
  const start = { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x + 12, start.y + 2, { steps: 3 });
  await page.mouse.move(target.x, target.y, { steps: 10 });
  await during?.();
  await page.mouse.up();
}
const areas = (page: Page) => page.locator("[data-agent-area-id]");
const tab = (page: Page, id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
async function shape(page: Page) {
  return areas(page).evaluateAll((nodes) => nodes.map((area) => ({
    id: area.getAttribute("data-agent-area-id"), active: area.getAttribute("data-active-area"),
    tabs: [...area.querySelectorAll("[role=tab][data-tab]")].map((tab) => tab.getAttribute("data-tab")),
    shown: area.querySelector("[data-canvas]")?.getAttribute("data-canvas"),
  })));
}

// Three tabs in one Agent area, hided running. Each contract below starts
// here and puts itself into the shape it needs, so a failure names one contract.
async function startGroups(page: Page, name: string) {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  try {
    const created = ["second", "third"].map((label) => herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", label, "--no-focus"]) as { result: { tab: { tab_id: string }; root_pane: { pane_id: string } } });
    const [first, second, third] = [herdr.tab, ...created.map((row) => row.result.tab.tab_id)] as [string, string, string];
    const group = { herdr, created, first, second, third, daemon: await startHided(herdr, name), sent: countSent(page) };
    await page.goto(`${group.daemon.origin}/?probe=1#token=${group.daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(tab(page, third)).toBeVisible();
    expect((await shape(page))[0]?.tabs).toEqual([first, second, third]);
    // `stop` reads the daemon from the object it returns: a test that
    // restarts hided puts the new daemon there, and the old one is gone.
    const groups = { ...group, stop: () => { groups.daemon.stop(); herdr.stop(); } };
    return groups;
  } catch (error) {
    herdr.stop();
    throw error;
  }
}
type Groups = Awaited<ReturnType<typeof startGroups>>;
// Moves the third tab into an area of its own on the right, as a setup step.
async function splitThirdRight(page: Page, group: Groups) {
  const body = await box(page.locator("[data-agent-body]").first());
  await drag(page, tab(page, group.third), { x: body.x + body.width * .92, y: body.y + body.height / 2 });
  await expect(areas(page)).toHaveCount(2);
  await expect(areas(page).nth(1).locator("[data-canvas]")).toHaveAttribute("data-canvas", group.third);
}

// The right area holds the second and third tabs, in that order.
async function gatherRight(page: Page, group: Groups) {
  await splitThirdRight(page, group);
  const right = await box(tab(page, group.third));
  await drag(page, tab(page, group.second), { x: right.x + right.width + 20, y: right.y + right.height / 2 });
  await expect.poll(async () => (await shape(page))[1]?.tabs).toEqual([group.third, group.second]);
  const last = await box(tab(page, group.second));
  await drag(page, tab(page, group.third), { x: last.x + last.width + 15, y: last.y + last.height / 2 });
  await expect.poll(async () => (await shape(page))[1]?.tabs).toEqual([group.second, group.third]);
}

test("dragging a tab to the right edge previews a split, then splits into two live canvases", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-split");
  try {
    const { herdr, created, first, third, sent } = group;
    const original = await shape(page);
    const body = await box(page.locator("[data-agent-body]").first());
    const beforeResize = sent.get("terminal_resize") ?? 0;
    await drag(page, tab(page, third), { x: body.x + body.width * .92, y: body.y + body.height / 2 }, async () => {
      await expect(page.locator('[data-agent-drop="right"]')).toHaveText("Split right");
      await expect(page.locator("[data-agent-drag-tab]")).toBeVisible();
      expect(await shape(page)).toEqual(original);
      expect(sent.get("terminal_resize") ?? 0).toBe(beforeResize);
      await screenshot(page, "agent-groups-split-preview");
    });
    await expect(areas(page)).toHaveCount(2);
    await expect(areas(page).nth(0).locator("[data-canvas]")).toHaveAttribute("data-canvas", first);
    await expect(areas(page).nth(1).locator("[data-canvas]")).toHaveAttribute("data-canvas", third);
    expect(sent.get("agent_layout.split")).toBe(1);
    await expect(page.locator('[data-transport="released"]')).toHaveCount(0);
    // Both shown tabs keep receiving fresh output while the other owns focus.
    const live = [herdr.panes[0], created[1]!.result.root_pane.pane_id];
    for (const [index, pane] of live.entries()) execFileSync(herdr.bin, ["pane", "run", pane, printLinesCommand(herdr.shell, `area-${index}-live-`, 3)], { env: herdr.env, timeout: 10_000 });
    for (const [index, pane] of live.entries()) await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane)).toContain(`area-${index}-live-3`);
    await screenshot(page, "agent-groups-two-live-areas");
  } finally { group.stop(); }
});

test("two live Agent areas render in the light theme", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-light");
  try {
    await splitThirdRight(page, group);
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator("html")).toHaveClass(/(^|\s)light(\s|$)/);
    await page.keyboard.press("Escape");
    await screenshot(page, "agent-groups-two-live-areas-light");
  } finally { group.stop(); }
});

test("a tab dropped on its own area's edge is refused and nothing moves", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-refused");
  try {
    await splitThirdRight(page, group);
    const ownBody = await box(areas(page).nth(1).locator("[data-agent-body]"));
    const beforeInvalid = await shape(page);
    await drag(page, tab(page, group.third), { x: ownBody.x + 5, y: ownBody.y + ownBody.height / 2 }, async () => {
      await expect(page.locator("[data-agent-drop]")).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-agent-drag", "forbidden");
    });
    expect(await shape(page)).toEqual(beforeInvalid);
  } finally { group.stop(); }
});

test("tabs reorder inside an area by drag, and Escape cancels a drag in progress", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-reorder");
  try {
    const { first, second, third } = group;
    await splitThirdRight(page, group);
    // The second tab joins the third's area first, as setup for the reorder.
    const right = await box(tab(page, third));
    await drag(page, tab(page, second), { x: right.x + right.width + 20, y: right.y + right.height / 2 });
    await expect.poll(async () => (await shape(page))[1]?.tabs).toEqual([third, second]);
    const last = await box(tab(page, second));
    await drag(page, tab(page, third), { x: last.x + last.width + 15, y: last.y + last.height / 2 });
    await expect.poll(async () => (await shape(page))[1]?.tabs).toEqual([second, third]);
    const beforeCancel = await shape(page);
    const target = await box(tab(page, first));
    await drag(page, tab(page, third), { x: target.x + target.width + 15, y: target.y + target.height / 2 }, async () => {
      await expect(page.locator('[data-agent-drop="bar"]')).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.locator("[data-agent-drop]")).toHaveCount(0);
    });
    expect(await shape(page)).toEqual(beforeCancel);
  } finally { group.stop(); }
});

test("the divider resizes two areas, a narrow window shows one, and widening restores the shape", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-resize");
  try {
    await splitThirdRight(page, group);
    const divider = page.locator("[data-agent-divider]");
    const boundary = await box(divider);
    await page.mouse.move(boundary.x + boundary.width / 2, boundary.y + 150);
    await page.mouse.down();
    await page.mouse.move(boundary.x - 130, boundary.y + 150, { steps: 12 });
    await expect(page.locator("[data-agent-resize-guide]")).toBeVisible();
    await page.mouse.up();
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBeLessThan(50);
    await screenshot(page, "agent-groups-resized");
    const wideShape = await shape(page);
    await page.setViewportSize({ width: 700, height: 900 });
    await expect(areas(page)).toHaveCount(1);
    await expect(page.locator("[data-agent-area-switch]")).toBeVisible();
    await screenshot(page, "agent-groups-narrow");
    await page.setViewportSize({ width: 1920, height: 1080 });
    await expect.poll(() => shape(page)).toEqual(wideShape);
  } finally { group.stop(); }
});

test("the area layout and its divider ratio survive a hided restart", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-restart");
  try {
    await splitThirdRight(page, group);
    const divider = page.locator("[data-agent-divider]");
    const boundary = await box(divider);
    await page.mouse.move(boundary.x + boundary.width / 2, boundary.y + 150);
    await page.mouse.down();
    await page.mouse.move(boundary.x - 130, boundary.y + 150, { steps: 12 });
    await page.mouse.up();
    await expect.poll(async () => Number(await divider.getAttribute("aria-valuenow"))).toBeLessThan(50);
    const restoredShape = await shape(page);
    const ratio = await divider.getAttribute("aria-valuenow");
    await expect.poll(() => fs.existsSync(path.join(group.daemon.stateDir, "workspace-views.json"))).toBe(true);
    group.daemon = await group.daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${group.daemon.origin}/?probe=1#token=${group.daemon.token}`);
    await expect.poll(() => shape(page)).toEqual(restoredShape);
    await expect(divider).toHaveAttribute("aria-valuenow", ratio!);
    await screenshot(page, "agent-groups-restarted");
  } finally { group.stop(); }
});

test("a saved Workspace still reopens when its initial catalog arrives after the startup fallback", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-delayed-restore");
  try {
    await splitThirdRight(page, group);
    const restored = await shape(page);
    let deliver: ((message: string | Buffer) => void) | undefined;
    const held: (string | Buffer)[] = [];
    let holding = true;
    await page.routeWebSocket(/\/ws$/, socket => {
      const server = socket.connectToServer();
      deliver = message => socket.send(message);
      socket.onMessage(message => server.send(message));
      server.onMessage(message => {
        if (!holding) return socket.send(message);
        if (held.length >= 256) throw new Error("restore fixture frame cap");
        held.push(message);
        if (typeof message !== "string") return;
        const frame = JSON.parse(message);
        if (frame.type !== "snapshot") return;
        // The real first async-catalog frame: the saved choice still exists,
        // Herdr is connected, but no confirmed catalog/View has arrived.
        frame.payload.rest.navigator.workspaces = [];
        frame.payload.rest.workspace_view = null;
        frame.payload.rest.status.herdr.state = "connected";
        socket.send(JSON.stringify(frame));
      });
    });
    group.daemon = await group.daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${group.daemon.origin}/?probe=1#token=${group.daemon.token}`);
    const starting = page.locator('[data-center-screen="starting"]');
    await expect(starting).toBeVisible();
    await quietFor(page, 3500, "a pending saved restore outlives the existing 3-second bootstrap fallback");
    await expect(starting).toBeVisible();
    holding = false;
    for (const message of held) deliver!(message);
    await expect.poll(() => shape(page)).toEqual(restored);
    await screenshot(page, "agent-groups-delayed-restored");
  } finally { group.stop(); }
});

test("dropping onto another area's bar collapses the areas, and the other edges split again", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-edges");
  try {
    const { first, second, third, sent } = group;
    await gatherRight(page, group);
    const destination = await box(tab(page, second));
    await drag(page, tab(page, first), { x: destination.x + destination.width / 4, y: destination.y + destination.height / 2 }, async () => {
      await expect(page.locator('[data-agent-drop="bar"]')).toBeVisible();
    });
    await expect(areas(page)).toHaveCount(1);
    expect((await shape(page))[0]?.tabs).toEqual([first, second, third]);
    expect(sent.get("reorder_tab") ?? 0).toBe(0);
    await screenshot(page, "agent-groups-collapsed");
    for (const edge of ["left", "up", "down"] as const) {
      const content = await box(page.locator("[data-agent-body]"));
      const target = {
        x: content.x + content.width * (edge === "left" ? .05 : .5),
        y: content.y + content.height * (edge === "up" ? .05 : edge === "down" ? .95 : .5),
      };
      await drag(page, tab(page, third), target, async () => {
        await expect(page.locator(`[data-agent-drop="${edge}"]`)).toHaveText(`Split ${edge}`);
      });
      await expect(areas(page)).toHaveCount(2);
      const back = await box(tab(page, first));
      // The sidebar's centered resize grab owns the column's first 10px.
      // The first quarter of the tab is inside its bar, before its midpoint.
      await drag(page, tab(page, third), { x: back.x + back.width / 4, y: back.y + back.height / 2 }, async () => {
        await expect(page.locator('[data-agent-drop="bar"]')).toBeVisible();
      });
      await expect(areas(page)).toHaveCount(1);
    }
  } finally { group.stop(); }
});

test("a tab dragged onto the lane over Agent Views is refused and the areas stay as they were", async ({ page }) => {
  const group = await startGroups(page, "agent-groups-panel");
  try {
    await showTool(page, "changes");
    const panel = page.locator('[data-lane-frame="overlay"]');
    await expect(panel).toBeVisible();
    const floating = await box(panel);
    const beforePanelDrop = await shape(page);
    await drag(page, tab(page, group.first), { x: floating.x + floating.width - 5, y: floating.y + floating.height / 2 }, async () => {
      await expect(page.locator("[data-agent-drop]")).toHaveCount(0);
      await expect(page.locator("html")).toHaveAttribute("data-agent-drag", "forbidden");
    });
    expect(await shape(page)).toEqual(beforePanelDrop);
  } finally { group.stop(); }
});

test("New tab and Reopen use the requested area and Rename works in either bar", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const second = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--no-focus"]) as { result: { tab: { tab_id: string } } };
    const secondId = second.result.tab.tab_id;
    daemon = await startHided(herdr, "agent-placement");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const body = await box(page.locator("[data-agent-body]"));
    await drag(page, tab(page, secondId), { x: body.x + body.width * .95, y: body.y + body.height / 2 });
    await expect(areas(page)).toHaveCount(2);
    const leftId = (await shape(page))[0]!.id;
    const left = page.locator(`[data-agent-area-id="${leftId}"]`);
    await left.getByRole("button", { name: /^New tab/ }).click();
    await expect(left.locator('[role="tab"]')).toHaveCount(2);
    const createdId = (await shape(page))[0]!.tabs.find((id) => id !== herdr.tab)!;
    // New tab opens its terminal in this area before Rename and close/reopen apply.
    await waitForTerminal(left, createdId);
    await expect(left.locator("[data-canvas]")).toHaveAttribute("data-canvas", createdId);
    await expect(tab(page, secondId)).toHaveAttribute("aria-selected", "true");
    for (const id of [createdId, secondId]) {
      await tab(page, id).click({ button: "right" });
      await page.locator('[data-menu-item="rename_tab"]').click();
      const input = page.getByRole("textbox", { name: "Tab name", exact: true });
      await expect(input).toBeFocused();
      await input.fill(`영역 ${id}`);
      await input.press("Enter");
      await expect(input).toHaveCount(0);
      await expect(tab(page, id)).toContainText(`영역 ${id}`);
    }
    await tab(page, createdId).hover();
    await tab(page, createdId).getByRole("button", { name: /^Close tab/ }).click();
    await expect(tab(page, createdId)).toHaveCount(0);
    await tab(page, secondId).click();
    await expect(page.locator("[data-connection]")).toHaveCount(0);
    await page.keyboard.press(chord("reopen_closed_tab"));
    await expect(left.locator('[role="tab"]')).toHaveCount(2);
    await expect(left).toHaveAttribute("data-active-area", "true");
    expect(sent.get("reopen_closed")).toBe(1);
    await tab(page, secondId).hover();
    await tab(page, secondId).getByRole("button", { name: /^Close tab/ }).click();
    await expect(areas(page)).toHaveCount(1);
    // Closing an area does not imply a live transport. A shortcut during
    // reconnect is deliberately dropped, so prove recovery before sending
    // this new intent; never replay the key or relax the placement assertion.
    await page.evaluate(() => window.__hideProbe!.dropSocket());
    await expect(page.locator("[data-connection]")).toHaveAttribute("data-connection", "reconnecting");
    await expect(page.locator("[data-connection]")).toHaveCount(0);
    await page.keyboard.press(chord("reopen_closed_tab"));
    await expect(left.locator('[role="tab"]')).toHaveCount(3);
    expect(sent.get("reopen_closed")).toBe(2);
    await expect(areas(page)).toHaveCount(1);
    await screenshot(page, "agent-groups-reopen-placement");
  } finally { daemon?.stop(); herdr.stop(); }
});

// The chip's focus is held at the Herdr socket, so the click on the normal tab
// lands while the chip's focus is still unanswered: the order is the test's,
// not the runner's speed.
test("Delegated canvas returns to its normal tab while the chip's focus is still waiting for Herdr", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  let gate: Awaited<ReturnType<typeof herdrGate>> | undefined;
  try {
    const [parent, child] = herdr.panes;
    gate = await herdrGate(herdr);
    daemon = await startHided({ ...herdr, socket: gate.socket }, "agent-delegated-return");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    declareParent(herdr, child, parent);
    const treeButton = page.locator(`[data-pane-view="${parent}"] [data-pane-tree="${parent}"]`);
    await expect(treeButton).toBeVisible({ timeout: 20_000 });
    // The tree button shows once the lineage is known; the delegated canvas exists
    // once the core has moved the child out of its parent's tab. The list is
    // opened after that move, so the move's redraw cannot close it.
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveCount(0);
    await treeButton.click();
    const chip = page.locator(`[data-agent-child="${child}"]`);
    await expect(chip).toBeVisible();
    const held = gate.arm("pane.focus");
    await chip.click();
    await held;
    expect(gate.params("pane.focus").map((params) => params.pane_id)).toEqual([child]);
    // The core already shows the child while Herdr has not answered, so the normal tab is not the selected one.
    const quick = { intervals: [25] };
    await expect(tab(page, herdr.tab)).toHaveAttribute("aria-selected", "false", quick);
    const before = sent.get("agent_layout.focus") ?? 0;
    await tab(page, herdr.tab).click();
    await expect(tab(page, herdr.tab)).toHaveAttribute("aria-selected", "true", quick);
    expect(sent.get("agent_layout.focus")).toBe(before + 1);
    await gate.release();
    // The tab focus waits behind the pane focus and leaves once it is answered: seeing it
    // reach Herdr proves the late answer has been handled before the final state is read.
    await expect.poll(() => gate.params("tab.focus").map((params) => params.tab_id)).toContain(herdr.tab);
    expect(gate.maximum("pane.focus")).toBe(1);
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
    await expect(tab(page, herdr.tab)).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveCount(0);
  } finally { daemon?.stop(); await gate?.stop(); herdr.stop(); }
});

test("A delegated canvas's normal tab keeps the Agent commands in its menu", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const [parent, child] = herdr.panes;
    daemon = await startHided(herdr, "agent-delegated-menu");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    declareParent(herdr, child, parent);
    const treeButton = page.locator(`[data-pane-view="${parent}"] [data-pane-tree="${parent}"]`);
    await expect(treeButton).toBeVisible({ timeout: 20_000 });
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveCount(0);
    await treeButton.click();
    const chip = page.locator(`[data-agent-child="${child}"]`);
    await expect(chip).toBeVisible();
    await chip.click();
    // The child pane view is focused before the core has switched the drawn
    // canvas to the delegated tab; until then the normal tab is the selected one
    // and a click on it is a no-op. Wait for the delegated canvas to be drawn.
    await expect(tab(page, herdr.tab)).toHaveAttribute("aria-selected", "false");
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveAttribute("data-focused", "true");
    await tab(page, herdr.tab).click();
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
    await tab(page, herdr.tab).click({ button: "right" });
    await expect(page.locator('[role="menu"] [data-menu-group="split"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator('[role="menu"]')).toHaveCount(0);
    await screenshot(page, "agent-groups-delegated-return");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("An external focused creation joins a bar without replacing either shown canvas", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const second = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "second", "--no-focus"]) as { result: { tab: { tab_id: string } } };
    daemon = await startHided(herdr, "agent-external-focus");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(tab(page, second.result.tab.tab_id)).toBeVisible();
    const body = await box(page.locator("[data-agent-body]"));
    await drag(page, tab(page, second.result.tab.tab_id), { x: body.x + body.width - 5, y: body.y + body.height / 2 });
    await expect(areas(page)).toHaveCount(2);
    const before = (await shape(page)).map(({ id, active, shown }) => ({ id, active, shown }));
    const external = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "외부 생성", "--focus"]) as { result: { tab: { tab_id: string } } };
    await expect(tab(page, external.result.tab.tab_id)).toBeVisible();
    // Wait through the actual separate creation/layout/focus stream, including
    // the next read-only projection, rather than asserting its first frame.
    await expect.poll(async () => (await shape(page)).map(({ id, active, shown }) => ({ id, active, shown }))).toEqual(before);
    await quietFor(page, 1500, "an externally created tab does not move the layout");
    expect((await shape(page)).map(({ id, active, shown }) => ({ id, active, shown }))).toEqual(before);
    // A genuinely later external focus still selects a known tab.
    // Pinned Herdr emits no event for a no-op focus on its current tab.
    // Exercise an observable external focus transition after creation instead.
    herdr.run(["tab", "focus", second.result.tab.tab_id]);
    // `herdr tab focus` returns once Herdr has applied it, and its events reach the core in order,
    // so the first focus (already the shown tab of its bar, nothing to observe) precedes the second.
    herdr.run(["tab", "focus", external.result.tab.tab_id]);
    await expect(tab(page, external.result.tab.tab_id)).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(`[data-canvas="${external.result.tab.tab_id}"]`)).toBeVisible();
    await screenshot(page, "agent-groups-external-focus");
  } finally { daemon?.stop(); herdr.stop(); }
});
