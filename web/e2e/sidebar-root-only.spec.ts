// A worktree whose only agents are delegated children is not a line of the
// sidebar (Session UI, Project ordering and inactive folding), on an isolated
// pinned Herdr and hided: the project's root agent sits on its main checkout
// with one child in worktree `kid`, and a second root works in worktree
// `own`. `own` keeps its checkout line; `kid` has none, and its child is
// reached from the root's tree, which wears the descendant mark.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setFixtureLifecycle, spawnAgent, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";

test.describe.configure({ timeout: 180_000 });

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "commit.gpgsign=false", "-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", ...args], { cwd, stdio: "ignore" });
}

test("a worktree with only a delegated child draws no checkout line, and the root's tree reaches the child", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon: Daemon | null = null;
  try {
    const main = path.join(herdr.root, "proj");
    const kid = path.join(herdr.root, "proj-kid");
    const own = path.join(herdr.root, "proj-own");
    fs.mkdirSync(main);
    git(main, ["init"]);
    git(main, ["commit", "--allow-empty", "-m", "initial"]);
    for (const [branch, dir] of [["kid", kid], ["own", own]]) {
      git(main, ["worktree", "add", "-b", branch, dir]);
      git(dir, ["commit", "--allow-empty", "-m", `${branch} work`]);
    }
    const root = await spawnAgent(herdr, "proj", null, main, { task: "프로젝트 조율하기" });
    const child = await spawnAgent(herdr, "proj-kid", root, kid, { task: "자식 작업하기" });
    const second = await spawnAgent(herdr, "proj-own", null, own, { task: "독립 작업하기" });
    for (const pane of [root, child, second]) await setFixtureLifecycle(herdr, pane, "working");
    daemon = await startHided(herdr, "sidebar-root-only");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^proj/ }) });
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
    herdr.stop();
  }
});
