// The desktop app's numbered chords and hold hints end to end (PRD
// electron-digit-shortcuts-hints D-09): ⌃2 moves no tab; holding ⌘ alone
// shows each tab's digit after the delay and moves nothing, adding ⌥ turns
// them to the device tiles' digits, ⌘2 and ⌘9 choose a tab, ⇧⌘] and ⇧⌘[
// the one beside; holding ⌥⌘
// shows the device tiles' digits, releasing hides them, a release before
// the delay shows nothing, a number with no device does nothing, and a key
// pressed during the hold,
// losing the window, or a menu or a sheet opening during it ends it; holding
// ⌥ numbers the Agents rows without moving their time or fold slot, ⌥2
// opens the second row, and in Projects a raised Needs You agent carries its
// number once. A private Herdr server, hided and Electron app.

import { expect, type ElectronApplication, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { labelAgent, setFixtureLifecycle, startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import "../../web/src/host";
import { countSent, enterWorkspace } from "../../web/e2e/wire";
import { hostLog, isolate, launchShell, test, type Isolated } from "./fixture";
import { animationsFinished, quietFor } from "../../web/e2e/wait";
import { chord } from "../../web/e2e/chords";

let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;

test.beforeAll(async () => {
  herdr = await startHerdr();
});

test.afterAll(() => {
  herdr?.stop();
});

test.beforeEach(() => {
  run = isolate(herdr, "digits");
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

/** After the one expected event, a quiet moment in which no second one arrives. */
async function exactlyOnce(sent: Map<string, number>, kind: string, expected: number, page: Page): Promise<void> {
  await expect.poll(() => sent.get(kind) ?? 0).toBe(expected);
  await quietFor(page, 600, "no second event follows the one counted");
  expect(sent.get(kind) ?? 0).toBe(expected);
}

/**
 * `boxes` of what must not move during a hold, plus the title's line (y, height)
 * only: a hold also draws the agent logo left of the title (agent-logo-view-mode
 * B13), so the title moves right while its line, the row and the fold slot stay.
 */
async function held(locators: Locator[], title: Locator): Promise<(string | null)[]> {
  const box = await title.boundingBox();
  return [...(await boxes(locators)), box ? [box.y, box.height].map(Math.round).join(",") : null];
}

/** The boxes of what must not move while a keycap shows, rounded to the device pixel. */
async function boxes(locators: Locator[]): Promise<(string | null)[]> {
  return Promise.all(
    locators.map(async (locator) => {
      const box = await locator.boundingBox();
      return box ? [box.x, box.y, box.width, box.height].map(Math.round).join(",") : null;
    }),
  );
}

/** A quiet moment in which no keycap appears. */
async function noKeycap(page: Page): Promise<void> {
  await quietFor(page, 400, "no keycap appears");
  await expect(page.locator("[data-keycap]")).toHaveCount(0);
}

/**
 * A capture of the app's one window when a run directory is named: the
 * native window image by id where the runner may read the screen, else the
 * renderer's own image, with a sidecar saying which one was taken.
 */
async function capture(page: Page, name: string, source: string): Promise<void> {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `${name}.png`);
  let method = "screencapture -l <window id>";
  try {
    execFileSync("/usr/sbin/screencapture", ["-x", "-o", "-l", source.split(":")[1]!, file], { stdio: "pipe" });
  } catch (error) {
    method = `page.screenshot (screencapture refused: ${error instanceof Error ? error.message.split("\n").at(-1) : String(error)})`;
    await page.screenshot({ path: file });
  }
  fs.writeFileSync(path.join(dir, `${name}.json`), JSON.stringify({ source, method, build: "worktree desktop/dist", isolated: true }));
}

type Opened = { page: Page; sent: Map<string, number>; tabs: string[]; canvas: Locator; tab: (id: string) => Locator; source: string };

/** The app on the fixture workspace with a second tab behind the first, the keyboard in the first tab's terminal. */
async function openTwoTabs(): Promise<Opened> {
  const made = herdr.run([
    "tab", "create", "--workspace", herdr.workspace, "--cwd", path.join(herdr.root, "fixture"), "--label", "second", "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
  ]) as { result: { tab: { tab_id: string } } };
  const tabs = [herdr.tab, made.result.tab.tab_id];
  try {
    const launched = await launchShell(run.env);
    app = launched.app;
    const page = launched.page;
    const sent = countSent(page);
    // Observe the new live socket before it connects, reloading the shell the host has already loaded.
    // The exactly-once expectations below still assert the real outgoing events.
    await Promise.all([
      page.waitForEvent("websocket", { predicate: (socket) => new URL(socket.url()).pathname === "/ws", timeout: 5_000 }),
      page.reload(),
    ]);
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await enterWorkspace(page, "fixture");
    const canvas = page.locator("[data-canvas]").first();
    const tab = (id: string) => page.locator(`[data-tab="${id}"]`);
    await expect(tab(tabs[1]!)).toBeVisible();
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);
    await page.locator("[data-pane-view] .xterm-helper-textarea").first().focus();
    const source = await app.evaluate(({ BrowserWindow }) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error("Expected exactly one candidate window");
      return windows[0]!.getMediaSourceId();
    });
    return { page, sent, tabs, canvas, tab, source };
  } catch (error) {
    herdr.run(["tab", "close", tabs[1]!]);
    throw error;
  }
}

test("⌘n selects a tab, ⌘9 the last, and a ⌘ hold numbers the tabs until an added ⌥ turns the numbers to the devices", async () => {
  const { page, tabs, canvas, tab, source } = await openTwoTabs();
  try {
    // Holding ⌘ alone floats each tab's digit, left to right, and the tabs
    // keep their boxes; no device tile is numbered.
    const tabCaps = page.locator("[data-tab-bar] [data-keycap]");
    const deviceCaps = page.locator("[data-device-rail] [data-keycap]");
    const tabsBefore = await boxes([tab(tabs[0]!), tab(tabs[1]!)]);
    await page.keyboard.down("Meta");
    await expect(tabCaps).toHaveText(["1", "2"]);
    await expect(tab(tabs[1]!).locator("[data-keycap]")).toHaveText("2");
    await expect(deviceCaps).toHaveCount(0);
    expect(await boxes([tab(tabs[0]!), tab(tabs[1]!)])).toEqual(tabsBefore);
    await capture(page, "digit-hints-tabs-dark", source);
    // Adding ⌥ turns the numbers to the device tiles at once, and releasing it turns them back.
    await page.keyboard.down("Alt");
    await expect(deviceCaps).toHaveText(["1"]);
    await expect(tabCaps).toHaveCount(0);
    await page.keyboard.up("Alt");
    await expect(tabCaps).toHaveCount(2);
    await page.keyboard.up("Meta");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);

    // ⌘2 chooses the second tab, ⌘1 the first, and ⌘9 the last.
    await page.keyboard.press("Meta+Digit2");
    await expect(canvas).toHaveAttribute("data-canvas", tabs[1]!);
    await page.keyboard.press("Meta+Digit1");
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);
    await page.keyboard.press("Meta+Digit9");
    await expect(canvas).toHaveAttribute("data-canvas", tabs[1]!);
  } finally {
    herdr.run(["tab", "close", tabs[1]!]);
  }
});

