import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { chord, commandLabel } from "./chords";
import { focusPane, splitPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { quietFor } from "./wait";
import { enterWorkspace, screenshot, showDocuments } from "./wire";

// Pair view (PRD lane-desk-pair-view B16-B29): a pane drawn beside its
// Agent tab's desk, the body split in half under the tab row, told from the
// usual screen by the strip, the tab mark, the sidebar mark and a pressed ◧;
// the two sides keep one owner, and leaving it brings the tab's own layout
// back with every terminal at the size it had.

test.describe.configure({ timeout: 240_000 }); // wait-ok: two agents start and report their sessions before the first step

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const deskTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const paneView = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"]`);
const pairButton = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"] [data-pane-pair]`);
const strip = (page: Page) => page.locator("[data-pair-strip]");

/** Every `terminal_resize` the page sends, by pane. */
function countResizes(page: Page): Map<string, number> {
  const counts = new Map<string, number>();
  page.on("websocket", (ws) => {
    ws.on("framesent", (frame) => {
      try {
        const event = JSON.parse(String(frame.payload)) as { kind?: string; payload?: { pane_id?: string } };
        if (event.kind === "terminal_resize" && event.payload?.pane_id) counts.set(event.payload.pane_id, (counts.get(event.payload.pane_id) ?? 0) + 1);
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
    await expect(paneView(page, shell)).toBeVisible();

    // The watched agent's View lands on the desk; the other agent's waits in its pane's list.
    await open(leftAgent, "a.md");
    await open(otherAgent, "b.md");
    await showDocuments(page);
    await expect(deskTabs(page)).toHaveCount(1);
    await page.locator(`[data-pane-views="${other}"]`).click();
    await page.getByRole("menuitem", { name: /b\.md/ }).click();
    await expect(deskTabs(page)).toHaveCount(2);
    await quietFor(page, 500, "every pane's first fit has landed");

    // B19: a shell with nothing to show has ◧ off with why, and ⌘. on it changes nothing.
    await expect(pairButton(page, shell)).toHaveAttribute("data-pane-pair", "none");
    await expect(pairButton(page, shell)).toHaveAttribute("aria-disabled", "true");
    await expect(pairButton(page, left!)).toHaveAttribute("data-pane-pair", "off");
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
    await expect(strip(page)).toBeVisible();
    // B24, D-21: a tab the operator opened belongs to nobody, so the left pane stays.
    await page.locator('[data-explorer-row$="/notes.md"]').click().catch(async () => {
      await page.locator('[data-lane-tool="explorer"]').click();
      await page.locator('[data-explorer-row$="/notes.md"]').click();
    });
    await expect(front(page)).toHaveAttribute("aria-label", /\/notes\.md/);
    await expect(page.locator(`[data-pair-pane="${other}"]`)).toBeVisible();
    await screenshot(page, "pair-owner-less-tab");

    // B25: choosing the first agent in the sidebar takes it to the left with its own View on the right.
    await page.locator(`nav[data-sidebar] [data-agent-open="${left}"]`).first().click();
    await expect(page.locator(`[data-pair-pane="${left}"]`)).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);

    // B26: ⌘. brings the tab's own layout back, every pane at the place and grid it had, one resize for the left terminal.
    const beforeLeave = new Map(resizes);
    await page.keyboard.press(chord("pair_view"));
    await expect(screen(page)).not.toHaveAttribute("data-lane-mode", "pair");
    await expect(strip(page)).toHaveCount(0);
    for (const pane of [left!, other!, shell]) await expect(paneView(page, pane)).toBeVisible();
    await expect.poll(() => (resizes.get(left!) ?? 0) - (beforeLeave.get(left!) ?? 0)).toBe(1);
    await quietFor(page, 500, "no second resize follows the return");
    for (const pane of [left!, other!, shell]) {
      expect(await box(page, pane)).toEqual(before[pane]!.box);
      expect(await grid(page, pane)).toEqual(before[pane]!.grid);
    }
    for (const pane of [other!, shell]) expect(resizes.get(pane) ?? 0).toBe(beforeLeave.get(pane) ?? 0);
    expect((await live(page)).sort()).toEqual(liveBefore);
    await expect(page.locator("[data-pair-tab]")).toHaveCount(0);
    await screenshot(page, "pair-left");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
