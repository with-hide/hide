// The recent place cycle reaches an automation as it reaches an agent pane
// (issue 301, docs/UI_BEHAVIOR.md, Recent navigation): the page reports the
// automation's detail as a place, the core keeps it in the one order beside
// the panes and saves it in `core-state.json`, and the held chord walks both
// and commits either. The unit tests hold each side alone; only a real hided
// shows the page's report taken by the core's decoder, kept and sent back in
// the snapshot the next cycle reads.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { held } from "./chords";
import { startCaller, type CallerFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, sendEvent } from "./wire";

test.describe.configure({ timeout: 180_000 }); // wait-ok: a fixture with an agent pane and a daemon, started and stopped whole

/** Makes an automation through the same call the form sends, answered to this window. */
async function add(page: Page, daemon: Daemon, name: string): Promise<string> {
  const frame = await sendEvent(page, daemon, "automation_call", { request_id: `e2e-${Date.now()}`, call: { call: "add", name, task: "Look", agent: { kind: "claude" }, schedule: { kind: "daily", time: "09:00" }, worktree: false, run_now: false } }, "automation_result");
  const answer = (frame?.payload ?? {}) as { ok?: boolean; value?: { id?: string }; code?: string };
  if (!answer.ok || !answer.value?.id) throw new Error(`the core refused the automation: ${answer.code}`);
  return answer.value.id;
}

/** Holds the cycle's modifier, presses its key `times` times, and leaves the modifier held. */
async function hold(page: Page, times: number): Promise<() => Promise<void>> {
  const chord = held("recent_area_tab");
  for (const modifier of chord.modifiers) await page.keyboard.down(modifier);
  for (let step = 0; step < times; step += 1) await page.keyboard.press(chord.key);
  return async () => { for (const modifier of [...chord.modifiers].reverse()) await page.keyboard.up(modifier); };
}

test("the held chord walks an automation's detail and an agent pane in one order, saved by the core", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture: CallerFixture = await startCaller();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "recent-places");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const automation = await add(page, daemon, "morning issues");

    // The agent's pane takes the keyboard: the first place.
    const pane = page.locator(`[data-pane-view="${fixture.pane}"]`);
    await pane.click({ position: { x: 30, y: 60 } });
    // The automation's detail: the second place.
    await page.locator("[data-sidebar-automations]").click();
    await page.locator(`[data-automation-row="${automation}"] [data-automation-open]`).click();
    await expect(page.locator(`[data-automation-detail="${automation}"]`)).toBeVisible();

    const stored = () => (JSON.parse(fs.readFileSync(path.join(daemon!.stateDir, "core-state.json"), "utf8")) as { recent_places?: unknown[] }).recent_places;
    await expect.poll(stored).toEqual([{ kind: "automation", automation_id: automation }, { kind: "pane", pane_id: fixture.pane }]);

    // From the automation's detail the cycle starts on it and the first chord lands on the pane.
    const places = page.locator("[data-cycle=places]");
    const highlighted = places.locator("[aria-selected=true]");
    let release = await hold(page, 1);
    await expect(places.locator("[role=option]").first()).toHaveAttribute("data-cycle-kind", "automation");
    await expect(places.locator("[role=option]").first()).toContainText("morning issues");
    await expect(highlighted).toHaveAttribute("data-cycle-row", fixture.pane);
    await screenshot(page, "recent-places-from-automation");
    await release();
    await expect(places).toHaveCount(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(pane).toHaveAttribute("data-focused", "true");
    await expect.poll(stored).toEqual([{ kind: "pane", pane_id: fixture.pane }, { kind: "automation", automation_id: automation }]);

    // And back: from the pane, the first chord lands on the automation, whose detail the commit shows.
    release = await hold(page, 1);
    await expect(highlighted).toHaveAttribute("data-cycle-kind", "automation");
    await expect(highlighted).toHaveAttribute("data-cycle-row", automation);
    await screenshot(page, "recent-places-from-pane");
    await release();
    await expect(page.locator(`[data-automation-detail="${automation}"]`)).toBeVisible();
    await expect.poll(stored).toEqual([{ kind: "automation", automation_id: automation }, { kind: "pane", pane_id: fixture.pane }]);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
