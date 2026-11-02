// A worktree whose only agents are delegated children is not a line of the
// sidebar (Session UI, Project ordering and inactive folding), on either
// engine: the project's root agent sits on its main checkout with one child
// in worktree `kid`, and a second root works in worktree `own`. `own` keeps
// its checkout line; `kid` has none, and its child is reached from the root's
// tree, which wears the descendant mark.

import { expect, test } from "@playwright/test";
import path from "node:path";
import { focusPane, viewPane, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { agentReading, setAgentState, spawnChild, spawnRoot } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 180_000 });

test("a worktree with only a delegated child draws no checkout line, and the root's tree reaches the child", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "sidebar-root-only");
    const main = path.join(fixture.root, "fixture");
    const [root] = fixture.panes;
    // The root delegates into a worktree of its own, and hands other work to the operator in a second.
    const child = await spawnChild(fixture, daemon, root, { name: "fixture-kid", repo: main, branch: "kid" });
    const second = await spawnRoot(fixture, daemon, root, { name: "fixture-own", repo: main, branch: "own" });
    for (const pane of [child, second]) await agentReading(fixture, pane);
    // The keyboard goes to a pane before it is typed into, as the operator clicks it: a tab nobody shows keeps no terminal for keys.
    for (const pane of [child, second, root]) {
      await withControl(daemon, async (control) => {
        if (pane !== root) viewPane(control, pane);
        await focusPane(control, pane);
      });
      await setAgentState(daemon, pane, "working");
    }
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^fixture/ }) });
    const checkout = (branch: string) => project.locator(`[data-checkout][aria-label^="${branch}"]`);
    await expect(checkout("own")).toBeVisible({ timeout: 30_000 });
    await expect(project.locator(`[data-pane="${root}"]`).first()).toBeVisible();
    await expect(checkout("kid")).toHaveCount(0);
    // Not moved into the agentless fold either: its agent is working.
    await expect(project.getByRole("button", { name: /^Worktrees with no agents/ })).toHaveCount(0);
    await screenshot(page, "sidebar-root-only");

    // The way to the child is the root's tree.
    const disclosure = project.locator(`[data-tree-chevron="${root}"]`).first();
    const sidebar = page.locator("nav[data-sidebar] > .sidebar-header-container");
    for (const width of [240, 292]) {
      await sidebar.evaluate((node, size) => { (node as HTMLElement).style.width = `${size}px`; (node as HTMLElement).style.flex = "none"; }, width);
      await page.evaluate(() => document.fonts.ready);
      const bounds = (await sidebar.boundingBox())!;
      const affordance = (await disclosure.boundingBox())!;
      console.info("singleton lineage fold bounds", JSON.stringify({ width, sidebar: bounds, disclosure: affordance }));
      expect(affordance.x).toBeGreaterThanOrEqual(bounds.x);
      expect(affordance.x + affordance.width).toBeLessThanOrEqual(bounds.x + bounds.width);
      await expect(disclosure).toBeVisible();
      await disclosure.click();
      await expect(page.locator(`nav[data-sidebar] [data-pane="${child}"]`).first()).toBeVisible();
      await disclosure.click();
      await expect(page.locator(`nav[data-sidebar] [data-pane="${child}"]`)).toHaveCount(0);
    }
    await disclosure.click();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${child}"]`).first()).toBeVisible();
    await screenshot(page, "sidebar-root-only-tree");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
