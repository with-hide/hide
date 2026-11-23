// The phone settings' steps a spec takes, on the screen that holds them since
// Settings › Mobile folded into Settings › Devices (UI_BEHAVIOR.md, Settings ›
// Devices' phones): the phone group, its connection window with the direct
// Tailscale way under Advanced, and the QR window Add device › Phone opens.
import { expect, type Locator, type Page } from "@playwright/test";
import { chord } from "./chords";

/** Opens Settings on Devices and returns its phone group. */
export async function openPhoneGroup(page: Page): Promise<Locator> {
  await page.keyboard.press(chord("settings"));
  await page.locator('[data-settings-tab="devices"]').click();
  const group = page.locator('[data-mobile-tab="true"]');
  await expect(group).toBeVisible();
  return group;
}

/** Opens Phone connection, the window that holds the relay's state and, under Advanced, Tailscale and its checks. */
export async function openPhoneConnection(page: Page): Promise<Locator> {
  await page.locator("[data-mobile-method-open]").click();
  const dialog = page.locator("[data-mobile-method-dialog]");
  await expect(dialog).toBeVisible();
  await dialog.getByText("Advanced · Connect directly with Tailscale", { exact: true }).click();
  return dialog;
}

/** Chooses the direct Tailscale way in Phone connection and closes the window. */
export async function chooseDirect(page: Page): Promise<void> {
  const dialog = await openPhoneConnection(page);
  const direct = dialog.getByRole("switch", { name: "Use Tailscale", exact: true });
  if ((await direct.getAttribute("aria-checked")) === "false") await direct.click();
  await expect(direct).toHaveAttribute("aria-checked", "true");
  await closeLayer(page, dialog);
}

/** Add device › Phone: the QR window, which turns phone access on and makes one code once the way is ready. */
export async function openPhoneQr(page: Page): Promise<void> {
  await page.locator('[data-device-add-open="true"]').click();
  await page.locator('[data-add-choice="phone"]').click();
  await expect(page.locator("[data-mobile-pair-dialog]")).toBeVisible();
}

/** Closes the QR window, which takes its code out of the page. */
export async function closePhoneQr(page: Page): Promise<void> {
  await closeLayer(page, page.locator("[data-mobile-pair-dialog]"));
}

/** ⋯ › Disconnect… on the phone named by `id`, or the only one, confirmed. */
export async function disconnectPhone(page: Page, id?: string): Promise<void> {
  await page.locator(id ? `[data-mobile-phone-menu="${id}"]` : "[data-mobile-phone-menu]").click();
  await page.locator("[data-mobile-revoke-menu]").click();
  await page.locator("[data-mobile-revoke]").click();
}

async function closeLayer(page: Page, layer: Locator): Promise<void> {
  await page.keyboard.press("Escape");
  await expect(layer).toHaveCount(0);
}
