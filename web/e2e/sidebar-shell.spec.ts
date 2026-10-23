// The sidebar's shell and the window's top row on an isolated pinned Herdr and
// hided (PRD sidebar-shell B3-B5, sidebar-titlebar-refresh B11-B14, B17-B21,
// D-07): the top row names the device over the rail and the sidebar and
// carries a Workspace's path and icons; under it the Search row opens the ⌘K
// palette with the chord as bound, the Overview row opens All projects, the
// free app has no Factory entry, and no Home row is left;
// a browser tab adds a project by a typed path; the footer's Help menu opens the
// shortcut sheet, copies the diagnostics, opens the new-issue page and names
// the version. Captured in Dark and Light.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chooseTheme, enterWorkspace, screenshot, showProjectRow, sidebarRowsFit, THIS_MACHINE } from "./wire";
import { chord, commandLabel, mod } from "./chords";

test.describe.configure({ timeout: 120_000 });

/** One private stack with the page on it, the Projects list drawn; `check` runs with it and both are stopped after. */
async function withShell(
  page: Page,
  label: string,
  check: (stack: { herdr: HerdrFixture; daemon: Daemon }) => Promise<void>,
  prepare?: (herdr: HerdrFixture) => void,
): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    prepare?.(herdr);
    daemon = await startHided(herdr, label);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("nav[data-sidebar]")).toHaveAttribute("data-sidebar", "projects", { timeout: 20_000 });
    await expect(page.locator("[data-project-list] [data-checkout]").first()).toBeVisible({ timeout: 20_000 });
    await check({ herdr, daemon });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
}

