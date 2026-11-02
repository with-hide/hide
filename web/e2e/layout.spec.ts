// Tabs, splits, zoom, closes, a closed tab given back and resizes, on the
// engine HIDE_E2E_ENGINE names (PRD engine-bundle-3-core-owns D-14, D-24,
// B37): each test starts its own checkout of two shell panes made through
// the daemon (`engine-fixture.ts`), so the same body runs on the pinned
// Herdr and on the core's own engine. `@own` puts a test in the own engine's
// lane (`web-e2e.yml`, `engine: own`); the Herdr lane runs every test.

import { expect, test, type Page } from "@playwright/test";
import { chord } from "./chords";
import { corePaneFocus, openLayout, startLayoutStack, type LayoutStack } from "./engine-fixture";
import { countSent, screenshot } from "./wire";

test.describe.configure({ timeout: 90_000 });

type Open = { stack: LayoutStack; sent: Map<string, number> };

/** A stack, the page counting what it sends, open on the stack's checkout. */
async function startOpen(page: Page): Promise<Open> {
  await page.setViewportSize({ width: 1680, height: 900 });
  const stack = await startLayoutStack();
  try {
    const sent = countSent(page);
    await openLayout(page, stack);
    return { stack, sent };
  } catch (error) {
    stack.stop();
    throw error;
  }
}

const probePane = (page: Page) => page.evaluate(() => window.__hideProbe?.paneId() ?? null);
const paneText = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
const paneIds = (page: Page) => page.locator("[data-pane-view]").evaluateAll((views) => views.map((view) => view.getAttribute("data-pane-view")!));

/** Puts the keyboard in `pane` and waits until the page and the core both hold it there. */
async function focusPane(page: Page, stack: LayoutStack, pane: string): Promise<void> {
  await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
  await expect.poll(() => probePane(page)).toBe(pane);
  await stack.control.until((rest) => corePaneFocus(rest) === pane, `the core's focus on ${pane}`);
}

/**
 * The split chord on the second pane: a third pane running the fixture
 * shell, which the core focuses. Returns once it has its prompt and the keyboard.
 */
async function splitShell(page: Page, { stack, sent }: Open): Promise<string> {
  await focusPane(page, stack, stack.panes[1]!);
  const before = await paneIds(page);
  await page.keyboard.press(chord("split_right"));
  await expect(page.locator("[data-pane-view]")).toHaveCount(3);
  await expect.poll(() => sent.get("create_pane")).toBe(1);
  const created = (await paneIds(page)).filter((id) => !before.includes(id));
  expect(created).toHaveLength(1);
  const shellPaneId = created[0]!;
  await expect.poll(() => probePane(page)).toBe(shellPaneId);
  await expect(page.locator(`[data-pane-view="${shellPaneId}"]`)).toHaveAttribute("data-transport", /connected|controlling|idle/, { timeout: 15_000 });
  await expect.poll(() => paneText(page, shellPaneId), { timeout: 15_000 }).toContain("fixture %");
  await focusPane(page, stack, shellPaneId);
  return shellPaneId;
}

/**
 * The size the pane's own program reads from its terminal, printed after
 * `marker`: `stty` in the Unix fixture shell, the console window PowerShell
 * reads in the Windows one (cmd.exe has no size command of its own).
 */
async function programSize(page: Page, pane: string, marker: string): Promise<{ rows: number; cols: number }> {
  const ask =
    process.platform === "win32"
      ? `powershell -NoProfile -Command "$s = $Host.UI.RawUI.WindowSize; '${marker} ' + $s.Height + ' ' + $s.Width"`
      : `echo ${marker} $(stty size)`;
  await page.keyboard.type(ask);
  await page.keyboard.press("Enter");
  // The probe's screen text pads each row with spaces to the grid's width.
  const pattern = new RegExp(`^${marker} (\\d+) (\\d+) *$`, "m");
  await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).toMatch(pattern);
  const [, rows, cols] = pattern.exec(await paneText(page, pane))!;
  return { rows: Number(rows), cols: Number(cols) };
}

