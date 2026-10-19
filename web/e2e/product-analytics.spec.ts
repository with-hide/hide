import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test("product usage defaults on and an opt-out stays off after a daemon restart @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "product-analytics");
    expect(fs.existsSync(path.join(daemon.home, ".hide", "product-analytics-disabled"))).toBe(true);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="privacy"]').click();
    const toggle = page.locator("[data-product-analytics]");
    await expect(toggle).toBeChecked();
    await expect(page.locator('[data-settings-group="privacy"]')).toContainText("No conversations or code are sent.");
    await toggle.click();
    await expect(toggle).not.toBeChecked();
    const saved = path.join(daemon.stateDir, "core-state.json");
    await expect.poll(() => JSON.parse(fs.readFileSync(saved, "utf8")).product_analytics_enabled).toBe(false);
    await screenshot(page, "product-analytics-off");
    daemon = await daemon.restart();
    // A hash-only navigation retains the old socket and its rotated token.
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="privacy"]').click();
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
    await toggle.click();
    await expect(toggle).toBeChecked();
    await expect.poll(() => JSON.parse(fs.readFileSync(saved, "utf8")).product_analytics_enabled).toBe(true);
    // Fixture homes never start the external transport, even in packaged builds.
    expect(fs.existsSync(path.join(daemon.stateDir, "product-analytics-id"))).toBe(false);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
