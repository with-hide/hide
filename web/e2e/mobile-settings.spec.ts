// Settings > Mobile on an isolated pinned Herdr and a debug hided (PRD
// settings-cleanup B57, B58, B59): the Tailscale checks fold into one line when
// they all pass, opening the tab makes no pairing code and Show QR makes one,
// and the push mode is one dropdown that describes only the mode it shows. The
// paired phone, push delivery and revoke are `mobile.spec.ts`'s.

import { expect, test } from "@playwright/test";
import { FakeTailscale } from "./fake-tailscale";
import { chord } from "./chords";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 120_000 });

test("a ready Mac is one line, the QR waits for Show QR, and the push dropdown on Notifications describes one mode", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  const herdr = await startHerdr({ readiness: "registered" });
  const tailscale = new FakeTailscale();
  let daemon: Daemon | null = null;
  try {
    tailscale.install();
    tailscale.ready();
    daemon = await startHided(herdr, "mobile-settings", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-sidebar-overview]")).toBeVisible({ timeout: 20_000 });
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="mobile"]').click();
    const tab = page.locator('[data-mobile-tab="true"]');
    await expect(tab).toBeVisible();

    // B58: opening the tab asked Tailscale nothing and made no code.
    await expect(page.locator('[data-mobile-pairing]')).toHaveCount(0);
    expect(tailscale.calls()).toBe("");

    // B57: every check passes, so one line stands where the steps were, and no step shows.
    await page.locator('[data-mobile-switch="true"]').click();
    await expect(page.locator('[data-mobile-ready="true"]')).toContainText("Tailscale is ready", { timeout: 20_000 });
    await expect(page.locator("[data-mobile-step]")).toHaveCount(0);

    // B58: still no code until Show QR; pressing it shows one with its countdown, Hide QR takes it away.
    await expect(page.locator('[data-mobile-pairing="idle"]')).toBeVisible();
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);
    await page.locator('[data-mobile-show-code="true"]').click();
    await expect(page.locator("[data-mobile-qr]")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("[data-mobile-countdown]")).toHaveText(/Code expires in [45]:\d\d/);
    await page.locator('[data-mobile-hide-code="true"]').click();
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);

    // B59: the push mode is one dropdown on Notifications, not Mobile (PRD settings-left-nav B6, B7), and only the chosen mode's description shows.
    await expect(page.locator("[data-push-select]")).toHaveCount(0);
    await page.locator('[data-settings-tab="notifications"]').click();
    await expect(page.locator("[data-push-select]")).toHaveText("Off");
    await expect(page.locator("[data-push-detail]")).toHaveCount(1);
    await expect(page.locator('[data-push-detail="off"]')).toBeVisible();
    await page.locator("[data-push-select]").click();
    await expect(page.locator("[data-push-choice]")).toHaveCount(3);
    await page.locator('[data-push-choice="always"]').click();
    await expect(page.locator('[data-push-detail="always"]')).toBeVisible();
    await expect(page.locator("[data-push-detail]")).toHaveCount(1);
  } finally {
    daemon?.stop();
    herdr.stop();
    tailscale.remove();
  }
});
