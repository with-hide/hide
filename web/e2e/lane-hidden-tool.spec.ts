import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fixtureExecutable } from "./platform-fixture";
import { runInPane, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showDocuments, showExplorer } from "./wire";

// Folded documents stay mounted in the viewing column, out of sight (PRD
// lane-two-columns B3). Nothing drawn in them shows through: a selected view
// tab draws its close button visible on its own, and `visibility` alone on
// the hidden wrapper let that button show over the tool beside it (seen
// 2026-10-16).

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

test("folded documents kept behind the tool column show nothing through them", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(herdr.root, "fixture");
    fs.writeFileSync(path.join(fixture, "a.md"), "a\n");
    daemon = await startHided(herdr, "lane-hidden-tool");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    const hide = path.resolve("..", "target", "debug", fixtureExecutable("hide"));
    const ran = await runInPane(herdr, herdr.panes[0], "cli-1", { env: { HIDE_STATE_DIR: daemon.stateDir }, argv: [hide, "file", "open", "a.md"] });
    expect(ran.status, ran.stderr).toBe(0);
    const selected = page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
    await expect(selected).toHaveAttribute("aria-label", /\/a\.md/);
    const close = selected.locator("button[aria-label]").last();
    await expect(close).toBeVisible();

    await page.locator('[data-lane-tool="documents"]').click();
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "none");
    const hidden = page.locator("[data-lane-frame] [data-lane-hidden]");
    await expect(hidden).toHaveAttribute("inert", "");
    // The button keeps its own visibility, so the wrapper's opacity is what hides it.
    await expect(close).toHaveCSS("visibility", "visible");
    await expect(hidden).toHaveCSS("visibility", "hidden");
    await expect(hidden).toHaveCSS("opacity", "0");
    await screenshot(page, "lane-hidden-documents");

    await showDocuments(page);
    await expect(close).toBeVisible();
    await expect(close).toHaveCSS("opacity", "1");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
