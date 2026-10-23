import { expect, type ElectronApplication, type Page, type TestInfo } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { enterWorkspace, pinState, setLanePinned, showExplorer } from "../../web/e2e/wire";
import { afterCleanup } from "../../web/e2e/worker-owned";
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

/** Observe the existing flow without adding waits between its menu commands. */
async function observeMenu(page: Page, info: TestInfo): Promise<() => Promise<void>> {
  const rows: unknown[] = [];
  let overflow = false;
  const record = (row: unknown) => { if (rows.length < 256) rows.push(row); else overflow = true; };
  const prefix = "[menu-target]";
  const consoleRecord = (message: { text(): string }) => {
    if (message.text().startsWith(prefix)) record(JSON.parse(message.text().slice(prefix.length)));
  };
  page.on("console", consoleRecord);
  const wire = await page.context().newCDPSession(page);
  await wire.send("Network.enable");
  wire.on("Network.webSocketFrameSent", ({ response }) => {
    if (response.opcode !== 1) return;
    const event = JSON.parse(response.payloadData) as { kind?: string; payload?: { pane_id?: string; confirmed?: boolean; open?: boolean; documents?: boolean; action?: string } };
    if (!["close_pane", "close_tab", "workspace_view", "view_layout"].includes(event.kind ?? "")) return;
    const p = event.payload;
    record({ boundary: "dispatch", at: Date.now(), kind: event.kind, pane: p?.pane_id, confirmed: p?.confirmed, open: p?.open, documents: p?.documents, action: p?.action });
  });
  wire.on("Network.webSocketFrameReceived", ({ response }) => {
    if (response.opcode !== 1) return;
    const frame = JSON.parse(response.payloadData) as { type: string; payload?: { revision?: number; rest?: { workspace_view?: { open?: boolean; documents?: boolean }; terminal?: { pane_id?: string }; focused?: { pane_id?: string } } } };
    if (frame.type !== "snapshot" && frame.type !== "delta") return;
    const rest = frame.payload?.rest;
    if (rest?.workspace_view || rest?.terminal || rest?.focused) record({ boundary: "core", at: Date.now(), revision: frame.payload?.revision, open: rest?.workspace_view?.open, documents: rest?.workspace_view?.documents, pane: rest?.terminal?.pane_id ?? rest?.focused?.pane_id });
  });
  await page.evaluate((tag) => {
    let previous = "";
    const capture = (boundary: string, id?: string) => {
      const workspace = document.querySelector<HTMLElement>("[data-workspace-screen]");
      const owner = (window as unknown as { __hideProbe: { commandOwner(): { kind: string; workspace?: string; paneId?: string; areaId?: string } } }).__hideProbe.commandOwner();
      // Checkout keys can contain paths; retain only the target's relation to this screen.
      const state = { owner: { kind: owner.kind, pane: owner.paneId, area: owner.areaId, inWorkspace: owner.workspace === workspace?.dataset.workspaceScreen }, lane: workspace?.dataset.laneMode, view: workspace?.dataset.laneView, panes: [...document.querySelectorAll<HTMLElement>("[data-pane-view]")].map((pane) => pane.dataset.paneView), confirmation: document.querySelector<HTMLElement>("[data-confirm-close]")?.dataset.confirmClose ?? null };
      const value = JSON.stringify(state);
      if (boundary === "draw" && value === previous) return;
      previous = value;
      console.debug(tag + JSON.stringify({ boundary, id, at: Date.now(), ...state }));
    };
    const commandOff = window.hideHost!.onCommand((id) => capture("host-command", id));
    const browserOff = window.hideHost!.browser.onEvent((event) => { if (event.kind === "focus") capture("native-focus", event.id); });
    const observer = new MutationObserver(() => capture("draw"));
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-lane-mode", "data-lane-view", "data-confirm-close"] });
    (window as unknown as { stopMenuTrace: () => void }).stopMenuTrace = () => { observer.disconnect(); commandOff(); browserOff(); };
    capture("start");
  }, prefix);
  return async () => {
    const failures: unknown[] = [];
    try { await page.evaluate(() => (window as unknown as { stopMenuTrace: () => void }).stopMenuTrace()); }
    catch (error) { failures.push(new Error(`stop menu listeners: ${String(error)}`, { cause: error })); }
    try { page.off("console", consoleRecord); }
    catch (error) { failures.push(new Error(`remove menu console listener: ${String(error)}`, { cause: error })); }
    try { await wire.detach(); }
    catch (error) { failures.push(new Error(`detach menu CDP session: ${String(error)}`, { cause: error })); }
    try { await info.attach("menu-target-transitions", { body: Buffer.from(JSON.stringify({ overflow, rows }, null, 2)), contentType: "application/json" }); }
    catch (error) { failures.push(new Error(`attach menu transitions: ${String(error)}`, { cause: error })); }
    if (failures.length === 1) throw failures[0];
    if (failures.length) throw new AggregateError(failures, failures.map(String).join("\n"));
  };
}

// eslint-disable-next-line no-empty-pattern -- Playwright supplies TestInfo after the fixtures argument.
test("Command T and Command W from a native page act on its View area, and after any other command on the terminal that has the keyboard", { tag: NEEDS_FOCUS }, async ({}, info) => {
  const herdr = await startHerdr({ agents: false });
  const run = isolate(herdr, "new-tab");
  let app: ElectronApplication | null = null;
  let stopObserving: (() => Promise<void>) | undefined;
  const failures: unknown[] = [];
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
    const url = new URL(page.url());
    url.searchParams.set("probe", "1");
    await page.goto(url.href);
    await enterWorkspace(page);
    await expect.poll(() => page.evaluate(() => typeof (window as unknown as { __hideProbe?: { commandOwner(): unknown } }).__hideProbe?.commandOwner)).toBe("function");
    stopObserving = await observeMenu(page, info);
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
    // The page and the Explorer fit at their minimums beside 480px of
    // agents, so the pin docks both columns, and the lane stays through a
    // terminal click (PRD lane-two-columns B19).
    await setLanePinned(page, true);
    expect(await pinState(page)).toBe("on");
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "docked");

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
  } catch (error) {
    failures.push(error);
  } finally {
    try { await stopObserving?.(); } catch (error) { failures.push(error); }
    try { await app?.close(); } catch (error) { failures.push(new Error(`close menu candidate: ${String(error)}`, { cause: error })); }
    try { run.cleanup(); } catch (error) { failures.push(new Error(`clean menu fixture: ${String(error)}`, { cause: error })); }
    try { herdr.stop(); } catch (error) { failures.push(new Error(`stop private menu Herdr: ${String(error)}`, { cause: error })); }
  }
  if (failures.length) {
    let primary = failures[0];
    for (const secondary of failures.slice(1)) primary = afterCleanup(primary, () => { throw secondary; });
    throw primary;
  }
});
