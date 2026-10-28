import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { createdTabs, herdrTabIds, waitForTerminal } from "./group-start-helper";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { quietFor } from "./wait";

// docs/UI_BEHAVIOR.md, Agent areas: closing the last tab of the checkout in
// front leaves it for the checkout the operator was in before, and Herdr
// closing that checkout's workspace afterwards does not move the screen.

type Workspace = { workspace_id: string; worktree?: { checkout_path: string } | null };

test("Closing a worktree's last tab goes back to the checkout it was entered from", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    const linked = path.join(herdr.root, "fixture-feature");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env });
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "commit", "-qm", "Initial files");
    git("worktree", "add", "-q", "-b", "feature", linked);
    const boundToLinked = () =>
      (herdr.run(["workspace", "list"]) as { result: { workspaces: Workspace[] } }).result.workspaces
        .filter((workspace) => workspace.worktree && fs.realpathSync(workspace.worktree.checkout_path) === fs.realpathSync(linked));

    daemon = await startHided(herdr, "last-tab-leaves-checkout");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const location = page.locator("[data-workspace-location]");
    await expect(location).toHaveText("main");

    // The operator opens a draft in the worktree, then explicitly starts
    // its terminal, opening the bound Herdr workspace in the same action.
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^fixture/ }) });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="feature"]') });
    await project.getByRole("button", { name: /^Worktrees with no agents/ }).click();
    const tabsBefore = herdrTabIds(herdr);
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await page.getByRole("menu", { name: "feature actions" }).locator('[data-menu-item="new_tab_here"]').click();
    const [featureTab] = await createdTabs(herdr, tabsBefore);
    await waitForTerminal(page, featureTab!);
    await expect.poll(() => boundToLinked().length, { timeout: 20_000 }).toBe(1);
    await expect(location).toHaveText("feature", { timeout: 15_000 });
    const tabs = page.locator('[data-agent-tab-bar] [role="tab"]');
    await expect(tabs).toHaveCount(1);

    await page.locator("[data-terminal-host]").first().click();
    await page.keyboard.press(chord("close_tab"));
    await expect(location).toHaveText("main");
    await expect(page.locator("[data-area-empty]")).toHaveCount(0);
    await expect(page.locator("[data-terminal-host]").first()).toBeVisible();

    // Herdr then closes the worktree's workspace and focuses one of its own.
    await expect.poll(() => boundToLinked().length, { timeout: 20_000 }).toBe(0);
    await quietFor(page, 1_000, "Herdr's own focus after the close does not move the screen");
    await expect(location).toHaveText("main");
    await screenshot(page, "last-tab-leaves-checkout");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
