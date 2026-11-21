// A card the operator opens takes the keyboard once the core places it, on
// whichever Agent tab's desk the core put it, as an open from the Explorer or
// ⌘P does (UI_BEHAVIOR.md, Opening, preview, and Open to the side): the core
// focuses the card owner's pane in the same event, and before this the
// terminal kept the keys, so the close chord asked to close that agent
// instead of closing the card. Each way in here starts with the keys in a
// terminal, opens a card, and the next close chord closes the card alone.
// The pull request row's way in is in `work-prs.spec.ts`, on its fake `gh`.

import { expect, test, type Page } from "@playwright/test";
import type { HideAnswer } from "./agent-pane";
import { chord } from "./chords";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { focusPane, showTabOf, withControl } from "./daemon-actions";
import { setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, hoverSettledTaskRow, showTasks } from "./wire";

test.describe.configure({ timeout: 150_000 });

/** The page on the family fixture's checkout. */
async function openOn(page: Page, fixture: AgentFixture, label: string): Promise<Daemon> {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const daemon = await startHided(fixture, label);
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  return daemon;
}

/** The keys go to `pane`'s terminal, its tab shown, as a click on it gives them. */
async function keysIn(page: Page, daemon: Daemon, pane: string): Promise<void> {
  await withControl(daemon, async (control) => {
    await showTabOf(control, pane);
    await focusPane(control, pane);
  });
  await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
  await page.locator(`[data-terminal-host="${pane}"]`).click();
  await expect(page.locator(`[data-terminal-host="${pane}"] textarea`)).toBeFocused();
}

/**
 * The card of `kind` is in front of the desk with the keys, and the close
 * chord closes it: the agent `owner` stays, and no close of it is asked.
 */
async function closeChordClosesCard(page: Page, kind: string, owner: string): Promise<void> {
  const card = page.locator(`[data-card="${kind}"]`);
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-view-area-id] [role="tab"][data-tab-kind="card"][aria-selected="true"]')).toBeFocused();
  await page.keyboard.press(chord("close_tab"));
  await expect(card).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(`[data-pane-view="${owner}"]`)).toBeVisible();
}

/**
 * The desk of `owner`'s tab stands with the card of `kind` on it, opened by
 * `open`, and the keys are back in `owner`'s terminal: a closed lane coming
 * up hands the keys to the desk on its own, so the operator's case is a card
 * opened while the desk already stands.
 */
async function deskStandsWith(page: Page, daemon: Daemon, kind: string, owner: string, open: () => Promise<void>): Promise<void> {
  await open();
  await expect(page.locator(`[data-card="${kind}"]`)).toBeVisible({ timeout: 30_000 });
  await keysIn(page, daemon, owner);
  await expect(page.locator(`[data-card="${kind}"]`)).toBeVisible();
}

test("a session record opened from 내 차례 while its agent's desk stands takes the keys, from that tab or another, so the close chord closes the record", { tag: "@platform" }, async ({ page }) => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await openOn(page, fixture, "card-keyboard-record");
    const [parent, child] = fixture.panes;
    await setAgentState(daemon, parent, "blocked");
    const tasks = await showTasks(page);
    const row = tasks.locator(`[data-task-group="needs_you"] [data-task-row="${parent}"]`);
    const openRecord = async () => {
      await hoverSettledTaskRow(row, "needs_you");
      await row.locator(`[data-task-record="${parent}"]`).click();
    };
    await deskStandsWith(page, daemon, "record", parent, openRecord);
    // The same tab: the keys in the parent's terminal beside its standing desk.
    await openRecord();
    await closeChordClosesCard(page, "record", parent);

    // Another tab: from the child's own tab, the core brings the parent's tab forward and focuses its pane with the card.
    await deskStandsWith(page, daemon, "record", parent, openRecord);
    await keysIn(page, daemon, child);
    await openRecord();
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toHaveAttribute("data-focused", "true");
    await closeChordClosesCard(page, "record", parent);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

/** One `hide mission` command as the Conductor, which must succeed. */
async function mission(hide: (args: string[]) => Promise<HideAnswer>, args: string[]): Promise<void> {
  const ran = await hide(args);
  expect(ran.out.trim().split("\n").at(-1) ?? "", args.join(" ")).toContain('"ok":true');
}

test("a mission opened from its Conductor's pane chip or the sidebar's chip while the Conductor's desk stands takes the keys, so the close chord closes the mission and not the Conductor", async ({ page }) => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await openOn(page, fixture, "card-keyboard-mission");
    const [conductor, worker] = fixture.panes;
    const hide = fixture.runAs(conductor);
    // The core reads this machine's projects just after it starts; until then it answers that it is still reading.
    await expect.poll(async () => (await hide(["mission", "create", "--title", "키보드 확인"])).out, { timeout: 20_000 }).toContain('"ok":true');
    await mission(hide, ["mission", "plan", "--json", JSON.stringify({ scope: "pr", check: "true", nodes: [{ key: "api", title: "API 추가", kind: "change", delivery: "pr", criteria: ["엔드포인트가 응답한다"] }] })]);
    await mission(hide, ["mission", "start"]);
    await mission(hide, ["mission", "decide", "--line", "방식 선택"]);

    // The pane's mission chip, with the keys in the Conductor's terminal beside its standing desk.
    const chip = page.locator("[data-pane-mission-chip]");
    await deskStandsWith(page, daemon, "mission", conductor, () => chip.click());
    await chip.click();
    await closeChordClosesCard(page, "mission", conductor);

    // The sidebar's chip on the Conductor's 내 차례 row, from the worker's tab: the Conductor's tab comes forward.
    const sidebarChip = page.locator('nav[data-sidebar] [data-mission-chip][data-mission-chip-turn="decision"]').first();
    await deskStandsWith(page, daemon, "mission", conductor, () => chip.click());
    await keysIn(page, daemon, worker);
    await sidebarChip.click();
    await expect(page.locator(`[data-pane-view="${conductor}"]`)).toHaveAttribute("data-focused", "true");
    await closeChordClosesCard(page, "mission", conductor);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
