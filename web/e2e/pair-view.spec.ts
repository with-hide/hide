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

// Pair view (PRD lane-desk-pair-view B16-B29, D-45..D-47, D-51, D-53): one mode
// for the window that draws the pane the operator is on beside its Agent tab's
// desk, or alone at the body's full width while it has no View to show,
// under the band that stands in the tab row's place, split at a divider the
// operator drags, told from the usual screen by that band, the sidebar mark
// and a pressed ◧ (D-61); ⌘E folds and stands its desk (D-57);
// the two sides keep one owner, it goes with the operator to another agent,
// only its own ways out end it, and leaving it brings the tab's own layout
// back, with no desk on the right, every terminal at the size it had.

test.describe.configure({ timeout: 240_000 }); // wait-ok: two agents start and report their sessions before the first step

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const deskTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const paneView = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"]`);
const pairButton = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"] [data-pane-pair]`);
const band = (page: Page) => page.locator("[data-pair-strip]");

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
    // The desk stands beside the panes, b.md in front; leaving pair view brings this screen back (D-50, D-51).
    await page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"][aria-label*="/b.md"]').waitFor();
    await quietFor(page, 500, "every pane's first fit has landed");

    // The keyboard goes to the shell, so ◧ is pressed on a pane that does not hold it.
    await paneView(page, shell).locator("[data-terminal-host]").click();
    await expect(paneView(page, shell)).toHaveAttribute("data-focused", "true");
    await pairButton(page, left!).hover();
    await expect(page.getByRole("tooltip")).toContainText("Focus mode");
    await expect(page.getByRole("tooltip")).toContainText(commandLabel("pair_view"));
    await screenshot(page, "pair-pane-button");

    const before = Object.fromEntries(await Promise.all([left!, other!, shell].map(async (pane) => [pane, { box: await box(page, pane), grid: await grid(page, pane) }] as const)));
    const resizesBefore = new Map(resizes);
    const liveBefore = (await live(page)).sort();

    // B18, B22: ◧ puts the pane on the left and its agent's View in front of the desk on the right, with the three marks.
    await pairButton(page, left!).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(band(page)).toBeVisible();
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(paneView(page, other!)).toHaveCount(0);
    await expect(paneView(page, shell)).toHaveCount(0);
    await expect(pairButton(page, left!)).toHaveAttribute("aria-pressed", "true");
    await expect(paneView(page, left!), "the pane on screen takes the keyboard").toHaveAttribute("data-focused", "true");
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

    // B26, D-51: ⌘. brings the screen before back, the desk standing with the View then in front and every pane at the place and grid it had, one resize for the left terminal.
    const beforeLeave = new Map(resizes);
    await page.keyboard.press(chord("pair_view"));
    // B26: the keyboard goes back to the pane that held it before entering.
    await expect(paneView(page, shell)).toHaveAttribute("data-focused", "true");
    for (const pane of [left!, other!, shell]) await expect(paneView(page, pane)).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
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
    await screenshot(page, "pair-left");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("focus mode's desk shows the left pane's tabs and what the operator opens meanwhile, folds the rest, and a closed tab is followed by the left pane's own (D-60)", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    for (const name of ["a.md", "c.md", "b.md", "notes.md", "later.md"]) fs.writeFileSync(path.join(fixture, name), `# ${name}\n`);
    daemon = await startHided(engine, "pair-view-own-tabs");
    const [left, other] = engine.panes;
    const otherAgent = await agentPane(engine, daemon.stateDir, other!);
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    await withControl(daemon, (control) => focusPane(control, left!));
    const open = async (agent: typeof leftAgent, name: string) => {
      const answer = await agent(["file", "open", name]);
      expect(answer.status, answer.out).toBe(0);
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // Before focus mode: the left agent's two files, the other agent's file brought onto the desk, and one the operator opened.
    await open(leftAgent, "a.md");
    await open(leftAgent, "c.md");
    await open(otherAgent, "b.md");
    await showDocuments(page);
    await page.locator(`[data-pane-views="${other}"]`).click();
    await page.getByRole("menuitem", { name: /b\.md/ }).click();
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    await expect(deskTabs(page)).toHaveCount(4);

    // (1)-(3): the strip holds the left agent's tabs only; the other agent's and the operator's earlier one fold.
    await pairButton(page, left!).click();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    const labels = () => deskTabs(page).evaluateAll((tabs) => tabs.map((tab) => tab.getAttribute("aria-label") ?? "").map((label) => label.split(" · ")[0]!.replace(/.*\//, "")).sort());
    await expect.poll(labels).toEqual(["a.md", "c.md"]);
    await expect(page.locator('[data-pair-others="2"]')).toBeVisible();

    // (5): closing the tab in front brings the left agent's other tab in front, never a folded one.
    await front(page).click();
    await page.keyboard.press(chord("close_tab"));
    await expect(deskTabs(page)).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/(a|c)\.md/);
    await expect(page.locator('[data-pair-others="2"]')).toBeVisible();

    // (4): what the operator opens while the mode is on stands in front, on the strip.
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/later.md"]').click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/later\.md/);
    await expect(deskTabs(page)).toHaveCount(2);
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await screenshot(page, "pair-own-tabs");

    // D-64: closing what the strip shows, the operator's later.md and the left agent's last tab, leaves the mode on and
    // the pane alone over the whole body; the folded tabs stay folded and never come in front.
    for (let shown = 2; shown > 0; shown -= 1) {
      await front(page).click();
      await page.keyboard.press(chord("close_tab"));
      if (shown > 1) await expect(deskTabs(page)).toHaveCount(shown - 1);
    }
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(page.locator("[data-pair-others]")).toBeHidden();
    const body = (await page.locator("[data-lane-row]").boundingBox())!;
    const alone = (await paneView(page, left!).boundingBox())!;
    expect(Math.abs(alone.width - body.width)).toBeLessThanOrEqual(1);
    await expect(paneView(page, left!).locator(".xterm-helper-textarea"), "the folded desk hands the keys to the pane").toBeFocused();
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
    await expect(band(page)).toBeVisible();

    // D-45: a tab switch keeps pair view, the new tab's agent on the left and its View on its desk; the band hides the tab row,
    // so the sidebar is the way there (D-61).
    await page.locator(`nav[data-sidebar] [data-agent-open="${second.pane}"]`).first().click();
    await expect(page.locator(`[data-pair-pane="${second.pane}"]`)).toBeVisible();
    await expect(band(page)).toBeVisible();
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

    // D-45, D-51: the pressed ◧ ends it everywhere; the second tab's desk, which only the mode brought up, folds, and nothing turns the mode on again on the way back to a tab.
    await page.locator('[data-pane-pair="on"]').click();
    await expect(band(page)).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await page.locator(`[data-agent-tab-bar] [data-tab="${engine.tab}"]`).click();
    await expect(paneView(page, other!)).toBeVisible();
    await expect(band(page)).toHaveCount(0);
    // D-51: the first tab's desk is back as the operator left it before the mode drew it, standing with a.md in front.
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await page.locator(`[data-agent-tab-bar] [data-tab="${second.tab}"]`).click();
    await expect(paneView(page, second.pane)).toBeVisible();
    await expect(band(page)).toHaveCount(0);
    await page.locator(`[data-agent-tab-bar] [data-tab="${engine.tab}"]`).click();
    // The desk closed, the panes are wide enough for the head's ◧.
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await page.keyboard.press(chord("toggle_lane"));
    await page.locator('[data-workspace-screen][data-lane-mode="closed"]').waitFor();

    // B26: the pressed ◧ toggles it.
    await pairButton(page, left!).click();
    await expect(pairButton(page, left!)).toHaveAttribute("aria-pressed", "true");
    await pairButton(page, left!).click();
    await expect(band(page)).toHaveCount(0);
    await pairButton(page, left!).click();
    await expect(band(page)).toBeVisible();

    // B28: the left pane closing ends pair view with no notice; the tab's other pane comes back.
    await withControl(daemon, (control) => closePane(control, left!));
    await expect(band(page)).toHaveCount(0);
    await expect(paneView(page, other!)).toBeVisible();
    await expect(page.locator('[role="alert"]')).toHaveCount(0);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("⌘. on a shell in a two-pane tab draws it alone at the full width and takes both panes back, and a tab switch in the mode to a shell draws it alone", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "pair-view-alone");
    // The fixture's tab holds two shells side by side; a second tab holds one more.
    const [left, shell] = engine.panes;
    const second = await withControl(daemon, async (control) => {
      const tab = await newTab(control, engine, "second");
      await focusTab(control, engine, engine.tab);
      return tab;
    });
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B19, D-53: a shell keeps a quiet head, with no ◧.
    for (const pane of [left!, shell!]) await expect(paneView(page, pane).locator("[data-pane-split]")).toHaveCount(0);
    await quietFor(page, 500, "every pane's first fit has landed");
    const before = Object.fromEntries(await Promise.all([left!, shell].map(async (pane) => [pane, { box: await box(page, pane), grid: await grid(page, pane) }] as const)));

    // D-53: ⌘. on the shell draws it alone at the body's full width, with no desk, the band in the tab row's place and its pressed ◧.
    await paneView(page, shell).locator("[data-terminal-host]").click();
    await expect(paneView(page, shell)).toHaveAttribute("data-focused", "true");
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toBeVisible();
    const alonePane = page.locator(`[data-pair-pane="${shell}"]`);
    await expect(alonePane).toBeVisible();
    await expect(page.locator(`[data-pair-layer="${shell}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(paneView(page, left!)).toHaveCount(0);
    await expect(pairButton(page, shell)).toHaveAttribute("aria-pressed", "true");
    const row = (await page.locator("[data-lane-row]").boundingBox())!;
    await expect.poll(async () => Math.abs((await alonePane.boundingBox())!.width - row.width)).toBeLessThanOrEqual(1);
    await screenshot(page, "pair-alone");

    // D-51, D-53: ⌘. again brings both panes back at the place and grid they had, the lane as it was.
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toHaveCount(0);
    for (const pane of [left!, shell]) await expect(paneView(page, pane)).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    for (const pane of [left!, shell]) {
      await expect.poll(() => box(page, pane)).toEqual(before[pane]!.box);
      await expect.poll(() => grid(page, pane)).toEqual(before[pane]!.grid);
    }

    // D-53: in the mode, a tab switch to a shell draws that shell alone at the full width too, and ⌘. ends it there.
    await page.keyboard.press(chord("pair_view"));
    await expect(alonePane).toBeVisible();
    // The band hides the tab row (D-61), so the switch comes from outside the page.
    await withControl(daemon, (control) => focusTab(control, engine, second.tab));
    await expect(page.locator(`[data-pair-layer="${second.pane}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(band(page)).toBeVisible();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect.poll(async () => Math.abs((await page.locator(`[data-pair-pane="${second.pane}"]`).boundingBox())!.width - row.width)).toBeLessThanOrEqual(1);
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toHaveCount(0);
    await expect(paneView(page, second.pane)).toBeVisible();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("⌘. puts the focused pane beside its desk and takes it back, a shell draws no ◧, ⌘E and Escape leave pair view on, and the divider's split is the mode's", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const resizes = countResizes(page);
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

    // B19, D-53: a shell and an agent that opened nothing keep a quiet head, with no views and no ◧.
    for (const pane of [shell, left!]) await expect(paneView(page, pane).locator("[data-pane-split]")).toHaveCount(0);

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
    // D-65: ⇧⌘↩ on the pane is the tab's zoom, which focus mode leaves alone: nothing moves and the mode stays.
    await page.keyboard.press(chord("toggle_zoom"));
    await quietFor(page, 300, "a zoom or a mode change would have redrawn by now");
    await expect(page.locator('[data-canvas][data-zoomed="true"]')).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");

    // D-57: ⌘E folds the desk, drawing the pane alone as a pane zoom did, and stands it again; the mode stays on.
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(band(page)).toBeVisible();
    await screenshot(page, "pair-desk-folded");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(deskTabs(page)).toHaveCount(1);
    // D-46: Escape in the desk only moves the keyboard.
    await front(page).click();
    await page.keyboard.press("Escape");
    await quietFor(page, 300, "an Escape that ended pair view would have redrawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(band(page)).toBeVisible();

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

    // B29, D-59: with the keyboard on the desk, ⇧⌘↩ expands it over the pane and back, as the desk's
    // expand does; the mode stays on, the dragged split is kept and no terminal resizes.
    const beforeExpand = resizes.get(left!) ?? 0;
    await front(page).click();
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveCount(1);
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await chooseExpand(page);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await chooseExpand(page);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect.poll(async () => (await pairPane.boundingBox())!.width).toBe(dragged);
    expect(resizes.get(left!) ?? 0, "expanding and back resizes no terminal").toBe(beforeExpand);

    // D-51, D-59: ⌘. pressed while the desk is expanded leaves to the screen before, here the lane
    // closed and nothing expanded; entering again draws the same split, unexpanded.
    await front(page).click();
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "expanded");
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await page.keyboard.press(chord("pair_view"));
    await expect(pairPane).toBeVisible();
    await expect.poll(async () => (await pairPane.boundingBox())!.width).toBe(dragged);

    // Off is off: on the usual screen, ⌘E opening the lane and a desk tab pressed keep the desk standing beside the agents, with no pair view (D-50).
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toHaveCount(0);
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await front(page).click();
    await quietFor(page, 300, "a hidden pair view would have been drawn by now");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await expect(band(page)).toHaveCount(0);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("the rail's desk button folds and stands focus mode's desk, counting the pane's Views while folded, and a tool stands under the band (D-61, D-63)", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    daemon = await startHided(engine, "pair-view-rail");
    const [left, shell] = engine.panes;
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    fs.writeFileSync(path.join(fixture, "a.md"), "# a.md\n");
    const opened = await leftAgent(["file", "open", "a.md"]);
    expect(opened.status, opened.out).toBe(0);
    await paneView(page, left!).locator("[data-terminal-host]").click();
    const tabRow = (await page.locator("[data-tab-bar]").first().boundingBox())!;
    const agents = (await page.locator("[data-agent-area]").boundingBox())!;
    await page.keyboard.press(chord("pair_view"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "pair");
    await expect(deskTabs(page)).toHaveCount(1);
    // D-61: the band takes the tab row's place and height, so the body is as tall as outside the mode.
    const bandBox = (await band(page).boundingBox())!;
    const paneBox = (await paneView(page, left!).boundingBox())!;
    expect(Math.abs(bandBox.y - tabRow.y)).toBeLessThanOrEqual(1);
    expect(Math.abs(bandBox.height - tabRow.height)).toBeLessThanOrEqual(1);
    expect(Math.abs(paneBox.y - (tabRow.y + tabRow.height))).toBeLessThanOrEqual(1);
    expect(Math.abs(paneBox.y + paneBox.height - (agents.y + agents.height))).toBeLessThanOrEqual(1);

    // D-63: the sidebar and the rail stay; the rail's desk button is ⌘E here, pressed while the desk stands and,
    // folded, counting the left pane's Views; choosing one from the pane's `N views` stands the desk again.
    const deskButton = page.locator('[data-tool-rail] [data-lane-tool="documents"]');
    await expect(page.locator("nav[data-sidebar]")).toBeVisible();
    await expect(deskButton).toHaveAttribute("aria-pressed", "true");
    await expect(deskButton.locator("[data-lane-badge]")).toHaveCount(0);
    await deskButton.click();
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(deskButton).toHaveAttribute("aria-pressed", "false");
    await expect(deskButton.locator('[data-lane-badge="1"]')).toBeVisible();
    await deskButton.click();
    await expect(page.locator(`[data-pair-layer="${left}"]`)).not.toHaveAttribute("data-pair-alone", "true");
    await expect(deskButton).toHaveAttribute("aria-pressed", "true");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveAttribute("data-pair-alone", "true");
    await paneView(page, left!).locator("[data-pane-views]").click();
    await page.locator(`[data-pane-views-list="${left}"] [role="menuitem"]`).first().click();
    await expect(page.locator(`[data-pair-layer="${left}"]`)).not.toHaveAttribute("data-pair-alone", "true");
    await expect(front(page)).toHaveAttribute("aria-label", /a\.md/);
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator(`[data-pair-layer="${left}"]`)).toHaveAttribute("data-pair-alone", "true");
    // D-61: a tool floating over the pane drawn alone stands under the band, so its way back is never covered.
    await showTool(page, "explorer");
    const leave = band(page).locator("[data-pair-leave]");
    await expect.poll(() => leave.evaluate((button) => {
      const rect = button.getBoundingClientRect();
      return button.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    })).toBe(true);
    await page.locator('[data-lane-tool="explorer"]').click();
    await expect(page.locator('aside[data-lane="explorer"]')).toHaveCount(0);

    // D-60, D-63: on the shell the desk holds only the agent's a.md, which its strip folds; the core says nothing can
    // stand, so ⌘E and the rail button ask for nothing, and the sidebar back to the agent leaves the keys in its terminal.
    await page.keyboard.press(chord("pair_view"));
    await expect(band(page)).toHaveCount(0);
    await withControl(daemon, (control) => focusPane(control, shell!));
    await expect(paneView(page, shell!)).toHaveAttribute("data-focused", "true");
    await page.keyboard.press(chord("pair_view"));
    await expect(page.locator(`[data-pair-layer="${shell}"]`)).toHaveAttribute("data-pair-alone", "true");
    await expect(deskButton).toHaveAttribute("aria-disabled", "true");
    await page.keyboard.press(chord("toggle_lane"));
    await page.locator(`nav[data-sidebar] [data-agent-open="${left}"]`).first().click();
    await expect(page.locator(`[data-pair-layer="${left}"]`)).not.toHaveAttribute("data-pair-alone", "true");
    await expect(paneView(page, left!).locator(".xterm-helper-textarea")).toBeFocused();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("right after ⌘. a desk tab pressed, the one in front or another, takes the keyboard from the pane", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    daemon = await startHided(engine, "pair-view-tab-keys");
    const [left] = engine.panes;
    const leftAgent = await agentPane(engine, daemon.stateDir, left!);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    fs.writeFileSync(path.join(fixture, "a.md"), "# a.md\n");
    fs.writeFileSync(path.join(fixture, "b.md"), "# b.md\n");
    for (const args of [["file", "open", "a.md"], ["file", "open", "b.md"]]) {
      const opened = await leftAgent(args);
      expect(opened.status, opened.out).toBe(0);
    }
    const terminal = paneView(page, left!).locator(".xterm-helper-textarea");
    const keysOnDesk = () => page.evaluate(() => document.activeElement?.closest("[data-view-tab-bar], [data-view-area]") != null);
    // b.md, opened last, is in front; a.md is behind it. A page's tab is the desktop host's and is checked there.
    for (const pressed of ['[aria-selected="true"]', '[aria-selected="false"]']) {
      for (const first of [true, false]) {
        // The desk is already open beside the agents, as the operator has it, so its tabs can be pressed at once.
        if (first) await showDocuments(page);
        await paneView(page, left!).locator("[data-terminal-host]").click({ position: { x: 12, y: 12 } });
        await expect(terminal).toBeFocused();
        // Right after ⌘. the tab is pressed as soon as it can be, before the core's answer to ⌘. has landed.
        if (first) await page.keyboard.press(chord("pair_view"));
        await page.locator(`[data-view-tab-bar] [role="tab"]${pressed}`).click();
        await expect(band(page)).toBeVisible();
        await quietFor(page, 500, "every late keyboard request has landed");
        expect(await keysOnDesk(), `${pressed} ${first ? "right after ⌘." : "later"}`).toBe(true);
        if (!first) await page.keyboard.press(chord("pair_view"));
        await expect(band(page)).toHaveCount(first ? 1 : 0);
      }
    }
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
