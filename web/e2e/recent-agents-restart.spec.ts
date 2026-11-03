// The Agent area's ⌥` order outlives a restart (issue 301, docs/UI_BEHAVIOR.md,
// Recent navigation): the page reports each pane visit and the core saves the
// order in `core-state.json`, so after hided restarts and the page reloads the
// cycle still offers an agent pane visited only before the restart.

import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { agentIn, openProject, sendKeys, withControl } from "./daemon-actions";
import { fixtureExecutable } from "./platform-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showProjectRow } from "./wire";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

test("the recent agent pane order survives a daemon restart and a reload", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    fs.mkdirSync(path.join(fixture.root, "beta"), { recursive: true });
    fs.copyFileSync(path.join(fixture.root, "bin", fixtureExecutable("claude")), path.join(fixture.root, "bin", fixtureExecutable("codex")));
    daemon = await startHided(fixture, "recent-agents-restart");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const canvas = page.locator("[data-canvas]").first();

    // A second project whose shell runs the Codex stand-in, typed at its
    // prompt once the page draws it: a pane takes the operator's keys only
    // while a screen shows it.
    const beta = await withControl(daemon, (control) => openProject(control, path.join(fixture.root, "beta"), "beta"));
    const betaPane = beta.pane;
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "beta" }).locator("[data-checkout]").first())).click();
    await expect(canvas).toHaveAttribute("data-canvas", beta.tab);
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", betaPane), { timeout: 20_000, message: "the prompt in beta" }).toContain("fixture %");
    await withControl(daemon, async (control) => {
      sendKeys(control, betaPane, "codex\r");
      await control.until((rest) => agentIn(rest, betaPane), "the Codex agent in beta", 30_000);
    });

    // The keyboard in beta's Codex, then back in the fixture's pane.
    await page.locator(`[data-pane-view="${betaPane}"]`).click();
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "fixture" }).locator("[data-checkout]").first())).click();
    await expect(canvas).not.toHaveAttribute("data-canvas", beta.tab);
    const fixturePane = (await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"))!;
    await page.locator(`[data-pane-view="${fixturePane}"]`).click({ position: { x: 30, y: 60 } });
    const stored = () => (JSON.parse(fs.readFileSync(path.join(daemon!.stateDir, "core-state.json"), "utf8")) as { recent_pane_ids?: string[] }).recent_pane_ids;
    await expect.poll(stored).toEqual([fixturePane, betaPane]);

    daemon = await daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // From a sidebar control, which supplies no pane visit, the cycle starts
    // at the most recent agent pane and still reaches beta's, visited only
    // before the restart.
    const sidebarOverview = page.locator('[data-sidebar-overview]');
    await sidebarOverview.focus();
    await expect(sidebarOverview).toBeFocused();
    const agents = page.locator("[data-cycle=agents]");
    const cycleRow = page.locator("[data-cycle] [aria-selected=true]");
    await page.keyboard.down("Alt");
    await page.keyboard.press("Backquote");
    await expect(agents).toBeVisible();
    await expect(agents.locator("[role=option]")).toHaveCount(2);
    await expect(cycleRow).toHaveAttribute("data-cycle-row", fixturePane);
    await page.keyboard.press("Backquote");
    await expect(cycleRow).toHaveAttribute("data-cycle-row", betaPane);
    await expect(cycleRow).toHaveAttribute("aria-label", /Codex agent/);
    await screenshot(page, "recent-agents-after-restart");
    await page.keyboard.up("Alt");
    await expect(canvas).toHaveAttribute("data-canvas", beta.tab);
    await expect.poll(stored).toEqual([betaPane, fixturePane]);
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