test("⌘D splits the focused pane, a divider drag sends one resize_pane, and ⌥⇧W closes the focused idle pane without a confirmation", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { sent } = open;
    await splitShell(page, open);
    await expect(page.locator("[data-split]")).toHaveCount(2);
    const boundaries = page.locator("[data-pane-boundary]");
    await expect(boundaries).toHaveCount(2);
    for (const line of await boundaries.all()) {
      const geometry = await line.evaluate((el) => ({ width: el.getBoundingClientRect().width, pointerEvents: getComputedStyle(el).pointerEvents }));
      expect(geometry).toEqual({ width: 1, pointerEvents: "none" });
    }
    await screenshot(page, "layout-two-splits");

    // A divider drag sends one resize_pane on release and the grid follows the engine.
    const outer = page.locator("[data-split=right]").first();
    const divider = outer.locator("> [data-divider]");
    const box = (await divider.boundingBox())!;
    const before = await outer.evaluate((el) => (el as HTMLElement).style.gridTemplateColumns);
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2, { steps: 6 });
    await expect(page.locator("[data-resize-guide]")).toHaveCount(1);
    await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2, { steps: 6 });
    await page.mouse.up();
    await expect(page.locator("[data-resize-guide]")).toHaveCount(0);
    await expect.poll(() => sent.get("resize_pane")).toBe(1);
    await expect.poll(() => outer.evaluate((el) => (el as HTMLElement).style.gridTemplateColumns), { timeout: 10_000 }).not.toBe(before);

    // ⌥⇧W closes the focused idle pane; the engine's new geometry redraws the
    // grid, and the closed pane's terminal is disposed, not parked (D-05).
    const closingPane = (await probePane(page))!;
    await page.keyboard.press(chord("close_pane"));
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await expect.poll(() => sent.get("close_pane")).toBe(1);
    await expect(page.locator("[data-confirm-close]")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => window.__hideProbe?.liveTerminals() ?? [])).not.toContain(closingPane);
  } finally {
    open.stack.stop();
  }
});

test("of three panes the focused one is outlined, and zoom hides the other two behind a chip", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { sent } = open;
    const shellPane = page.locator(`[data-pane-view="${await splitShell(page, open)}"]`);

    // Of three panes, the one holding the keyboard is outlined, and only it.
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(1);
    await expect(shellPane.locator("[data-pane-focus-outline]")).toBeVisible();
    await expect(page.locator("[data-pane-boundary]")).toHaveCount(2);

    await page.keyboard.press(chord("toggle_zoom"));
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "true");
    await expect(page.locator("[data-pane-boundary]")).toHaveCount(0);
    await expect.poll(() => sent.get("toggle_zoom")).toBe(1);
    // Zoom keeps the input perimeter and names the two panes it hides.
    await expect(page.locator("[data-pane-zoom]")).toHaveAttribute("data-pane-zoom", "2");
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(1);
    await expect(shellPane.locator("[data-pane-focus-outline]")).toBeVisible();
    await screenshot(page, "layout-zoom-chip");
    await page.locator("[data-pane-zoom]").click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "false");
    await expect(page.locator("[data-pane-boundary]")).toHaveCount(2);
    await expect.poll(() => sent.get("toggle_zoom")).toBe(2);
    await expect(page.locator('[data-pane-zoom]:not([data-pane-zoom="false"])')).toHaveCount(0);
  } finally {
    open.stack.stop();
  }
});

test("a right-click focuses its pane and opens the terminal menu, whose Zoom pane fills the canvas", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { stack, sent } = open;
    const shellPaneId = await splitShell(page, open);
    const shellPane = page.locator(`[data-pane-view="${shellPaneId}"]`);
    const otherPane = stack.panes[0]!;

    // A right-click selects its pane, but the menu holds the keyboard until
    // it closes, so no canvas carries input emphasis while it is open.
    const otherView = page.locator(`[data-pane-view="${otherPane}"]`);
    await otherView.locator("[data-terminal-host]").click({ button: "right", position: { x: 40, y: 40 } });
    await expect(page.getByRole("menu")).toBeVisible();
    await expect(otherView).toHaveAttribute("data-focused", "true");
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await stack.control.until((rest) => corePaneFocus(rest) === otherPane, `the core's focus on ${otherPane}`);
    await expect(otherView.locator("[data-pane-focus-outline]")).toBeVisible();
    await focusPane(page, stack, shellPaneId);

    // A right-click in the terminal opens the editing, layout and pane menu.
    await shellPane.locator("[data-terminal-host]").click({ button: "right", position: { x: 40, y: 40 } });
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    for (const item of ["Paste", "Select all", "Find", "Split right", "Split down", "Zoom terminal", "Copy name and ID"]) {
      await expect(menu.getByRole("menuitem", { name: item, exact: false }).first()).toBeVisible();
    }
    await expect(page.locator("[data-pane-focus-outline]")).toHaveCount(0);
    await screenshot(page, "layout-terminal-menu");
    await menu.getByRole("menuitem", { name: /^Zoom terminal/ }).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "true");
    await expect.poll(() => sent.get("toggle_zoom")).toBe(1);
    // The zoomed pane takes the whole canvas, not just its own split cell.
    const canvasBox = (await page.locator("[data-canvas]").boundingBox())!;
    const zoomedBox = (await page.locator('[data-pane-view][data-focused="true"]').boundingBox())!;
    expect(Math.abs(zoomedBox.width - canvasBox.width)).toBeLessThan(2);
    expect(Math.abs(zoomedBox.height - canvasBox.height)).toBeLessThan(2);
    expect(Math.abs(zoomedBox.x - canvasBox.x)).toBeLessThan(2);
    await page.keyboard.press(chord("toggle_zoom"));
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-zoomed", "false");
    await expect(page.locator("[data-pane-view]")).toHaveCount(3);
  } finally {
    open.stack.stop();
  }
});

