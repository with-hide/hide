import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { chord, commandLabel } from "./chords";
import { closePane, focusPane, focusTab, newTab, splitPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { quietFor } from "./wait";
import { chooseExpand, enterWorkspace, screenshot, showDocuments, showTool } from "./wire";

// Pair view (PRD lane-desk-pair-view B16-B29, D-44..D-47): one mode for the
// window that draws the pane the operator is on beside its Agent tab's desk,
// under the tab row, split at a divider the operator drags, told from the
// usual screen by the strip, the tab mark, the sidebar mark and a pressed ◧;
// the two sides keep one owner, it goes with the operator to another agent,
// only its own ways out end it, and leaving it brings the tab's own layout
// back, with no desk on the right, every terminal at the size it had.

test.describe.configure({ timeout: 240_000 }); // wait-ok: two agents start and report their sessions before the first step

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const deskTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const paneView = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"]`);
const pairButton = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"] [data-pane-pair]`);
const strip = (page: Page) => page.locator("[data-pair-strip]");

/**
 * Every grid change the page sends, by pane: a `terminal_resize` whose size
 * differs from the one the pane last sent. A view attached again reports the
 * size it already had, which the core drops without resizing the PTY.
 */
function countResizes(page: Page): Map<string, number> {
  const counts = new Map<string, number>();
  const last = new Map<string, string>();
  page.on("websocket", (ws) => {
    ws.on("framesent", (frame) => {
      try {
        const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: { pane_id?: string; cols?: number; rows?: number } };
        const pane = event.payload?.pane_id;
        if (event.kind !== "terminal_resize" || !pane) return;
        const size = `${event.payload?.cols}x${event.payload?.rows}`;
        if (last.get(pane) !== size) counts.set(pane, (counts.get(pane) ?? 0) + 1);
        last.set(pane, size);
      } catch {
        /* the handshake is not an event */
      }
    });
  });
  return counts;
}

const grid = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, pane);
const live = (page: Page) => page.evaluate(() => window.__hideProbe?.liveTerminals() ?? []);
const box = async (page: Page, pane: string) => (await paneView(page, pane).boundingBox())!;

