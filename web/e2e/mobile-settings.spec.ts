// Settings > Mobile on an isolated engine and a debug hided (PRD
// settings-cleanup B57, B58, B59): the Tailscale checks fold into one line when
// they all pass, opening the tab makes no pairing code and Show QR makes one,
// and the push mode is one dropdown that describes only the mode it shows. The
// paired phone, push delivery and revoke are `mobile.spec.ts`'s.

import { expect, test } from "@playwright/test";
import { FakeTailscale } from "./fake-tailscale";
import { chord } from "./chords";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 120_000 });

test("a ready Mac is one line, the QR waits for Show QR, and the phone pause switch lives on Notifications", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  const engine = await startFixture();
  const tailscale = new FakeTailscale();
  let daemon: Daemon | null = null;
  try {
    tailscale.install();
    tailscale.ready();
    daemon = await startHided(engine, "mobile-settings", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-sidebar-overview]")).toBeVisible({ timeout: 20_000 });
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="mobile"]').click();
    const tab = page.locator('[data-mobile-tab="true"]');
    await expect(tab).toBeVisible();

    // B58: opening the tab asked Tailscale nothing and made no code.
    await expect(page.locator('[data-mobile-pairing]')).toHaveCount(0);
    expect(tailscale.calls()).toBe("");

    // Direct connection is an explicit choice under Advanced; opening it still makes no code.
    await tab.getByText("Advanced · Connect directly with Tailscale", { exact: true }).click();
    await tab.getByRole("switch", { name: "Use Tailscale", exact: true }).click();
    await expect(tab.getByRole("switch", { name: "Use Tailscale", exact: true })).toHaveAttribute("aria-checked", "true");

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

    // Phone delivery pauses on the desktop; the phone owns whether it receives notifications.
    await expect(page.locator("[data-phone-push-rest]")).toHaveCount(0);
    await page.locator('[data-settings-tab="notifications"]').click();
    const rest = page.locator("[data-phone-push-rest]");
    await expect(rest).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText("No phone notifications while desktop Hide is open.", { exact: true })).toBeVisible();
    await expect(page.getByText("Choose whether to receive notifications in the phone app settings.", { exact: true })).toBeVisible();
    await rest.click();
    await expect(rest).toHaveAttribute("aria-checked", "false");

  } finally {
    daemon?.stop();
    engine.stop();
    tailscale.remove();
  }
});
