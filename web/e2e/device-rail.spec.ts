// The device rail on an isolated engine and hided (quick
// device-rail-badges B1-B6 and quick device-rail-slack, replacing PRD
// home-device-rail B1-B13): the rail is the sidebar's full-height left column,
// shown with This Mac alone, and has no Inbox or footer device button, its
// `+` opens Settings > Devices > Add device, and each device's sidebar is its
// name, shared Overview and Projects | Agents. A right-click
// hides the rail, the name on the top line becomes the device menu, and the
// choice survives a reload. Registering one device that cannot be reached (a
// Host of the fixture's ssh config that never resolves) adds a dimmed monogram tile with a cross and no
// mark, named by its hint, and, selected, the sidebar reduced to its name, `Not connected` and `Reconnect`.
// Removing it leaves This Mac's rail. The Add project dialog's Host list entry
// needs the desktop host's folder picker, so it is proved in desktop/e2e, not
// in a browser tab.

import { expect, test, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, writeSshHost, type Daemon } from "./hided-fixture";
import { countSent, screenshot, THIS_MACHINE } from "./wire";
import { SYSTEM } from "./chords";
import { animationsFinished } from "./wait";

test.describe.configure({ timeout: 180_000 });

const ALIAS = "unreachable-e2e";
const CENTER = "[data-main-screen], [data-workspace-screen]";

async function openAddDeviceForm(page: Page): Promise<void> {
  await expect(page.locator('[data-settings="true"]')).toBeVisible();
  await expect(page.locator('[data-settings-tab="devices"]')).toHaveAttribute("data-state", "active");
  // Every entry point opens Settings with the Add device dialog in front.
  await expect(page.locator("[data-add-device-dialog]")).toBeVisible();
  await expect(page.locator("[data-add-device]")).toBeVisible();
}

/** Leaves Settings: the Add device dialog is in front, and either still is or closes itself once the device is listed. */
async function leaveSettings(page: Page, dialog: "is open" | "closes itself"): Promise<void> {
  if (dialog === "is open") await page.keyboard.press("Escape");
  await expect(page.locator("[data-add-device-dialog]")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
}

/** Closes the Add device dialog, leaving the Settings sheet it opened over. */
async function closeAddDeviceDialog(page: Page): Promise<void> {
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-add-device-dialog]")).toHaveCount(0);
}