test("a pane goes beside its desk and back: one owner on both sides, three marks, the same layout and sizes after, and one resize each way", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const resizes = countResizes(page);
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    for (const name of ["a.md", "b.md", "notes.md"]) fs.writeFileSync(path.join(fixture, name), `# ${name}\n`);
    daemon = await startHided(engine, "pair-view");
    const [left, other] = engine.panes;
    const otherAgent = await agentPane(engine, daemon.stateDir, other!);
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    const shell = await withControl(daemon, async (control) => {
      const made = await splitPane(control, engine.tab, fixture, { direction: "down" });
      await focusPane(control, left!);
      return made;
    });
    const open = async (agent: typeof leftAgent, name: string) => {
      const answer = await agent(["file", "open", name]);
      expect(answer.status, answer.out).toBe(0);
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // The watched agent's View lands on the desk; the other agent's waits in its pane's list.
    await open(leftAgent, "a.md");
    await open(otherAgent, "b.md");
    await showDocuments(page);
    await expect(deskTabs(page)).toHaveCount(1);
    await page.locator(`[data-pane-views="${other}"]`).click();
    await page.getByRole("menuitem", { name: /b\.md/ }).click();
    await expect(deskTabs(page)).toHaveCount(2);
    await quietFor(page, 500, "every pane's first fit has landed");

    // The keyboard goes to the shell, so ◧ is pressed on a pane that does not hold it.
    await paneView(page, shell).locator("[data-terminal-host]").click();
    await expect(paneView(page, shell)).toHaveAttribute("data-focused", "true");
    await pairButton(page, left!).hover();
    await expect(page.getByRole("tooltip")).toContainText("Pair view");
    await expect(page.getByRole("tooltip")).toContainText(commandLabel("pair_view"));
    await screenshot(page, "pair-pane-button");

    const before = Object.fromEntries(await Promise.all([left!, other!, shell].map(async (pane) => [pane, { box: await box(page, pane), grid: await grid(page, pane) }] as const)));
    const resizesBefore = new Map(resizes);
    const liveBefore = (await live(page)).sort();

    // B18, B22: ◧ puts the pane on the left and its agent's View in front of the desk on the right, with the three marks.
    await pairButton(page, left!).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(strip(page)).toBeVisible();
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(paneView(page, other!)).toHaveCount(0);
    await expect(paneView(page, shell)).toHaveCount(0);
    await expect(pairButton(page, left!)).toHaveAttribute("aria-pressed", "true");
    await expect(paneView(page, left!), "the pane on screen takes the keyboard").toHaveAttribute("data-focused", "true");
    await expect(page.locator(`[data-pair-tab="${engine.tab}"]`)).toBeVisible();
    await expect(page.locator(`[data-pair-row="${left}"]`).first()).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    // D-41: the other agent's tab folds into one menu at the strip's end.
    await expect(deskTabs(page)).toHaveCount(1);
    await expect(page.locator('[data-pair-others="1"]')).toBeVisible();
    await screenshot(page, "pair-entered");

    // B29, D-26: the left terminal is the same instance, resized once, and no pane gained a terminal.
    expect((await live(page)).sort()).toEqual(liveBefore);
    await expect.poll(() => (resizes.get(left!) ?? 0) - (resizesBefore.get(left!) ?? 0)).toBe(1);
    await quietFor(page, 500, "no second resize follows the entry");
    expect((resizes.get(left!) ?? 0) - (resizesBefore.get(left!) ?? 0)).toBe(1);
    for (const pane of [other!, shell]) expect(resizes.get(pane) ?? 0).toBe(resizesBefore.get(pane) ?? 0);

    // B24, D-41: choosing the other agent's tab from the fold takes its pane to the left.
    await page.locator("[data-pair-others]").click();
    await page.locator("[data-pair-others-item]").first().click();
    await expect(page.locator(`[data-pair-pane="${other}"]`)).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    // The pane on screen becomes the tab's keyboard pane, so pane commands reach it; the keys stay on the desk.
    await expect(paneView(page, other!)).toHaveAttribute("data-focused", "true");
    await expect(paneView(page, other!).locator(".xterm-helper-textarea")).not.toBeFocused();
    // B24, D-21: a tab the operator opened belongs to nobody, so the left pane stays.
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/notes\.md/);
    await expect(page.locator(`[data-pair-pane="${other}"]`)).toBeVisible();
    await screenshot(page, "pair-owner-less-tab");

    // B25: choosing the first agent in the sidebar takes it to the left with its own View on the right.
    await page.locator(`nav[data-sidebar] [data-agent-open="${left}"]`).first().click();
    await expect(page.locator(`[data-pair-pane="${left}"] .xterm-helper-textarea`), "the keys follow the pane chosen").toBeFocused();
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // B26: ⌘. brings the tab's own layout back, every pane at the place and grid it had, one resize for the left terminal.
    const beforeLeave = new Map(resizes);
    await page.keyboard.press(chord("pair_view"));
    await expect(screen(page)).not.toHaveAttribute("data-lane-mode", "pair");
    // B26: the keyboard goes back to the pane that held it before entering.
    await expect(paneView(page, shell)).toHaveAttribute("data-focused", "true");
    for (const pane of [left!, other!, shell]) await expect(paneView(page, pane)).toBeVisible();
    await expect.poll(() => (resizes.get(left!) ?? 0) - (beforeLeave.get(left!) ?? 0)).toBe(1);
    await quietFor(page, 500, "no second resize follows the return");
    for (const pane of [left!, other!, shell]) {
      expect(await box(page, pane)).toEqual(before[pane]!.box);
      expect(await grid(page, pane)).toEqual(before[pane]!.grid);
    }
    // The shell never left its canvas; the other agent's pane was last drawn on the left (B24), so it takes its grid back once.
    expect(resizes.get(shell) ?? 0).toBe(beforeLeave.get(shell) ?? 0);
    expect((resizes.get(other!) ?? 0) - (beforeLeave.get(other!) ?? 0)).toBe(1);
    expect((await live(page)).sort()).toEqual(liveBefore);
    await expect(page.locator("[data-pair-tab]")).toHaveCount(0);
    await screenshot(page, "pair-left");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("pair view is the window's mode: a tab switch, ⌃⇥ and the sidebar keep it on the agent gone to, a restart keeps it, off stays off, and it ends when its pane closes", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    for (const name of ["a.md", "b.md"]) fs.writeFileSync(path.join(fixture, name), `# ${name}\n`);
    daemon = await startHided(engine, "pair-view-tab");
    const [left, other] = engine.panes;
    const second = await withControl(daemon, (control) => newTab(control, engine, "second"));
    // An agent in the second tab that opened a View of its own, which waits in its pane's list.
    const secondAgent = await agentPane(engine, daemon.stateDir, second.pane);
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    await withControl(daemon, async (control) => {
      await focusTab(control, engine, engine.tab);
      await focusPane(control, other!);
    });
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    // The watched pane is `other`: `left`'s open waits in its pane's list until the operator puts it on the desk.
    for (const [agent, name] of [[leftAgent, "a.md"], [secondAgent, "b.md"]] as const) {
      const answer = await agent(["file", "open", name]);
      expect(answer.status, answer.out).toBe(0);
    }
    await page.locator(`[data-pane-views="${left}"]`).click();
    await page.getByRole("menuitem", { name: /a\.md/ }).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // B17: the desk's ◧ puts the tab in front beside the pane of the agent it belongs to, not the focused one.
    await page.locator('[data-desk-pair="off"]').click();
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(paneView(page, left!), "the pane on screen takes the keyboard").toHaveAttribute("data-focused", "true");
    // D-42: the desk's ◧ is a way in only; pair view draws none, nor the desk's ✕ (D-46).
    await expect(page.locator("[data-desk-pair]")).toHaveCount(0);
    await expect(page.locator("[data-lane-documents-close]")).toHaveCount(0);
    await expect(strip(page)).toBeVisible();

    // D-45: a tab switch keeps pair view, the new tab's agent on the left and its View on its desk.
    await page.locator(`[data-agent-tab-bar] [data-tab="${second.tab}"]`).click();
    await expect(page.locator(`[data-pair-pane="${second.pane}"]`)).toBeVisible();
    await expect(strip(page)).toBeVisible();
    await expect(page.locator(`[data-pair-tab="${second.tab}"]`)).toBeVisible();
    await expect(page.locator(`[data-pair-tab="${engine.tab}"]`), "one mode, one mark").toHaveCount(0);
    await expect(deskTabs(page)).toHaveCount(1);
    await screenshot(page, "pair-tab-switch");

    // D-45: ⌃⇥ back to the first agent keeps it too, its own View in front.
    await page.keyboard.press(chord("recent_area_tab"));
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // D-45: so does the sidebar, to an agent in another tab.
    await page.locator(`nav[data-sidebar] [data-agent-open="${second.pane}"]`).first().click();
    await expect(page.locator(`[data-pair-pane="${second.pane}"]`)).toBeVisible();

    // A restart keeps the mode, drawn on the pane the operator is on.
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(page.locator(`[data-pair-pane="${second.pane}"]`)).toBeVisible({ timeout: 20_000 }); // wait-ok: the restarted daemon reads the saved mode and the page draws it again

    // D-44, D-45: the strip's way back ends it everywhere, with no desk left on the right; nothing turns it on again on the way back to a tab.
    await page.locator("[data-pair-leave]").click();
    await expect(strip(page)).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await page.locator(`[data-agent-tab-bar] [data-tab="${engine.tab}"]`).click();
    await expect(paneView(page, other!)).toBeVisible();
    await expect(strip(page)).toHaveCount(0);
    await page.locator(`[data-agent-tab-bar] [data-tab="${second.tab}"]`).click();
    await expect(paneView(page, second.pane)).toBeVisible();
    await expect(strip(page)).toHaveCount(0);
    await page.locator(`[data-agent-tab-bar] [data-tab="${engine.tab}"]`).click();

    // B26: the pressed ◧ toggles it.
    await pairButton(page, left!).click();
    await expect(pairButton(page, left!)).toHaveAttribute("aria-pressed", "true");
    await pairButton(page, left!).click();
    await expect(strip(page)).toHaveCount(0);
    await pairButton(page, left!).click();
    await expect(strip(page)).toBeVisible();

    // B28: the left pane closing ends pair view with no notice; the tab's other pane comes back.
    await withControl(daemon, (control) => closePane(control, left!));
    await expect(strip(page)).toHaveCount(0);
    await expect(paneView(page, other!)).toBeVisible();
    await expect(page.locator("[data-pair-tab]")).toHaveCount(0);
    await expect(page.locator('[role="alert"]')).toHaveCount(0);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("⌘. puts the focused pane beside its desk and takes it back, a shell draws no ◧, ⌘E and Escape leave pair view on, and the divider's split is the mode's", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const resizes = countResizes(page);
  const sent = { pair: 0 };
  page.on("websocket", (ws) => ws.on("framesent", (frame) => {
    if (String(frame.payload).includes('"kind":"pair_view"')) sent.pair += 1;
  }));
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    daemon = await startHided(engine, "pair-view-key");
    const [left] = engine.panes;
    // An agent with its session and nothing opened yet.
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    const shell = await withControl(daemon, (control) => splitPane(control, engine.tab, fixture, { direction: "down" }));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B19, D-49: a shell and an agent that opened nothing have nothing to show: no views and ◧ button at all, and ⌘. sends nothing.
    for (const pane of [shell, left!]) {
      await expect(paneView(page, pane).locator("[data-pane-split]")).toHaveCount(0);
      await paneView(page, pane).locator("[data-terminal-host]").click();
      await expect(paneView(page, pane)).toHaveAttribute("data-focused", "true");
      await page.keyboard.press(chord("pair_view"));
    }
    await quietFor(page, 300, "a refused ⌘. would have sent its event by now");
    expect(sent.pair).toBe(0);
    await expect(strip(page)).toHaveCount(0);

    // B18: once the agent opened a View, ⌘. on its pane enters with it on the desk.
    fs.writeFileSync(path.join(fixture, "a.md"), "# a.md\n");
    const opened = await leftAgent(["file", "open", "a.md"]);
    expect(opened.status, opened.out).toBe(0);
    await expect(pairButton(page, left!)).toHaveAttribute("data-pane-pair", "off");
    // B20, B21, D-43: ◧ stands left of the zoom, and stays drawn on a pane that does not hold the keyboard.
    const [pairBox, zoomBox] = await Promise.all([pairButton(page, left!).boundingBox(), paneView(page, left!).locator('[data-pane-zoom="false"]').boundingBox()]);
    expect(pairBox!.x + pairBox!.width).toBeLessThanOrEqual(zoomBox!.x + 1);
    await paneView(page, shell).locator("[data-terminal-host]").click();
    await page.mouse.move(1, 1);
    await expect(pairButton(page, left!)).toHaveCSS("opacity", "1");
    await paneView(page, left!).locator("[data-terminal-host]").click();
    await expect(paneView(page, left!)).toHaveAttribute("data-focused", "true");
    await page.keyboard.press(chord("pair_view"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(deskTabs(page)).toHaveCount(1);

    // D-46: ⌘E does nothing in pair view, and Escape in the desk only moves the keyboard.
    await page.keyboard.press(chord("toggle_lane"));
    await quietFor(page, 300, "a ⌘E that closed the lane would have redrawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await front(page).click();
    await page.keyboard.press("Escape");
    await quietFor(page, 300, "an Escape that ended pair view would have redrawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(strip(page)).toBeVisible();

    // D-47: the divider drags the split, the left terminal's grid changes once on release, and the ratio is the mode's.
    const pairPane = page.locator(`[data-pair-pane="${left}"]`);
    const start = (await pairPane.boundingBox())!;
    const divider = page.locator('[data-lane-divider="pair"]');
    const grip = (await divider.boundingBox())!;
    const resizesBefore = resizes.get(left!) ?? 0;
    await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await page.mouse.down();
    for (const step of [1, 2, 3, 4]) await page.mouse.move(grip.x + grip.width / 2 + step * 30, grip.y + grip.height / 2);
    await expect(page.locator("[data-lane-guide]")).toBeVisible();
    expect(resizes.get(left!) ?? 0, "no resize while dragging").toBe(resizesBefore);
    await page.mouse.up();
    // The landing is a ratio in thousandths, so the width lands within a pixel or two of the pointer.
    await expect.poll(async () => Math.abs((await pairPane.boundingBox())!.width - (start.width + 120))).toBeLessThanOrEqual(2);
    await expect.poll(() => (resizes.get(left!) ?? 0) - resizesBefore).toBe(1);
    await quietFor(page, 500, "no second resize follows the landing");
    expect((resizes.get(left!) ?? 0) - resizesBefore).toBe(1);
    await screenshot(page, "pair-divider-dragged");
    const dragged = (await pairPane.boundingBox())!.width;

    // B29: expanding the desk over the pane and back keeps the dragged split and resizes no terminal.
    const beforeExpand = resizes.get(left!) ?? 0;
    await chooseExpand(page);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await chooseExpand(page);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect.poll(async () => (await pairPane.boundingBox())!.width).toBe(dragged);
    expect(resizes.get(left!) ?? 0, "expanding and back resizes no terminal").toBe(beforeExpand);

    // D-44: ⌘. leaves, with no desk on the right; entering again draws the same split.
    await page.keyboard.press(chord("pair_view"));
    await expect(strip(page)).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await page.keyboard.press(chord("pair_view"));
    await expect(pairPane).toBeVisible();
    await expect.poll(async () => (await pairPane.boundingBox())!.width).toBe(dragged);

    // Off is off: on the usual screen, ⌘E opening the lane and a desk tab pressed keep the desk floating over the agents, with no pair view.
    await page.keyboard.press(chord("pair_view"));
    await expect(strip(page)).toHaveCount(0);
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await front(page).click();
    await quietFor(page, 300, "a hidden pair view would have been drawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "overlay");
    await expect(strip(page)).toHaveCount(0);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