test("⇧⌘] and ⇧⌘[ (Ctrl+PageDown and Ctrl+PageUp off macOS) step to the tab beside, wrapping, and the keyboard follows", async () => {
  const { page, tabs, canvas } = await openTwoTabs();
  try {
    const typing = (id: string) => page.locator(`[data-canvas="${id}"] [data-pane-view][data-focused="true"] .xterm-helper-textarea`);
    await page.keyboard.press(chord("next_tab", "electron"));
    await expect(canvas).toHaveAttribute("data-canvas", tabs[1]!);
    await expect(typing(tabs[1]!)).toBeFocused();
    await page.keyboard.press(chord("next_tab", "electron"));
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);
    await page.keyboard.press(chord("previous_tab", "electron"));
    await expect(canvas).toHaveAttribute("data-canvas", tabs[1]!);
    await page.keyboard.press(chord("previous_tab", "electron"));
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);
    await expect(typing(tabs[0]!)).toBeFocused();
  } finally {
    herdr.run(["tab", "close", tabs[1]!]);
  }
});

test("⌥⌘n selects a device, ⌥n an agent, ⌃n nothing, and holding ⌥⌘ or ⌥ shows the numbers without moving anything", async () => {
  const { page, sent, tabs, canvas, tab, source } = await openTwoTabs();
  try {

    // ⌃n is no chord: it stays the terminal's, and no tab moves.
    const focused = sent.get("focus_tab") ?? 0;
    await page.keyboard.press("Control+Digit2");
    await quietFor(page, 400, "⌃2 is the terminal's");
    expect(sent.get("focus_tab") ?? 0).toBe(focused);
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);

    // B8: a hold released before the delay shows nothing.
    await page.keyboard.down("Meta");
    await page.keyboard.up("Meta");
    await noKeycap(page);

    // B5, B7, B9: holding ⌥⌘ floats each device tile's digit at its top
    // left, This Mac first, and the tile keeps its box; no tab is numbered.
    const tabCaps = page.locator("[data-tab-bar] [data-keycap]");
    const deviceCaps = page.locator("[data-device-rail] [data-keycap]");
    const tile = page.locator("[data-rail-tile]").first();
    const before = await boxes([tile]);
    await page.keyboard.down("Meta");
    await page.keyboard.down("Alt");
    await expect(deviceCaps).toHaveText(["1"]);
    await expect(tabCaps).toHaveCount(0);
    expect(await boxes([tile])).toEqual(before);
    const cap = await deviceCaps.first().boundingBox();
    const owner = await tile.boundingBox();
    expect(cap!.x).toBeGreaterThanOrEqual(owner!.x - 1);
    expect(cap!.y).toBeGreaterThanOrEqual(owner!.y - 1);
    await capture(page, "digit-hints-devices-dark", source);
    // B6: releasing the modifiers hides the numbers at once.
    await page.keyboard.up("Alt");
    await page.keyboard.up("Meta");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);

    // B1: ⌥⌘2 with no second device sends nothing.
    const devicesFocused = sent.get("focus_device") ?? 0;
    await page.keyboard.press("Meta+Alt+Digit2");
    await quietFor(page, 400, "the shortcut with no device sends nothing");
    expect(sent.get("focus_device") ?? 0).toBe(devicesFocused);
    await expect(canvas).toHaveAttribute("data-canvas", tabs[0]!);

    // B6: a key during the hold is the chord itself; the numbers go with it
    // and stay gone while ⌘ is still down.
    await page.keyboard.down("Meta");
    await expect(tabCaps).toHaveCount(2);
    await page.keyboard.press("Digit1");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    await noKeycap(page);
    await page.keyboard.up("Meta");

    // B6: losing the window ends the hold.
    await page.keyboard.down("Meta");
    await expect(tabCaps).toHaveCount(2);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    await page.keyboard.up("Meta");

    // B6: a layer opening during the hold ends it: a tab's context menu,
    // then the Settings sheet, each with ⌘ still down.
    await page.keyboard.down("Meta");
    await expect(tabCaps).toHaveCount(2);
    await tab(tabs[0]!).click({ button: "right" });
    await expect(page.getByRole("menuitem", { name: "Rename…", exact: true })).toBeVisible();
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    await page.keyboard.up("Meta");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menuitem", { name: "Rename…", exact: true })).toHaveCount(0);
    await page.keyboard.down("Meta");
    await expect(tabCaps).toHaveCount(2);
    await page.locator("[data-open-settings]").click();
    await expect(page.locator('[data-settings="true"]')).toBeVisible();
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    await page.keyboard.up("Meta");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);

    // B5, B2: holding ⌥ alone numbers the Agents rows top to bottom; the
    // time and the fold slot stay put; ⌥2 opens the second row.
    const rows = page.locator("nav[data-sidebar] [data-pane]");
    await expect(rows).toHaveCount(2);
    const firstRow = rows.first();
    const kept = [firstRow, firstRow.locator("[data-fold-slot]"), rows.nth(1)];
    const title = firstRow.locator("[data-agent-title]");
    const rowsBefore = await held(kept, title);
    await page.keyboard.down("Alt");
    const rowCaps = page.locator("nav[data-sidebar] [data-keycap]");
    await expect(rowCaps).toHaveCount(2);
    await expect(rowCaps).toHaveText(["1", "2"]);
    expect(await held(kept, title)).toEqual(rowsBefore);
    await capture(page, "digit-hints-agents-dark", source);
    await page.keyboard.up("Alt");
    await expect(page.locator("[data-keycap]")).toHaveCount(0);
    const secondPane = await rows.nth(1).getAttribute("data-pane");
    const opened = sent.get("focus_pane") ?? 0;
    await page.keyboard.press("Alt+Digit2");
    await exactlyOnce(sent, "focus_pane", opened + 1, page);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${secondPane}"] [aria-current="true"]`)).toHaveCount(1);
    await page.keyboard.press("Alt+Digit3");
    await quietFor(page, 400, "the shortcut with no target sends nothing");
    expect(sent.get("focus_pane") ?? 0).toBe(opened + 1);

    // docs/status-model.md: a Needs You agent is raised above the Projects
    // tree and also stays in it; an ⌥ hold shows its number once, on the
    // raised row, and the other agent's on its own row in the tree.
    // The pane ⌥2 opened is in front, where a question is read the moment
    // it arrives; the one behind it asks.
    const asking = herdr.panes.find((pane) => pane !== secondPane)!;
    const other = secondPane!;
    labelAgent(herdr, asking, { task: asking === herdr.panes[0] ? "Agent one" : "Agent two", reply: "계속할까요?", question: true });
    try {
      const raised = page.locator(`[data-raised-group="needs_you"] [data-pane="${asking}"]`);
      await expect(raised).toBeVisible({ timeout: 20_000 });
      const checkoutToggle = page.locator("[data-checkout-toggle]").first();
      if ((await checkoutToggle.getAttribute("aria-expanded")) === "false") await checkoutToggle.click();
      const tree = page.locator("[data-checkout-agents-open]");
      await expect(tree.locator(`[data-pane="${asking}"]`)).toBeVisible();
      await page.keyboard.down("Alt");
      await expect(raised.locator("[data-keycap]")).toHaveText("1");
      await expect(tree.locator(`[data-pane="${other}"] [data-keycap]`)).toHaveText("2");
      await expect(page.locator(":is([data-raised-list], [data-project-list]) [data-keycap]")).toHaveCount(2);
      await capture(page, "digit-hints-projects-raised-dark", source);
      await page.keyboard.up("Alt");
      await expect(page.locator("[data-keycap]")).toHaveCount(0);
    } finally {
      // Answered: the agent runs and stops again, which ends the question.
      await setFixtureLifecycle(herdr, asking, "working");
      await setFixtureLifecycle(herdr, asking, "idle");
    }

    // B7: the keycap draws from the Light tokens too.
    if (process.env.HIDE_E2E_SCREENSHOT_DIR) {
      await page.locator("[data-open-settings]").click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator('[data-theme-option="light"]').click();
      await expect(page.locator("html")).toHaveClass(/\blight\b/);
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
      await animationsFinished(page);
      await page.keyboard.down("Meta");
      await page.keyboard.down("Alt");
      await expect(deviceCaps).toHaveCount(1);
      await capture(page, "digit-hints-devices-light", source);
      await page.keyboard.up("Alt");
      await page.keyboard.up("Meta");
    }
  } finally {
    herdr.run(["tab", "close", tabs[1]!]);
  }
});