/** New tab from the second pane: returns the tab, once its shell has drawn its prompt in front. */
async function newTab(page: Page, { stack, sent }: Open): Promise<string> {
  await focusPane(page, stack, stack.panes[1]!);
  await page.keyboard.press(chord("new_tab"));
  await expect(page.locator("[data-tab]")).toHaveCount(2);
  await expect.poll(() => sent.get("agent_group.new_tab")).toBe(1);
  const tab = (await page.locator("[data-tab]").evaluateAll((tabs) => tabs.map((el) => el.getAttribute("data-tab")!))).find((id) => id !== stack.tab)!;
  await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab);
  await expect(page.locator("[data-pane-view]")).toHaveCount(1);
  const pane = (await paneIds(page))[0]!;
  await expect.poll(() => paneText(page, pane), { timeout: 15_000 }).toContain("fixture %");
  return tab;
}

test("New tab opens a terminal in front, a tab click switches back, and its close control closes it", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { stack, sent } = open;
    const tab = await newTab(page, open);

    await page.locator(`[data-tab="${stack.tab}"]`).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", stack.tab);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await page.locator(`[data-tab="${tab}"]`).click();
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", tab);

    // The tab close control closes the visible tab (an idle pane needs no confirmation).
    await page.locator(`[data-tab="${tab}"] button`).click();
    await expect.poll(() => sent.get("close_tab")).toBe(1);
    await expect(page.locator("[data-tab]")).toHaveCount(1);
    await expect(page.locator(`[data-tab="${tab}"]`)).toHaveCount(0);
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", stack.tab);
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
  } finally {
    open.stack.stop();
  }
});

test("Reopen closed tab gives back the tab just closed with a working shell", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { stack, sent } = open;
    const tab = await newTab(page, open);
    await page.locator(`[data-tab="${tab}"] button`).click();
    await expect(page.locator("[data-tab]")).toHaveCount(1);
    // Reopen does nothing until the core offers a close to reopen.
    await stack.control.until((rest) => (rest.recent_closed as { can_reopen?: boolean } | undefined)?.can_reopen === true, "a closed tab to reopen");
    await focusPane(page, stack, stack.panes[1]!);

    await page.keyboard.press(chord("reopen_closed_tab"));
    await expect.poll(() => sent.get("reopen_closed")).toBe(1);
    await expect(page.locator("[data-tab]")).toHaveCount(2);
    const reopened = (await page.locator("[data-tab]").evaluateAll((tabs) => tabs.map((el) => el.getAttribute("data-tab")!))).find((id) => id !== stack.tab)!;
    await expect(page.locator("[data-canvas]")).toHaveAttribute("data-canvas", reopened, { timeout: 15_000 });
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const pane = (await paneIds(page))[0]!;
    await expect.poll(() => paneText(page, pane), { timeout: 15_000 }).toContain("fixture %");
    await focusPane(page, stack, pane);
    expect(await programSize(page, pane, "reopened")).toEqual(await page.evaluate((id) => window.__hideProbe!.paneGrid(id), pane));
  } finally {
    open.stack.stop();
  }
});

test("a narrower window reaches each pane's program as a narrower terminal", { tag: ["@platform", "@own"] }, async ({ page }) => {
  const open = await startOpen(page);
  try {
    const { stack } = open;
    const [left, right] = stack.panes as [string, string];
    const grid = (pane: string) => page.evaluate((id) => window.__hideProbe!.paneGrid(id), pane);
    await focusPane(page, stack, left);
    const wide = await programSize(page, left, "wide");
    expect(wide).toEqual(await grid(left));

    await page.setViewportSize({ width: 1200, height: 900 });
    await expect.poll(async () => (await grid(left))!.cols).toBeLessThan(wide.cols);
    await expect.poll(async () => (await grid(right))!.cols).toBeLessThan(wide.cols);
    // The engine sizes the program's terminal to the grid the screen drew.
    expect(await programSize(page, left, "narrow")).toEqual(await grid(left));
    await focusPane(page, stack, right);
    expect(await programSize(page, right, "narrow")).toEqual(await grid(right));
  } finally {
    open.stack.stop();
  }
});
