// Settings › Devices' phones on an isolated engine and a debug hided (PRD
// settings-cleanup B57, B58, B59; operator review 2026-11-23): the Tailscale
// checks fold into one line when they all pass, opening the page makes no
// pairing code and the QR window Add device › Phone opens makes one, and the
// phone pause switch lives on Notifications. The paired phone, push delivery
// and revoke are `mobile.spec.ts`'s.

import { expect, test } from "@playwright/test";
import { FakeTailscale } from "./fake-tailscale";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { closePhoneQr, openPhoneConnection, openPhoneGroup, openPhoneQr } from "./phone-settings";

test.describe.configure({ timeout: 120_000 });

test("a ready Mac is one line, the QR waits for the QR window, and the phone pause switch lives on Notifications", async ({ page }) => {
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
    await openPhoneGroup(page);

    // B58: opening the page asked Tailscale nothing and made no code.
    await expect(page.locator("[data-mobile-pairing], [data-mobile-qr]")).toHaveCount(0);
    expect(tailscale.calls()).toBe("");

    // Direct connection is an explicit choice under Phone connection › Advanced; choosing it still makes no code.
    const connection = await openPhoneConnection(page);
    const direct = connection.getByRole("switch", { name: "Use Tailscale", exact: true });
    await direct.click();
    await expect(direct).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
    await expect(connection).toHaveCount(0);

    // B57: every check passes, so one line stands where the steps were, and no step shows.
    await page.locator('[data-mobile-switch="true"]').click();
    const checks = await openPhoneConnection(page);
    await expect(checks.locator('[data-mobile-ready="true"]')).toContainText("Tailscale is ready", { timeout: 20_000 });
    await expect(page.locator("[data-mobile-step]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(checks).toHaveCount(0);

    // B58: still no code until the QR window; opening it shows one with its countdown, closing it takes it away.
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);
    await openPhoneQr(page);
    await expect(page.locator("[data-mobile-qr]")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("[data-mobile-countdown]")).toHaveText(/Code expires in [45]:\d\d/);
    await closePhoneQr(page);
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);

    // Phone delivery pauses on the desktop; the phone owns whether it receives notifications.
    await expect(page.locator("[data-phone-push-rest]")).toHaveCount(0);
    await page.locator('[data-settings-tab="notifications"]').click();
    const rest = page.locator("[data-phone-push-rest]");
    await expect(rest).toHaveAttribute("aria-checked", "true");
    await expect(page.getByText("No phone notifications while you use your computer. After 10 minutes away, or when the screen locks, they go to your phone.", { exact: true })).toBeVisible();
    await expect(page.getByText("Choose whether to receive notifications in the phone app settings.", { exact: true })).toBeVisible();
    await rest.click();
    await expect(rest).toHaveAttribute("aria-checked", "false");

  } finally {
    daemon?.stop();
    engine.stop();
    tailscale.remove();
  }
});
