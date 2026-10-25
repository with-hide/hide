// A relationship holds only while both panes still host the sessions it was
// written for, on an isolated pinned Herdr and hided: the tokens hided writes
// outlive the agent that earned them (they die with the pane), so an agent
// that takes over a pane must not inherit its parent, and a parent pane taken
// over by another agent must not adopt the old children.

import { expect, test, type Page } from "@playwright/test";
import { declareParent, setFixtureSession, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
}

test("a pane taken over by another agent is a root, and so is a child of a parent pane that was", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | null = null;
  try {
    const [parent, child] = herdr.panes;
    daemon = await startHided(herdr, "lineage-session");
    await open(page, daemon);
    const row = (pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`);
    await expect(row(parent)).toBeVisible({ timeout: 20_000 });
    await expect(row(child)).toBeVisible({ timeout: 20_000 });

    // Declared, the child is delegated: folded under its parent, not a row of its own.
    declareParent(herdr, child, parent);
    await expect(row(child)).toHaveCount(0, { timeout: 20_000 });

    // The child's agent ends and a different one starts in the same pane. Its
    // tokens are still there, and it is the operator's own work again.
    setFixtureSession(herdr, child, "fixture-another-agent");
    await expect(row(child)).toBeVisible({ timeout: 20_000 });
    await expect(row(child)).not.toHaveAttribute("data-delegated", "true");

    // hided writes the relationship again for the session the pane runs now.
    declareParent(herdr, child, parent);
    await expect(row(child)).toHaveCount(0, { timeout: 20_000 });

    // The parent pane is taken over: it adopts none of its old children.
    setFixtureSession(herdr, parent, "fixture-another-parent");
    await expect(row(child)).toBeVisible({ timeout: 20_000 });
    await expect(row(parent).locator(`[data-tree-chevron="${parent}"]`)).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
