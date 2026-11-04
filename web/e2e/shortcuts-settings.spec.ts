// Settings, Shortcuts on an isolated pinned Herdr and hided (PRD settings-cleanup
// B60-B63, B66): the chip is the control and a chord applies as it is pressed,
// Escape cancels, restore and clear show on hover or keyboard focus, the eight
// area commands fold into one line, and the sheet no longer carries the
// macOS-app-only row or its explanations.

import { expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { label } from "./chords";

test.describe.configure({ timeout: 120_000 });

test("a chord chip rebinds as the chord is pressed, and the area commands fold into one line", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  const herdr = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "shortcuts-settings");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="shortcuts"]').click();
    const sheet = page.locator("[data-settings]");
    const effective = (id: string) => page.locator(`[data-shortcut-effective="${id}"]`);

    // B60, B63: no macOS-app-only row, and the renamed commands.
    await expect(sheet).not.toContainText("Toggle Conversation");
    await expect(sheet).not.toContainText("macOS app");
    await expect(sheet).toContainText("Next recent tab");
    await expect(sheet).toContainText("Next panel (all projects)");

    // B61: a click on the chip waits for a chord, which applies without a second step.
    const original = await effective("split_right").textContent();
    const chip = page.locator('[data-shortcut-record="split_right"]');
    await chip.click();
    await expect(chip).toHaveAttribute("aria-label", /Recording a chord for Split right/);
    await page.keyboard.press("Alt+KeyR");
    await expect(effective("split_right")).toHaveText(label({ code: "KeyR", alt: true }));
    expect(Object.keys((last.get("ui_state_update")?.browser_shortcut_bindings as Record<string, string>) ?? {})).toContain("split_right");
    await expect(chip).toHaveAttribute("aria-label", `Change the shortcut for Split right, now ${label({ code: "KeyR", alt: true })}`);

    // Escape cancels the recording and leaves the sheet open.
    await chip.press("Enter");
    await expect(chip).toHaveAttribute("aria-label", /Recording a chord/);
    await page.keyboard.press("Escape");
    await expect(effective("split_right")).toHaveText(label({ code: "KeyR", alt: true }));
    await expect(sheet).toBeVisible();

    // The restore control reads as hidden until the row is hovered or focused.
    const reset = page.locator('[data-shortcut-reset="split_right"]');
    const clear = page.locator('[data-shortcut-clear="overview"]');
    await page.locator("[data-settings] header").hover();
    await expect(clear).toHaveCSS("opacity", "0");
    await page.locator('[data-shortcut-record="overview"]').hover();
    await expect(clear).toHaveCSS("opacity", "1");
    await reset.click();
    await expect(effective("split_right")).toHaveText(original!);

    // B62: eight area commands fold into one line that opens to bind each.
    const fold = page.locator('[data-settings-fold="area-commands"]');
    await expect(fold).toContainText("8 without a shortcut");
    await expect(fold).not.toHaveAttribute("open", "");
    await fold.locator("summary").click();
    await page.locator('[data-shortcut-record="focus_next_agent_area"]').click();
    await page.keyboard.press("Alt+KeyJ");
    await expect(effective("focus_next_agent_area")).toHaveText(label({ code: "KeyJ", alt: true }));
    await expect(fold).toContainText("7 without a shortcut");

    // A browser tab has no numbered chords, so the fixed rows have nothing to hold.
    const fixed = page.locator('[data-settings-group="numbered-chords"]');
    await expect(fixed).toContainText("not on this host");
    await expect(fixed).not.toContainText("Hold");
    await screenshot(page, "settings-shortcuts");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