test("the Search row and the Overview row under the device, with no Home row, and Add project by a typed path in a browser tab", async ({ page }) => {
  await withShell(page, "sidebar-shell", async ({ daemon }) => {
    const sidebar = page.locator("nav[data-sidebar]");
    const list = page.locator("[data-project-list]");
    // B3: Projects is the first tab and the one shown.
    await expect(page.locator("[data-sidebar-mode]")).toHaveCount(0);
    // B11: no Home row and no `+` for it.
    await expect(sidebar.getByRole("button", { name: /^Home\b/ })).toHaveCount(0);
    await expect(list).not.toContainText("All projects");
    await expect(sidebar.locator("input")).toHaveCount(0);

    // B12: the Search row sits under the top row, above Overview, with the chord as bound; a press or Enter opens the palette.
    const search = page.locator("[data-sidebar-search]");
    const overview = page.locator("[data-sidebar-overview]");
    await expect(search.locator("[data-sidebar-search-chord]")).toHaveText(commandLabel("search"));
    expect((await search.boundingBox())!.y).toBeLessThan((await overview.boundingBox())!.y);
    await search.click();
    const palette = page.locator('[data-palette="Search"]');
    await expect(palette.locator("[data-palette-input]")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(palette).toHaveCount(0);
    await search.focus();
    await page.keyboard.press("Enter");
    await expect(palette.locator("[data-palette-input]")).toBeFocused();
    await page.keyboard.press("Escape");
    // The free app has no Factory entry or screen.
    await expect(page.locator("[data-sidebar-factory], [data-sidebar-factory-soon]")).toHaveCount(0);
    await page.keyboard.press(mod("KeyF", { shift: true }));
    await page.keyboard.press(chord("search"));
    await expect(palette.locator("[data-palette-input]")).toBeFocused();
    await expect(page.locator("[data-factory-screen]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    for (const theme of ["dark", "light"] as const) {
      await chooseTheme(page, theme);
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      await screenshot(page, `sidebar-shell-search-${theme}`);
    }

    // Over a Workspace the Overview row opens the device Overview as a layer, and Escape returns to the Workspace.
    await list.locator("[data-checkout]").first().click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(overview).not.toHaveAttribute("aria-current", "page");
    await overview.click();
    const modal = page.getByRole("dialog", { name: "Overview", exact: true });
    await expect(modal.locator("[data-overview-page]")).toBeVisible();
    await expect(overview).toHaveAttribute("aria-current", "page");
    await page.keyboard.press("Escape");
    await expect(modal).toHaveCount(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();

    // B5: a browser tab has no folder picker and no chord for Add project (Chrome keeps ⇧⌘N), so the row
    // under Overview carries no keycap and the desktop chord opens nothing; the row opens the dialog with
    // this machine's `~/` path field in place of Browse folder, and a typed `~/` folder, read against
    // the daemon's home, registers the project (the desktop app's picker: desktop/e2e/add-project.spec.ts).
    const addProject = page.locator("[data-sidebar-new-workspace]");
    await expect(addProject).toBeVisible();
    await expect(addProject.locator("kbd")).toHaveCount(0);
    await page.keyboard.press(chord("new_workspace", "electron"));
    await expect(page.locator("[data-add-project]")).toHaveCount(0);
    fs.mkdirSync(path.join(daemon.home, "projects", "typed-project"), { recursive: true });
    await addProject.click();
    const dialog = page.locator("[data-add-project]");
    await expect(dialog).toBeVisible();
    await expect(dialog.locator("[data-add-project-browse]")).toHaveCount(0);
    const field = dialog.locator("[data-add-project-path]");
    await expect(field).toBeFocused();
    await screenshot(page, "sidebar-shell-add-project-browser");
    await field.fill("~/projects/typed-project");
    await field.press("Enter");
    await expect(page.locator("[data-project]", { hasText: "typed-project" })).toBeVisible({ timeout: 20_000 });
    // The same folder typed again is answered by the core as already a project; the shell cannot match
    // `~/` against the stored absolute path itself.
    await addProject.click();
    await field.fill("~/projects/typed-project/");
    await field.press("Enter");
    await expect(dialog.locator('[data-registration-reason="already_registered"]')).toBeVisible();
    await expect(dialog.locator("[data-registration-pending]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);

    // The direct commands focus this list or restore the Overview's last tab (overview-redesign B1).
    await page.keyboard.press(chord("sidebar_projects"));
    await expect.poll(() => sidebar.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    await page.keyboard.press(chord("overview"));
    await expect(modal.locator('[data-main-view="work"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(sidebar).toHaveAttribute("data-sidebar", "projects");
    await expect(list).toBeVisible();
  });
});

test("the window's top row names the device over the rail and the sidebar, and carries the Workspace's path and icons", async ({ page }) => {
  await withShell(page, "sidebar-title-row", async () => {
    const row = page.locator("[data-title-row]");
    const sidebar = page.locator("nav[data-sidebar]");
    const center = page.locator("[data-title-center]");
    // B17, B23: one row at the window's top, with no traffic lights' space in a browser tab, the device over the rail and the sidebar.
    const rowBox = (await row.boundingBox())!;
    expect([rowBox.x, rowBox.y]).toEqual([0, 0]);
    await expect(row).toHaveAttribute("data-traffic-lights", "false");
    const title = page.locator("[data-sidebar-title]");
    await expect(title.locator("[data-sidebar-title-name]")).toHaveText(THIS_MACHINE);
    const titleBox = (await title.boundingBox())!;
    const sidebarBox = (await sidebar.boundingBox())!;
    expect([Math.round(titleBox.x), Math.round(titleBox.width)]).toEqual([Math.round(sidebarBox.x), Math.round(sidebarBox.width)]);
    expect(sidebarBox.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height - 0.5);
    // B20: on a screen that is not a Workspace, here All projects where a first run starts, the row's center is empty.
    await expect(page.locator("[data-overview-page]")).toBeVisible();
    await expect(center.locator("*")).toHaveCount(0);

    // B17, B18: a Workspace puts its project / checkout path and its icons in the row, and no toolbar under it.
    await enterWorkspace(page, "fixture");
    const project = center.locator("[data-workspace-project]");
    await expect(project).toHaveText("fixture");
    await expect(center.locator("[data-workspace-location]")).toBeVisible();
    await expect(center.locator("[data-workspace-actions] button")).toHaveCount(2);
    const workspaceBox = (await page.locator("[data-workspace-screen]").boundingBox())!;
    expect(workspaceBox.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height - 0.5);
    for (const theme of ["dark", "light"] as const) {
      await chooseTheme(page, theme);
      await page.mouse.move(640, 600);
      await screenshot(page, `title-row-workspace-${theme}`);
    }
    // The checkout keeps its path menu; a project opens its local home in Projects.
    await center.locator("[data-workspace-location]").click({ button: "right" });
    await expect(page.locator('[role="menu"] [data-menu-item="copy_path"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await project.click();
    const modal = page.getByRole("dialog", { name: "Overview", exact: true });
    await expect(modal.locator('[data-main-view="projects"]')).toBeVisible();
    await expect(modal.locator('[data-overview-project-home] h2').first()).toHaveText("fixture");
    // The project home is the tab's content, so Escape closes the Overview.
    await page.keyboard.press("Escape");
    await expect(modal).toHaveCount(0);

    // B21: the sidebar's switch opens the row and stays where it was when the sidebar hides; the path follows it,
    // and a press on it brings the sidebar back without the chord, leaving the keyboard in the terminal as the chord does.
    const sidebarSwitch = row.locator("[data-sidebar-switch]");
    const switchX = Math.round((await sidebarSwitch.boundingBox())!.x);
    const terminal = page.locator("[data-pane-view] .xterm-helper-textarea").first();
    await terminal.focus();
    await page.keyboard.press(chord("toggle_left_sidebar"));
    await expect(sidebar).toHaveCount(0);
    await expect(title).toHaveCount(0);
    await expect(sidebarSwitch).toHaveAttribute("aria-expanded", "false");
    const switchBox = (await sidebarSwitch.boundingBox())!;
    expect(Math.round(switchBox.x)).toBe(switchX);
    expect((await center.boundingBox())!.x).toBeGreaterThanOrEqual(switchBox.x + switchBox.width - 0.5);
    await expect(project).toBeVisible();
    await sidebarSwitch.click();
    await expect(title).toBeVisible();
    await expect(sidebar).toBeVisible();
    await expect(sidebarSwitch).toHaveAttribute("aria-expanded", "true");
    await expect(terminal).toBeFocused();
    await expect(page.locator('[data-agent-area-id][data-keyboard-area="true"]')).toHaveCount(1);
  });
});

/** A folder whose name is longer than any row or path has room for at the smallest window. */
const LONG_NAME = "빌드-서버-자동화-장비-점검-작업-폴더-for-the-weekly-release-notes";

// PRD sidebar-titlebar-refresh B24.
test("at the smallest window and the largest Interface text, long names shorten and no icon hides", async ({ page }) => {
  await withShell(page, "sidebar-title-row-small", async () => {
    await page.setViewportSize({ width: 720, height: 480 });
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    const fontSize = page.locator('[data-font-size="true"] [role="slider"]');
    await fontSize.focus();
    await page.keyboard.press("End");
    await expect(fontSize).toHaveAttribute("aria-valuenow", "17");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);

    const row = await showProjectRow(page, page.locator("[data-project]").filter({ has: page.locator("[data-row-name]", { hasText: LONG_NAME }) }).locator("[data-checkout]").first());
    await row.click();
    const center = page.locator("[data-title-center]");
    const project = center.locator("[data-workspace-project]");
    await expect(project).toHaveText(LONG_NAME);
    // The path shortens with an ellipsis, inside the row; the icons keep their place.
    const cut = (element: HTMLElement) => element.scrollWidth > element.clientWidth;
    expect(await project.evaluate(cut) || await center.locator("[data-workspace-location]").evaluate(cut)).toBe(true);
    expect(await page.locator("[data-title-row]").evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    const toggles = center.locator("[data-workspace-actions] button");
    await expect(toggles).toHaveCount(2);
    const rowBox = (await page.locator("[data-title-row]").boundingBox())!;
    for (const toggle of await toggles.all()) {
      await expect(toggle).toBeVisible();
      const box = (await toggle.boundingBox())!;
      expect(box.x + box.width).toBeLessThanOrEqual(rowBox.x + rowBox.width + 0.5);
    }
    // The sidebar's long Korean name, the No agents fold, Search and the footer overlap nothing.
    expect(await sidebarRowsFit(page)).toEqual([]);
    await screenshot(page, "title-row-smallest-largest-text");
  }, (herdr) => {
    const folder = path.join(herdr.root, LONG_NAME);
    fs.mkdirSync(folder);
    herdr.run(["workspace", "create", "--cwd", folder, "--label", LONG_NAME, "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
  });
});

test("the Help menu opens the shortcut sheet, copies the diagnostics, opens the new-issue page and names the version", async ({ page }) => {
  await withShell(page, "sidebar-help", async ({ daemon }) => {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: daemon.origin });
    // The new-issue page is answered here, so the test never reaches GitHub.
    await page.context().route("https://github.com/**", (route) => route.fulfill({ status: 200, contentType: "text/html", body: "<title>new issue</title>" }));
    const trigger = page.locator("[data-help-menu-trigger]");
    const settings = page.locator("[data-open-settings]");
    // B13: `?` sits before the gear.
    await expect(trigger).toHaveAccessibleName("Help");
    const triggerBox = (await trigger.boundingBox())!;
    const settingsBox = (await settings.boundingBox())!;
    expect(triggerBox.x + triggerBox.width).toBeLessThanOrEqual(settingsBox.x);
    expect(Math.round(triggerBox.y)).toBe(Math.round(settingsBox.y));

    // B13: the actions, then the version as a line to read.
    await trigger.click();
    const menu = page.locator("[data-help-menu]");
    await expect(menu.locator("[data-help-item]")).toHaveText([`Keyboard shortcuts${commandLabel("shortcuts")}`, "Practice Hide", "Copy diagnostics", "Report an issue"]);
    const version = menu.locator("[data-help-version]");
    await expect(version).toHaveText(/^hide \S+$/);
    await expect(version).not.toHaveAttribute("role", "menuitem");
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    for (const theme of ["dark", "light"] as const) {
      await chooseTheme(page, theme);
      await trigger.click();
      await expect(menu).toBeVisible();
      await screenshot(page, `sidebar-help-menu-${theme}`);
      await page.keyboard.press("Escape");
    }

    // B14: Copy diagnostics copies the About text and says so in the open menu.
    await trigger.click();
    await menu.locator('[data-help-item="copy_diagnostics"]').click();
    await expect(menu.locator("[data-help-copied]")).toHaveText("Copied");
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain((await version.getAttribute("data-help-version"))!);
    await page.keyboard.press("Escape");

    // B14: Report an issue opens the repository's new-issue page outside hide.
    await trigger.click();
    const popup = page.context().waitForEvent("page");
    await menu.locator('[data-help-item="report_issue"]').click();
    const opened = await popup;
    expect(opened.url()).toBe("https://github.com/with-hide/hide/issues/new");
    await opened.close();

    // B14: Keyboard shortcuts opens the shortcut sheet.
    await trigger.click();
    await menu.locator('[data-help-item="shortcuts"]').click();
    await expect(page.locator("[data-shortcut-sheet]")).toBeVisible();
  });
});
