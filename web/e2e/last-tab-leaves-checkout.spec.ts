import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { tabsOf, withControl } from "./daemon-actions";
import { waitForTerminal } from "./group-start-helper";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, openOtherCheckouts, screenshot } from "./wire";
import { chord } from "./chords";
import { quietFor } from "./wait";

// docs/UI_BEHAVIOR.md, Agent areas: closing the last tab of the checkout in
// front leaves it for the checkout the operator was in before, and the engine
// letting go of that checkout afterwards does not move the screen.


test("Closing a worktree's last tab goes back to the checkout it was entered from", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1400, height: 900 });
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(fixture.root, "fixture");
    const linked = path.join(fixture.root, "fixture-feature");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: { ...process.env, GIT_CONFIG_GLOBAL: os.devNull, GIT_CONFIG_NOSYSTEM: "1" } });
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q", "-b", "main"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", "commit", "-qm", "Initial files");
    git("worktree", "add", "-q", "-b", "feature", linked);

    daemon = await startHided(fixture, "last-tab-leaves-checkout");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const location = page.locator("[data-workspace-location]");
    await expect(location).toHaveText("main");

    // The operator opens a draft in the worktree, then explicitly starts
    // its terminal, opening the bound workspace in the same action.
    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^fixture/ }) });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator('[data-checkout][aria-label^="feature"]') });
    // A clean worktree with no agent waits in the project's Checkouts fold.
    await openOtherCheckouts(project);
    const tabsBefore = await withControl(daemon, (control) => control.until((rest) => tabsOf(rest, fixture.workspace), "the fixture's tabs")) as string[];
    await feature.locator("[data-checkout-menu]").click({ button: "right" });
    await page.getByRole("menu", { name: "feature actions" }).locator('[data-menu-item="new_tab_here"]').click();
    const featureTab = await withControl(daemon, (control) => control.until((rest) => tabsOf(rest, fixture.workspace)?.find((id) => !tabsBefore.includes(id)), "the worktree's new tab", 20_000)) as string;
    await waitForTerminal(page, featureTab);
    await expect(location).toHaveText("feature", { timeout: 15_000 });
    const tabs = page.locator('[data-agent-tab-bar] [role="tab"]');
    await expect(tabs).toHaveCount(1);

    await page.locator("[data-terminal-host]").first().click();
    await page.keyboard.press(chord("close_tab"));
    await expect(location).toHaveText("main");
    await expect(page.locator("[data-area-empty]")).toHaveCount(0);
    await expect(page.locator("[data-terminal-host]").first()).toBeVisible();

    await quietFor(page, 1_000, "the engine's own focus after the close does not move the screen");
    await expect(location).toHaveText("main");
    await screenshot(page, "last-tab-leaves-checkout");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
