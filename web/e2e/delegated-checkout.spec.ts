// A checkout whose only tab holds a delegated child, on either engine
// (PRD hide-orchestrator B1, B4): the tab stays off the strip,
// but the checkout is not empty. Opening the checkout, or its agent row in the
// sidebar, draws the child's pane rather than the "No agent tab" state that a
// checkout with no tab shows.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { tabOfPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { spawnChild } from "./fixture-agents";
import { git } from "./hide-ai-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, projectOpener, screenshot, showProjectRow } from "./wire";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

test("a checkout whose only tab is delegated opens on the child's pane", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "delegated-checkout");
    const [parent] = fixture.panes;
    // A second checkout, its agent spawned by the fixture's first agent: the
    // way an Observer dispatches an Implementor into a checkout of its own.
    const beta = path.join(fixture.root, "beta");
    fs.mkdirSync(beta, { recursive: true });
    git(beta, "init", "-q", "-b", "main");
    git(beta, "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "first");
    const child = await spawnChild(fixture, daemon, parent, { name: "three", repo: fs.realpathSync.native(beta) });
    const childTab = await withControl(daemon, (control) => control.until((rest) => tabOfPane(rest, child), `the tab of ${child}`));
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // The checkout row opens the Workspace on the child's pane: the strip
    // has no tab for it (B1), and the empty state is not shown.
    const checkout = await showProjectRow(page, projectOpener(page.locator("[data-project]", { hasText: "beta" })));
    await checkout.click();
    await expect(page.locator("[data-canvas]").first()).toHaveAttribute("data-canvas", childTab, { timeout: 20_000 });
    await expect(page.locator(`[data-pane-view="${child}"]`)).toBeVisible();
    await expect(page.locator("[data-empty-new-tab]")).toHaveCount(0);
    await expect(page.locator('[data-agent-tab-bar] [role="tab"]')).toHaveCount(0);
    await screenshot(page, "delegated-checkout-open");

    // Session UI B1/B38: lineage belongs to the pane header and the root's tree.
    // The child's header names the path back to its root (agent-hierarchy-screens B20).
    const raisedArrow = page.locator(`[data-pane-view="${child}"] [data-pane-path="${child}"] [data-pane-return="${parent}"]`);
    await expect(raisedArrow).toBeVisible();
    await screenshot(page, "delegated-checkout-raised-from");

    // Back on the first checkout, the root's chevron opens its child in place (agent-hierarchy-screens B10).
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "fixture" }).locator("[data-checkout]").first())).click({ position: { x: 12, y: 8 } });
    await expect(page.locator(`[data-pane-view="${parent}"]`)).toBeVisible({ timeout: 20_000 });
    await page.locator(`nav[data-sidebar] [data-pane="${parent}"] [data-tree-chevron="${parent}"]`).first().click();
    await page.locator(`nav[data-sidebar] [data-pane="${child}"] [data-agent-open="${child}"]`).first().click();
    await expect.poll(() => last.get("focus_pane")?.pane_id).toBe(child);
    await expect(page.locator(`[data-pane-view="${child}"]`)).toHaveAttribute("data-focused", "true", { timeout: 20_000 });
    await expect(page.locator("[data-empty-new-tab]")).toHaveCount(0);
    await screenshot(page, "delegated-checkout-row");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
