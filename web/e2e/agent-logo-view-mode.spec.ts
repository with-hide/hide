// The sidebar footer's View menu and the agent logos it turns on (PRD
// agent-logo-view-mode B1-B12, B17): hidden is the quiet default, the menu is
// keyboard-driven, the choice is the core's and follows every window and a
// daemon restart, and a browser has no ⌥ hold, so no peek and no key line.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, openTaskFolds, screenshot, showTasks } from "./wire";

const trigger = (page: Page) => page.locator("[data-view-menu-trigger]");
const menu = (page: Page) => page.locator("[data-view-menu]");
const item = (page: Page, mode: "shown" | "hidden") => page.locator(`[data-view-item="${mode}"]`);
const sidebarRow = (page: Page) => page.locator("[data-sidebar] li[data-pane]").first();
const marks = (page: Page) => page.locator("[data-sidebar] li[data-pane] [data-agent-mark], [data-tab] [data-agent-mark], [data-task-row] [data-agent-mark]");
const box = async (locator: Locator) => (await locator.boundingBox())!;

async function open(page: Page, daemon: Daemon) {
  // A hash-only navigation after a restart does not remount the connection.
  if (page.url().startsWith(daemon.origin)) await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
}

async function choose(page: Page, mode: "shown" | "hidden") {
  await trigger(page).click();
  await item(page, mode).click();
  await expect(menu(page)).toHaveCount(0);
}

test("hidden by default, a menu that checks the core's value, and logos that return in the sidebar, the tab and the 태스크 list", async ({ page }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  const daemon = await startHided(herdr, "agent-logo-menu");
  const sent = countSent(page);
  try {
    await open(page, daemon);
    await expect(sidebarRow(page)).toBeVisible();
    // B4: nothing draws a logo until it is chosen.
    await expect(marks(page)).toHaveCount(0);
    const hiddenRow = await box(sidebarRow(page));

    // B1-B3: the eye button names itself, the menu checks Hide, and the key line is absent in a browser.
    await expect(trigger(page)).toHaveAccessibleName("View");
    await trigger(page).click();
    await expect(menu(page)).toContainText("Agent logo");
    await expect(item(page, "hidden")).toHaveAttribute("aria-checked", "true");
    await expect(item(page, "shown")).toHaveAttribute("aria-checked", "false");
    await expect(page.locator("[data-view-peek]")).toHaveCount(0);
    await screenshot(page, "view-menu-hidden");

    // B3, B9: Escape changes nothing and hands focus back; choosing the checked item sends nothing.
    await page.keyboard.press("Escape");
    await expect(menu(page)).toHaveCount(0);
    await expect(trigger(page)).toBeFocused();
    await choose(page, "hidden");
    expect(sent.get("agent_logo_mode_set") ?? 0).toBe(0);

    // B5-B7: shown draws the logo after the status mark and before the title, in all three places.
    await choose(page, "shown");
    expect(sent.get("agent_logo_mode_set")).toBe(1);
    await expect(sidebarRow(page).locator("[data-agent-mark]")).toHaveCount(1);
    await expect(page.locator("[data-tab] [data-agent-mark]").first()).toBeVisible();
    const status = await box(sidebarRow(page).locator("[data-agent-status-mark]"));
    const logo = await box(sidebarRow(page).locator("[data-agent-mark]"));
    const title = await box(sidebarRow(page).locator("[data-agent-title]"));
    expect(logo.x).toBeGreaterThanOrEqual(status.x + status.width);
    expect(title.x).toBeGreaterThanOrEqual(logo.x + logo.width);
    expect(logo.width).toBe(16);
    // B17: the row keeps its height and the logo adds no wrap.
    expect((await box(sidebarRow(page))).height).toBe(hiddenRow.height);
    await trigger(page).click();
    await expect(item(page, "shown")).toHaveAttribute("aria-checked", "true");
    await screenshot(page, "view-menu-shown");
    await page.keyboard.press("Escape");
    // The sidebar's 태스크 list draws it on every task row as well (PRD sidebar-task-lens B8).
    const tasks = await showTasks(page);
    await openTaskFolds(page);
    await expect(tasks.locator("[data-task-row] [data-agent-mark]").first()).toBeVisible();

    // B8: hiding takes every logo away again.
    await choose(page, "hidden");
    await expect(marks(page)).toHaveCount(0);
    await page.locator('[data-sidebar-lens-option="projects"]').click();
    expect((await box(sidebarRow(page))).height).toBe(hiddenRow.height);
  } finally { daemon.stop(); herdr.stop(); }
});

test("the choice follows a second window at once and survives a daemon restart", async ({ page, browser }) => {
  const herdr = await startHerdr({ readiness: "registered" });
  let daemon = await startHided(herdr, "agent-logo-persist");
  const other = await (await browser.newContext()).newPage();
  try {
    await open(page, daemon);
    await open(other, daemon);
    await choose(page, "shown");
    // B10: the other window changes with no action of its own.
    await expect(sidebarRow(other).locator("[data-agent-mark]")).toHaveCount(1);
    daemon = await daemon.restart();
    await open(page, daemon);
    await expect(sidebarRow(page).locator("[data-agent-mark]")).toHaveCount(1);
    await trigger(page).click();
    await expect(item(page, "shown")).toHaveAttribute("aria-checked", "true");
  } finally { daemon.stop(); herdr.stop(); }
});
