// A drag selection in an agent pane keeps the text it covers while the wheel
// scrolls the holder's viewport (the pane has no local scrollback, so the frame
// is redrawn in place under the selection), a drag past the screen's edge
// scrolls the history and copies every row it passed, and a program that
// tracks the mouse gets the drag instead, unless shift is held.

import fs from "node:fs";
import os from "node:os";
import { expect, test, type Page } from "@playwright/test";
import { mod } from "./chords";
import { sendKeys, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { dumpOnFailure } from "./failure-dump";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace } from "./wire";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

const paneText = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
const selection = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneSelection(id) ?? null, pane);

test("a selection stays on the text it covers when the wheel scrolls the pane", async ({ page, context }) => {
  const lastSent = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, lastSent);
  await page.setViewportSize({ width: 1680, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    const rows = Array.from({ length: 160 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`).join("\n");
    daemon = await startHided(engine, "selection-scroll");
    const [pane] = engine.panes;
    await withControl(daemon, async (control) => sendKeys(control, pane, `${rows}\n`));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect.poll(() => paneText(page, pane), { timeout: 20_000 }).toContain("row-160");

    const screen = (await page.locator(`[data-pane-view="${pane}"] .xterm-screen`).boundingBox())!;
    const grid = (await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, pane))!;
    const cell = (column: number, row: number) => ({
      x: screen.x + (column + 0.5) * (screen.width / grid.cols),
      y: screen.y + (row + 0.5) * (screen.height / grid.rows),
    });
    const from = cell(1, 4);
    const to = cell(12, 6);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(to.x, to.y, { steps: 6 });
    await page.mouse.up();
    await expect.poll(() => selection(page, pane)).toMatch(/row-\d+/);
    const before = (await selection(page, pane))!;
    const textBefore = await paneText(page, pane);

    // Scrolled up, the selection moves down with its text.
    await page.mouse.move(cell(20, 10).x, cell(20, 10).y);
    await page.mouse.wheel(0, -3 * (screen.height / grid.rows));
    await dumpOnFailure("first wheel", async () => ({
      before: textBefore,
      after: await paneText(page, pane),
      selection: await selection(page, pane),
      grid: await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, pane),
      focused: await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"),
      scrollCount: sent.get("terminal_scroll") ?? 0,
      lastScroll: lastSent.get("terminal_scroll"),
      snapshot: await withControl(daemon!, (control) => control.until((rest) => rest, "the failure snapshot")),
    }), () => expect.poll(() => paneText(page, pane), { timeout: 10_000 }).not.toBe(textBefore));
    await expect.poll(() => selection(page, pane)).toBe(before);

    // Scrolled back down, it returns with the text.
    await page.mouse.wheel(0, 3 * (screen.height / grid.rows));
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).toBe(textBefore);
    await expect.poll(() => selection(page, pane)).toBe(before);

    // Crossing the edge clips only the drawing, never the original selection.
    // Returning while part remains visible restores its complete highlight.
    const highlight = page.locator(`[data-pane-view="${pane}"] .xterm-selection div`);
    const drawn = () => highlight.evaluateAll((nodes) => nodes.map((node) => node.getAttribute("style")));
    await expect.poll(() => highlight.count()).toBeGreaterThan(0);
    const originalHighlight = await drawn();
    await page.mouse.wheel(0, -(grid.rows - 6) * (screen.height / grid.rows));
    await expect.poll(() => paneText(page, pane)).not.toBe(textBefore);
    await expect.poll(() => selection(page, pane)).toBe(before);
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    // Menu Copy preserves the selection on every OS; Ctrl+Shift+C clears it.
    await page.mouse.click(cell(20, 10).x, cell(20, 10).y, { button: "right" });
    await page.locator('[data-menu-item="copy"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(before);
    await page.mouse.wheel(0, (grid.rows - 6) * (screen.height / grid.rows));
    await expect.poll(() => paneText(page, pane)).toBe(textBefore);
    await expect.poll(drawn).toEqual(originalHighlight);

    // Several wheels in a row, answered by several frames, move it each time.
    for (let turn = 0; turn < 4; turn += 1) await page.mouse.wheel(0, -(screen.height / grid.rows));
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).not.toBe(textBefore);
    await expect.poll(() => selection(page, pane), { timeout: 10_000 }).toBe(before);
    for (let turn = 0; turn < 4; turn += 1) await page.mouse.wheel(0, screen.height / grid.rows);
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).toBe(textBefore);
    await expect.poll(() => selection(page, pane)).toBe(before);

    // Scrolled until its text leaves the screen, nothing is drawn selected on
    // other text, and Copy still takes the text that was selected.
    await page.mouse.wheel(0, -(grid.rows + 6) * (screen.height / grid.rows));
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).not.toContain("row-12");
    await expect(page.locator(`[data-pane-view="${pane}"] .xterm-selection div`)).toHaveCount(0);
    expect(await selection(page, pane)).toBe(before);
    await page.keyboard.press(mod("KeyC"));
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(before);

    // A click in the pane ends it.
    await page.mouse.click(cell(20, 10).x, cell(20, 10).y);
    await expect.poll(() => selection(page, pane)).toBeNull();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

// @platform: The holder's history and its view of it (OSC 7721) on the platform's PTY, and the system's copy chord.
test("a drag past the top edge scrolls the history and its copy holds every row it passed", { tag: "@platform" }, async ({ page, context }) => {
  const lastSent = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, lastSent);
  await page.setViewportSize({ width: 1680, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    const rows = Array.from({ length: 160 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`).join("\n");
    daemon = await startHided(engine, "selection-autoscroll");
    const [pane] = engine.panes;
    await withControl(daemon, async (control) => sendKeys(control, pane, `${rows}\n`));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect.poll(() => paneText(page, pane), { timeout: 20_000 }).toContain("row-160");
    const view = () => page.evaluate((id) => window.__hideProbe?.paneView(id) ?? null, pane);
    await expect.poll(async () => (await view())?.bottom ?? 0).toBeGreaterThan(0);

    const screen = (await page.locator(`[data-pane-view="${pane}"] .xterm-screen`).boundingBox())!;
    const grid = (await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, pane))!;
    const rowHeight = screen.height / grid.rows;
    const rowAt = (row: number) => screen.y + (row + 0.5) * rowHeight;
    // The press is on the last row that names one, at its start.
    const shown = (await paneText(page, pane)).split("\n");
    const pressRow = shown.findLastIndex((line) => /^row-\d{3}/.test(line));
    const pressed = shown[pressRow]!.slice(0, 7);
    await page.mouse.move(screen.x + 1, rowAt(pressRow));
    await page.mouse.down();
    // Held above the screen, the drag scrolls toward the oldest row.
    await page.mouse.move(screen.x + 1, screen.y - 3 * rowHeight, { steps: 6 });
    await expect.poll(async () => (await view())?.top ?? -1, { timeout: 20_000 }).toBe(0);
    await page.mouse.up();
    expect(sent.get("terminal_mouse") ?? 0).toBe(0);
    expect(sent.get("terminal_scroll") ?? 0).toBeGreaterThan(0);

    // Every row from the first to the pressed one, none missed or repeated.
    const copied = (await selection(page, pane))!;
    const named = copied.split("\n").filter((line) => /^row-\d{3}$/.test(line));
    const last = Number(pressed.slice(4));
    expect(named).toEqual(Array.from({ length: last - 1 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`));
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.keyboard.press(mod("KeyC"));
    // The Windows text clipboard represents line endings as CRLF.
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(copied.split("\n").join(os.EOL));
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

// @platform: Mouse modes a program sets through the platform's PTY, and the press, motion and release its holder reports to it.
test("a program that tracks the mouse gets the drag, and a drag with shift selects instead", { tag: "@platform" }, async ({ page }) => {
  const lastSent = new Map<string, Record<string, unknown>>();
  const sent = countSent(page, lastSent);
  await page.setViewportSize({ width: 1680, height: 900 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "program-drag");
    const [pane] = engine.panes;
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect.poll(() => paneText(page, pane), { timeout: 20_000 }).toContain("claude fixture ready");
    // The fixture agent writes back what it reads, so these modes are its own
    // output: it tracks a held button's motion, in SGR form.
    await withControl(daemon, async (control) => sendKeys(control, pane, "\x1b[?1002h\x1b[?1006h"));
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneView(id)?.mouse ?? null, pane), { timeout: 10_000 }).toBe(true);

    const screen = (await page.locator(`[data-pane-view="${pane}"] .xterm-screen`).boundingBox())!;
    const grid = (await page.evaluate((id) => window.__hideProbe?.paneGrid(id) ?? null, pane))!;
    const cell = (column: number, row: number) => ({
      x: screen.x + (column + 0.5) * (screen.width / grid.cols),
      y: screen.y + (row + 0.5) * (screen.height / grid.rows),
    });
    const log = engine.inputLogs[0];
    const before = fs.readFileSync(log).length;
    await page.mouse.move(cell(2, 3).x, cell(2, 3).y);
    await page.mouse.down();
    await page.mouse.move(cell(9, 5).x, cell(9, 5).y, { steps: 4 });
    await page.mouse.up();
    // A press, motion with the button held, and the release where it ended.
    const report = new RegExp(`^${"\u001b"}\\[<0;3;4M(${"\u001b"}\\[<32;\\d+;\\d+M)+${"\u001b"}\\[<0;10;6m$`);
    await expect.poll(() => fs.readFileSync(log, "latin1").slice(before), { timeout: 10_000 }).toMatch(report);
    expect(await selection(page, pane)).toBeNull();
    expect(lastSent.get("terminal_mouse")).toMatchObject({ action: "release", button: "left", column: 9, row: 5 });

    // With shift held the drag is the operator's selection, and the program hears nothing.
    const mice = sent.get("terminal_mouse") ?? 0;
    const heard = fs.readFileSync(log).length;
    await page.keyboard.down("Shift");
    await page.mouse.move(cell(0, 0).x, cell(0, 0).y);
    await page.mouse.down();
    await page.mouse.move(cell(8, 0).x, cell(8, 0).y, { steps: 4 });
    await page.mouse.up();
    await page.keyboard.up("Shift");
    await expect.poll(() => selection(page, pane)).not.toBeNull();
    expect(sent.get("terminal_mouse") ?? 0).toBe(mice);
    expect(fs.readFileSync(log).length).toBe(heard);

    // A program that asks for pixels (mode 1016), as Claude Code does, hears
    // a drag past the pane's top edge above it, in the holder's 10 by 20
    // pixel cells, told at a cell's middle.
    await withControl(daemon, async (control) => sendKeys(control, pane, "\x1b[?1016hpixels-on"));
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).toContain("pixels-on");
    const pixelsFrom = fs.readFileSync(log).length;
    await page.mouse.move(cell(2, 1).x, cell(2, 1).y);
    await page.mouse.down();
    await page.mouse.move(cell(2, 1).x, screen.y - 2.5 * (screen.height / grid.rows), { steps: 4 });
    await page.mouse.up();
    const esc = "\u001b";
    const pixels = new RegExp(`^${esc}\\[<0;25;30M(${esc}\\[<32;25;-?\\d+M)*${esc}\\[<32;25;-50M${esc}\\[<0;25;-50m$`);
    await expect.poll(() => fs.readFileSync(log, "latin1").slice(pixelsFrom), { timeout: 10_000 }).toMatch(pixels);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
