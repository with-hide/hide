import { expect, test } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord, held } from "./chords";

test("shared Overview preserves work, returns the keyboard and has direct sidebar commands", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "overview-modal");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    const sidebar = page.locator("[data-sidebar-overview]");
    const modal = page.getByRole("dialog", { name: "Overview", exact: true });
    // With no Workspace in front, where a first run starts, the Overview row keeps All projects a page, not the layer.
    await expect(page.locator("[data-overview-page]")).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('[data-main-tab="work"]')).toHaveAttribute("aria-selected", "true");
    await sidebar.click();
    await expect(page.locator("[data-main-screen]")).toBeVisible();
    await expect(modal).toHaveCount(0);
    await screenshot(page, "overview-page-dark");
    await enterWorkspace(page, "fixture");
    const pane = page.locator('[data-pane-view][data-focused="true"]').first();
    const input = pane.locator(".xterm-helper-textarea");
    await input.focus();
    const paneId = await pane.getAttribute("data-pane-view");
    const geometry = await pane.boundingBox();
    await page.keyboard.press(chord("overview"));
    await expect(modal).toBeVisible();
    // On a 1920 × 1080 window the modal stops at its cap, 1440 × 960.
    expect(await modal.boundingBox().then((box) => [box?.width, box?.height])).toEqual([1440, 960]);
    await expect(sidebar).toHaveAttribute("aria-current", "page");
    expect(await pane.boundingBox()).toEqual(geometry);
    expect(await modal.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    await page.keyboard.press("Tab");
    expect(await modal.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    await screenshot(page, "overview-modal-all-dark");
    await modal.locator('[data-main-tab="projects"]').click();
    await expect(modal.locator("[data-overview-projects]")).toBeVisible();
    await screenshot(page, "overview-modal-projects-dark");
    await page.keyboard.press("Escape");
    await expect(modal).toHaveCount(0);
    await expect(input).toBeFocused();
    expect(await pane.boundingBox()).toEqual(geometry);
    await page.keyboard.type("overview-terminal-preserved");
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id!), paneId)).toContain("overview-terminal-preserved");
    await page.keyboard.press("Control+KeyU");

    // The palette hands Overview its original keyboard owner, not its input.
    await page.keyboard.press(chord("search"));
    await page.locator("[data-palette-input]").fill("fixture");
    await page.keyboard.press(chord("overview"));
    await expect(page.locator("[data-palette]")).toHaveCount(0);
    await expect(modal).toBeVisible();
    await expect(modal.locator("[data-overview-page]")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(input).toBeFocused();

    // Settings prevents a second modal. Its own Escape closes it.
    await page.locator("[data-open-settings]").click();
    await page.keyboard.press(chord("overview"));
    await expect(modal).toHaveCount(0);
    await page.keyboard.press("Escape");

    // Clearing a shortcut persists, removes its keycap and disables its chord.
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="shortcuts"]').click();
    await page.locator('[data-shortcut-clear="overview"]').click();
    await expect(page.locator('[data-shortcut-effective="overview"]')).toHaveText("-");
    await page.keyboard.press("Escape");
    await expect(sidebar.locator("kbd")).toHaveCount(0);
    await page.keyboard.press(chord("overview"));
    await expect(modal).toHaveCount(0);
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="shortcuts"]').click();
    await page.locator('[data-shortcut-reset="overview"]').click();
    await expect(page.locator('[data-shortcut-effective="overview"]')).not.toHaveText("-");
    await page.keyboard.press("Escape");
    await expect(sidebar.locator("kbd")).toHaveCount(1);

    // A modal is not a visit; cycling begins on the most recent agent pane.
    await input.focus();
    await page.keyboard.press(chord("overview"));
    const cycle = held("recent_area_tab");
    for (const key of cycle.modifiers) await page.keyboard.down(key);
    await page.keyboard.press(cycle.key);
    await expect(page.locator('[data-cycle="agents"] [aria-selected="true"]')).toHaveAttribute("data-cycle-row", paneId!);
    await page.keyboard.press("Escape");
    for (const key of cycle.modifiers) await page.keyboard.up(key);
    await expect(modal).toBeVisible();
    const before = sent.get("focus_pane") ?? 0;
    for (const key of cycle.modifiers) await page.keyboard.down(key);
    await page.keyboard.press(cycle.key);
    for (const key of cycle.modifiers) await page.keyboard.up(key);
    await expect(modal).toHaveCount(0);
    await expect(input).toBeFocused();
    expect((sent.get("focus_pane") ?? 0) - before).toBe(1);

    // Focus sidebar closes the Overview and focuses the one sidebar.
    await page.keyboard.press(chord("overview"));
    await expect(modal).toBeVisible();
    // The Overview opens again on the tab it was left on.
    await expect(modal.locator('[data-main-view="projects"]')).toBeVisible();
    await page.keyboard.press(chord("sidebar_projects"));
    await expect(modal).toHaveCount(0);
    await expect.poll(() => page.locator('nav[data-sidebar]').evaluate((node) => node.contains(document.activeElement))).toBe(true);
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    await page.setViewportSize({ width: 720, height: 640 });
    await page.locator("[data-open-overview]").click();
    // Below the cap it takes the window less a 40 px inset on every side.
    const bounds = (await modal.boundingBox())!;
    expect([bounds.x, bounds.y, bounds.width, bounds.height]).toEqual([40, 40, 640, 560]);
    expect(await modal.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    await screenshot(page, "overview-modal-narrow-dark");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
