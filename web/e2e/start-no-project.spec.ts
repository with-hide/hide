// The start panel with no project registered (PRD retire-hide-home B1): Hide
// keeps no Home to start in, so the panel offers Add a project in place of its
// start control, and that opens the Add a project dialog.

import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { asFirstRun, startHided, type Daemon } from "./hided-fixture";
import { screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });

test("the start panel with no project offers Add a project and opens its dialog", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await asFirstRun(await startHided(fixture, "start-no-project"));
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    await page.keyboard.press(chord("search"));
    await page.keyboard.type("Start an agent");
    await page.locator('[data-palette-row="command:start-agent"]').click();
    const panel = page.locator("[data-start-panel]");
    await expect(panel).toBeVisible();
    // No Home target and no start control: the panel offers Add a project instead.
    await expect(panel.locator("[data-start-submit]")).toHaveCount(0);
    await expect(panel.locator("[data-start-target-option^='home:']")).toHaveCount(0);
    await screenshot(page, "start-panel-no-project");
    await panel.locator("[data-start-add-project]").click();
    await expect(page.locator("[data-add-project]")).toBeVisible();
    await screenshot(page, "start-panel-no-project-add-dialog");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
