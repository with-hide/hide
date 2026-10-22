import { expect, type ElectronApplication, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace, pinState, setLanePinned, showExplorer } from "../../web/e2e/wire";
import { fitWindow, isolate, launch, NEEDS_FOCUS, test } from "./fixture";

const TITLE = "New tab page";

/**
 * Enters the native page the way the operator does and waits until the shell
 * has heard of it. The page lives in native child contents, outside the shell
 * DOM, and its host reports the focus; the shell records it before any later
 * listener on that channel runs.
 */
async function enterPage(app: ElectronApplication, page: Page) {
  type Counted = { pageFocus?: number; pageFocusOff?: () => void };
  const before = await page.evaluate(() => {
    const counted = window as unknown as Counted;
    counted.pageFocusOff ??= window.hideHost!.browser.onEvent((event) => {
      if (event.kind === "focus") counted.pageFocus = (counted.pageFocus ?? 0) + 1;
    });
    return counted.pageFocus ?? 0;
  });
  await expect.poll(() => focusPageTakingAppFocus(app)).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as unknown as Counted).pageFocus ?? 0), { message: "the shell hears the page take the keyboard" }).toBeGreaterThan(before);
}

/**
 * What the browser does when the host hands the keyboard back to the shell
 * to deliver a menu command: it announces focus again on the element that had
 * it before the page took the keyboard, here the terminal's input.
 */
async function handBack(page: Page) {
  const inPane = await page.evaluate(() => {
    const element = document.activeElement!;
    element.dispatchEvent(new FocusEvent("focus"));
    element.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    return element.closest("[data-pane-view]") !== null;
  });
  expect(inPane).toBe(true);
}

/**
 * TAKES THE APP'S FOCUS: a page holds the keyboard only in the key window, so
 * this steals it for the app where the machine grants that, and the menu
 * command then takes the host's real hand-back path. A click in the shell DOM
 * does not move the contents' focus, so the shell takes it first and the page
 * reports entering again. False until the page has loaded.
 */
function focusPageTakingAppFocus(app: ElectronApplication): Promise<boolean> {
  return app.evaluate(({ app: electron, BrowserWindow }, title) => {
    const window = BrowserWindow.getAllWindows()[0]!;
    const view = window.contentView.children.find((child) =>
      (child as { webContents?: Electron.WebContents }).webContents?.getTitle() === title);
    if (!view) return false;
    electron.focus({ steal: true });
    window.focus();
    window.webContents.focus();
    (view as unknown as { webContents: Electron.WebContents }).webContents.focus();
    return true;
  }, TITLE);
}

const menuClick = (app: ElectronApplication, id: string) =>
  app.evaluate(({ Menu }, command) => Menu.getApplicationMenu()!.getMenuItemById(command)!.click(), id);

test("Command T and Command W from a native page act on its View area, and after any other command on the terminal that has the keyboard", { tag: NEEDS_FOCUS }, async () => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "new-tab");
  let app: ElectronApplication | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "page.html"), `<!doctype html><meta charset="utf-8"><title>${TITLE}</title><h1>Browser keyboard owner</h1>`);
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    // D-07/B25 place a 1024px body in the 848..1115px step: Agent Views
    // stays beside File Views, while Tools gives way. Hiding the sidebar
    // makes the actual body fit that contract on the CI runner's screen.
    await fitWindow(app, { width: 1024, height: 700 });
    await enterWorkspace(page);
    await menuClick(app, "toggle_left_sidebar");
    await expect(page.locator("[data-sidebar]")).toHaveCount(0);
    const agentTabs = page.locator('[data-agent-tab-bar] [role="tab"]');
    const viewTabs = page.locator('[data-view-tab-bar] [role="tab"]');
    const panes = page.locator("[data-pane-view]");
    const terminal = (pane: string) => page.locator(`[data-terminal-host="${pane}"]`).click({ position: { x: 20, y: 20 } });
    await expect(agentTabs).toHaveCount(1);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/page.html"]').click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-browser"]').click();
    await expect(page.locator("[data-browser-slot]")).toBeVisible();
    // Too narrow to dock, the pinned lane opens over the agents and stays
    // through a terminal click (PRD right-lane B19).
    await setLanePinned(page, true);
    expect(await pinState(page)).toBe("narrow");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "overlay");

    // Command T: the page's area gets the View strip's New tab.
    await terminal(herdr.panes[0]);
    await enterPage(app, page);
    await handBack(page);
    await menuClick(app, "new_tab");
    await expect(viewTabs).toHaveCount(2);
    await expect(page.getByRole("textbox", { name: "Page address" })).toBeFocused();
    await expect(page.getByRole("textbox", { name: "Page address" })).toHaveValue("");
    await expect(agentTabs).toHaveCount(1);

    // Command W: the page's display closes and every pane stays.
    await viewTabs.filter({ hasText: TITLE }).click();
    await expect(page.locator("[data-browser-slot]")).toBeVisible();
    await terminal(herdr.panes[0]);
    await enterPage(app, page);
    await handBack(page);
    await menuClick(app, "close_tab");
    await expect(viewTabs).toHaveCount(1);
    await expect(viewTabs.filter({ hasText: TITLE })).toHaveCount(0);
    await expect(panes).toHaveCount(2);

    // Command E from the page closes the pinned lane and leaves the keyboard
    // with the terminal; the next Command E brings it back on Files, and one
    // more closes it. Command W must still close the pane, not the page (B20, B23).
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/page.html"]').click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-browser"]').click();
    await expect(viewTabs.filter({ hasText: TITLE })).toHaveCount(1);
    await terminal(herdr.panes[0]);
    await enterPage(app, page);
    await handBack(page);
    await menuClick(app, "toggle_lane");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator(`[data-terminal-host="${herdr.panes[0]}"] textarea`)).toBeFocused();
    await menuClick(app, "toggle_lane");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-view", "documents");
    await menuClick(app, "toggle_lane");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await menuClick(app, "close_tab");
    await expect(page.locator(`[data-pane-view="${herdr.panes[0]}"]`)).toHaveCount(0, { timeout: 15_000 });
    await expect(panes).toHaveCount(1);
    await page.locator('[data-lane-tool="documents"]').click();
    await expect(viewTabs.filter({ hasText: TITLE })).toHaveCount(1);

    // A click back into a terminal is the operator's move: Command T is an agent tab again.
    await terminal(herdr.panes[1]);
    await menuClick(app, "new_tab");
    await expect(agentTabs).toHaveCount(2);
  } finally {
    await app?.close().catch(() => undefined);
    run.cleanup();
    herdr.stop();
  }
});
