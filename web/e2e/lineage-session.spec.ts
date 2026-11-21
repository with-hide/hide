// A relationship holds only while both panes still host the sessions it was
// written for, on either engine; an agent that takes a pane over takes the
// registration there with its parent and its children (PRD
// letters-missions-simplify D-24), so the core writes the relationship again
// for the sessions the panes run now.

import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { agentIn, withControl } from "./daemon-actions";
import { takeOverSession, spawnChild } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

test("an agent that takes a pane over takes its place in the lineage: a child's stays the child, a parent's keeps the children", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "lineage-session");
    const live = daemon;
    const [parent, child] = fixture.panes;
    await open(page, live);
    const row = (pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`);
    await expect(row(parent)).toBeVisible({ timeout: 20_000 });
    // The row's parent as the core draws it, once the tokens name the sessions the panes run now.
    const parentOf = (pane: string, of: string) =>
      withControl(live, (control) => control.until((rest) => agentIn(rest, pane)?.lineage_parent_pane_id === of || undefined, `${pane} drawn under ${of}`, 20_000));

    // Spawned by the parent, the child is delegated: folded under its parent, not a row of its own.
    await expect(row(child)).toHaveCount(0, { timeout: 20_000 });

    // The child's agent ends and a different one starts in the same pane: it
    // takes the child's parent with the rest (PRD letters-missions-simplify D-24).
    await takeOverSession(fixture, live, child, "fixture-another-agent");
    await parentOf(child, parent);
    await expect(row(child)).toHaveCount(0);

    // The parent delegates again: the core records the relationship for the session the new child runs.
    const second = await spawnChild(fixture, live, parent, { name: "second-child", repo: path.join(fixture.root, "fixture") });
    await expect(row(second)).toHaveCount(0, { timeout: 20_000 });

    // The parent pane is taken over: the agent after it keeps its children.
    await takeOverSession(fixture, live, parent, "fixture-another-parent");
    await parentOf(second, parent);
    await expect(row(second)).toHaveCount(0);
    await expect(row(parent).locator(`[data-tree-chevron="${parent}"]`).first()).toBeVisible();
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