test("the rail follows the registered devices; a device that cannot be reached is dimmed and offers Reconnect", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "device-rail");
    writeSshHost(daemon, ALIAS);
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator(CENTER).first()).toBeVisible({ timeout: 20_000 });
    const sidebar = page.locator("nav[data-sidebar]");

    // B1: the rail is shown with This Mac alone, and there is no Inbox tile and no footer device button.
    const rail = page.locator("[data-device-rail]");
    await expect(rail).toBeVisible();
    await expect(rail.locator("[data-rail-tile]")).toHaveCount(1);
    await expect(rail.locator(`[data-rail-tile="${daemon.node}"]`)).toHaveAttribute("aria-pressed", "true");
    await expect(rail.locator('[data-rail-tile="inbox"]')).toHaveCount(0);
    // #349: device name, shared Overview, then Projects | Agents with direct keycaps.
    await expect(page.locator("[data-sidebar-title-name]")).toHaveText(THIS_MACHINE);
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    await expect(page.locator("[data-sidebar-overview]")).toHaveText(SYSTEM === "mac" ? "Overview⇧⌘O" : "OverviewAlt+Shift+O");
    await expect(page.locator("[data-project-list]")).toBeVisible();

    // B6: a right-click on the rail offers Hide rail; hidden, the name is the device menu with Add device… and Show device rail; the choice is the core's, so it survives a reload.
    await rail.click({ button: "right", position: { x: 10, y: 400 } });
    const railMenu = page.locator('[data-device-rail-menu][role="menu"]');
    await expect(railMenu).toContainText("Hide rail");
    await railMenu.locator('[data-menu-item="hide"]').click();
    await expect(rail).toHaveCount(0);
    await expect(page.locator("[data-sidebar-device-menu]")).toContainText(THIS_MACHINE);
    await page.reload();
    await expect(page.locator(CENTER).first()).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("[data-device-rail]")).toHaveCount(0);
    await screenshot(page, "device-rail-hidden");
    await page.locator("[data-sidebar-device-menu]").click();
    const deviceMenu = page.locator("[data-device-menu]");
    await expect(deviceMenu.locator(`[data-device-menu-item="${daemon.node}"]`)).toBeVisible();
    await screenshot(page, "device-rail-hidden-menu");
    // B1: Add device… opens Settings > Devices > Add device.
    await deviceMenu.locator("[data-device-menu-add]").click();
    await openAddDeviceForm(page);
    await leaveSettings(page, "is open");
    await page.locator("[data-sidebar-device-menu]").click();
    await page.locator("[data-device-menu-show-rail]").click();
    await expect(rail).toBeVisible();

    // B1: `+` sits directly under the last device tile and opens the same form.
    const addBox = (await rail.locator("[data-rail-add]").boundingBox())!;
    const lastBox = (await rail.locator(`[data-rail-tile="${daemon.node}"]`).boundingBox())!;
    expect(addBox.y).toBeGreaterThan(lastBox.y + lastBox.height - 1);
    expect(addBox.y - (lastBox.y + lastBox.height)).toBeLessThan(24);
    await rail.locator("[data-rail-add]").click();
    await openAddDeviceForm(page);

    // Registering a device, even an unreachable one, adds its tile; the center stays where it was.
    const centerBefore = await page.locator("[data-main-screen]").count();
    await page.locator(`[data-ssh-host="${ALIAS}"]`).click();
    await page.locator("[data-device-label]").fill("연구실 빌드 서버 자동화 장비");
    await page.locator("[data-add-device]").click();
    await leaveSettings(page, "closes itself");
    await expect(rail.locator("[data-rail-tile]")).toHaveCount(2, { timeout: 20_000 });
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    const ids = await rail.locator("[data-rail-tile]").evaluateAll((tiles) => tiles.map((tile) => tile.getAttribute("data-rail-tile")));
    expect(ids).toEqual([daemon.node, ALIAS]);
    // The rail is its own fixed column beside the content column: the stored width stays the content's, and the rail adds to it.
    // It runs the sidebar's full height under the window's top row, whose device title spans the rail and the content (sidebar-titlebar-refresh B17).
    const railBox = (await rail.boundingBox())!;
    const contentBox = (await page.locator("[data-sidebar-content]").boundingBox())!;
    const sidebarBox = (await sidebar.boundingBox())!;
    expect(Math.round(railBox.width)).toBe(48);
    // The sidebar's right hairline is the column's own edge, inside the stored width.
    expect(Math.round(contentBox.width)).toBe(292 - 1);
    expect(Math.round(sidebarBox.width)).toBe(48 + 292);
    expect(Math.round(railBox.x)).toBe(Math.round(sidebarBox.x));
    expect(Math.round(railBox.y)).toBe(Math.round(sidebarBox.y));
    expect(Math.round(railBox.height)).toBe(Math.round(sidebarBox.height));
    const titleBox = (await page.locator("[data-sidebar-title]").boundingBox())!;
    expect([Math.round(titleBox.x), Math.round(titleBox.width), Math.round(titleBox.y + titleBox.height) <= Math.round(railBox.y)]).toEqual([Math.round(railBox.x), Math.round(sidebarBox.width), true]);
    // This Mac is the selection, and the center is where it was.
    await expect(rail.locator(`[data-rail-tile="${daemon.node}"]`)).toHaveAttribute("aria-pressed", "true");
    expect(await page.locator("[data-main-screen]").count()).toBe(centerBefore);
    await expect(page.locator("[data-sidebar-title-name]")).toHaveText(THIS_MACHINE);

    // B3, B8: the unreachable device is dimmed with a cross, has no badge, and its name says so.
    const tile = rail.locator(`[data-rail-tile="${ALIAS}"]`);
    await expect(tile).toHaveAttribute("data-rail-connected", "false", { timeout: 30_000 });
    await expect(tile.locator("[data-rail-off]")).toBeVisible();
    await expect(tile.locator("[data-rail-badge]")).toHaveCount(0);
    await expect(tile).toHaveAccessibleName(/Not connected/);
    for (const theme of ["dark", "light"] as const) {
      await page.evaluate((next) => {
        document.documentElement.classList.toggle("dark", next === "dark");
        document.documentElement.classList.toggle("light", next === "light");
      }, theme);
      await animationsFinished(page);
      await screenshot(page, `device-rail-this-mac-${theme}`);
    }

    // B41: a tile takes Tab focus and answers Enter.
    await tile.focus();
    await expect(tile).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(tile).toHaveAttribute("aria-pressed", "true");
    await expect(rail.locator(`[data-rail-tile="${daemon.node}"]`)).toHaveAttribute("aria-pressed", "false");

    // B8, B9: selected, the sidebar is the name and Not connected, with no tree; Reconnect is the center's, drawn once.
    const disconnected = page.locator("[data-device-disconnected]");
    await expect(disconnected).toBeVisible();
    await expect(disconnected).toContainText("Not connected");
    await expect(disconnected.locator("[data-device-disconnected-name]")).toHaveText("연구실 빌드 서버 자동화 장비");
    await expect(page.locator("[data-project-list]")).toHaveCount(0);
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    await expect(page.locator("[data-sidebar-title-name]")).toContainText("연구실");
    await screenshot(page, "device-rail-not-connected");
    await expect(disconnected.locator("[data-device-reconnect]")).toHaveCount(0);
    const retried = sent.get("retry_connect") ?? 0;
    await page.locator("[data-device-retry]").click();
    await expect.poll(() => (sent.get("retry_connect") ?? 0) - retried).toBe(1);
    await expect(disconnected).toBeVisible();

    // The Korean name pushes none of the rail, the header line and the content column sideways (B46).
    // The nav itself is not measured: its drag edge straddles its right side on purpose.
    for (const part of ["[data-device-rail]", "[data-sidebar-title]", "[data-sidebar-content]"]) {
      const overflow = await page.locator(part).evaluate((element) => element.scrollWidth - element.clientWidth);
      expect(overflow, part).toBeLessThanOrEqual(0);
    }
    // The tile draws the name's monogram and no name; its hint is the name and its connection.
    await expect(tile.locator("[data-rail-glyph]")).toHaveText("연빌");
    await tile.hover();
    await expect(page.getByRole("tooltip")).toContainText("연구실 빌드 서버 자동화 장비 · Not connected");

    // B13: the rail's + opens the same Add device form.
    await rail.locator("[data-rail-add]").click();
    await openAddDeviceForm(page);
    await closeAddDeviceDialog(page);

    // B10: removing the only remote device leaves This Mac's rail, with This Mac in front.
    await page.locator(`[data-device-menu="${ALIAS}"]`).click();
    await page.locator(`[data-device-remove="${ALIAS}"]`).click();
    await page.locator("[data-device-remove-go]").click();
    await expect(rail.locator("[data-rail-tile]")).toHaveCount(1, { timeout: 20_000 });
    await page.keyboard.press("Escape");
    await expect(rail.locator(`[data-rail-tile="${daemon.node}"]`)).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    await expect(page.locator("[data-project-list]")).toBeVisible();
    await expect(page.locator(CENTER).first()).toBeVisible();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
