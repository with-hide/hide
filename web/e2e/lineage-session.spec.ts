// A relationship holds only while both panes still host the sessions it was
// written for, on either engine: the relationship the core recorded when the
// parent spawned its child outlives the agent that earned it, so an agent that
// takes over a pane must not inherit its parent, and a parent pane taken
// over by another agent must not adopt the old children.

import { expect, test, type Page } from "@playwright/test";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { takeOverSession, spawnChild } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

test("a pane taken over by another agent is a root, and so is a child of a parent pane that was", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "lineage-session");
    const [parent, child] = fixture.panes;
    await open(page, daemon);
    const row = (pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`);
    await expect(row(parent)).toBeVisible({ timeout: 20_000 });

    // Spawned by the parent, the child is delegated: folded under its parent, not a row of its own.
    await expect(row(child)).toHaveCount(0, { timeout: 20_000 });

    // The child's agent ends and a different one starts in the same pane. Its
    // relationship is still recorded, and it is the operator's own work again.
    await takeOverSession(fixture, daemon, child, "fixture-another-agent");
    await expect(row(child)).toBeVisible({ timeout: 20_000 });
    await expect(row(child)).not.toHaveAttribute("data-delegated", "true");

    // The parent delegates again: the core records the relationship for the session the new child runs.
    const second = await spawnChild(fixture, daemon, parent, { name: "second-child", repo: path.join(fixture.root, "fixture") });
    await expect(row(second)).toHaveCount(0, { timeout: 20_000 });

    // The parent pane is taken over: it adopts none of its old children.
    await takeOverSession(fixture, daemon, parent, "fixture-another-parent");
    await expect(row(second)).toBeVisible({ timeout: 20_000 });
    await expect(row(parent).locator(`[data-tree-chevron="${parent}"]`)).toHaveCount(0);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
