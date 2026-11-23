// A phone paired with a fixture daemon over the direct connection's fake-Tailscale boundary, never the operator's
// service: the desktop's Settings shows the code, and an iPhone-sized page opens its pairing address on the daemon.
import { devices, expect, type Browser, type BrowserContext, type Page } from "@playwright/test";
import { chooseDirect, openPhoneGroup, openPhoneQr } from "./phone-settings";
import type { Daemon } from "./hided-fixture";

/** Pairs a new iPhone 13 page with the daemon; the caller closes the returned context. */
export async function pairDirectPhone(browser: Browser, daemon: Daemon): Promise<{ context: BrowserContext; phone: Page }> {
  const settings = await browser.newPage();
  const context = await browser.newContext({ ...devices["iPhone 13"], locale: "en-US", colorScheme: "dark" });
  try {
    await settings.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(settings.locator("[data-sidebar-overview]")).toBeVisible();
    await openPhoneGroup(settings);
    await chooseDirect(settings);
    await openPhoneQr(settings);
    const qr = settings.locator("[data-mobile-qr]");
    await expect(qr).toBeVisible();
    const address = (await qr.getAttribute("data-mobile-qr"))!;
    const phone = await context.newPage();
    await phone.goto(`${daemon.origin}/m/${new URL(address).hash}`);
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible();
    return { context, phone };
  } catch (error) {
    await context.close();
    throw error;
  } finally { await settings.close(); }
}
