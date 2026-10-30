// Nerd Font icons and CJK text drawn from fallbacks fitted to the terminal's
// cells (`src/terminalFonts.ts`), on the private daemon and a real PTY. The
// suite's DOM renderer lays cells out from the same measure as WebGL; a
// capture of software WebGL comes back blank here, so the pixels are read
// from the DOM rows. The icon comes from the bundled font, so its
// fit holds on every system; whether a CJK glyph is drawn depends on the
// runner's fonts, so only its height is held here.
import { expect, test, type Locator, type Page } from "@playwright/test";
import path from "node:path";
import { chord } from "./chords";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.use({ deviceScaleFactor: 2 });

/** Material Design's target, U+F04FE: its ink fills the whole 1 em box, which ran 1.6 cells wide unfitted. */
const TARGET = String.fromCodePoint(0xf04fe);
/** 가나, two syllables of two cells each. */
const HANGUL = String.fromCodePoint(0xac00, 0xb098);
/** Typed into the fixture's agent, which copies its input to its output, after an empty line, so the first line of each has a blank row above it. */
const SAMPLE = ["", `${TARGET} ${TARGET} |ICONS`, "", `${HANGUL}|HANGUL`, "0123456789|RULER"];

type Ink = { columns: number[]; rows: number[]; boxes: { width: number; height: number }[] };

/**
 * The cells of the terminal's screen that hold ink, from a capture of it:
 * per column of `row`, the share of the cell's width inked, and per row the
 * share of `columns`' pixels inked.
 */
async function inkOf(page: Page, screen: Locator, grid: { cols: number; rows: number }, row: number, columns: number): Promise<Ink> {
  const png = (await screen.screenshot()).toString("base64");
  return page.evaluate(async ({ png, grid, row, columns }) => {
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d")!;
    context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, image.width, image.height).data;
    const cellWidth = image.width / grid.cols;
    const cellHeight = image.height / grid.rows;
    // The screen's own background, read from its last column.
    const at = (x: number, y: number) => (Math.floor(y) * image.width + Math.floor(x)) * 4;
    const ground = at(image.width - 2, cellHeight * row + cellHeight / 2);
    const inked = (x: number, y: number) => {
      const index = at(x, y);
      return [0, 1, 2].some((channel) => Math.abs(pixels[index + channel]! - pixels[ground + channel]!) > 48);
    };
    // The focused pane's ring overlaps the screen's left edge.
    const ring = Math.ceil(2 * window.devicePixelRatio);
    const share = (x0: number, x1: number, y0: number, y1: number, across: boolean) => {
      const hit = new Set<number>();
      for (let x = Math.max(ring, Math.floor(x0)); x < Math.floor(x1); x += 1)
        for (let y = Math.floor(y0); y < Math.floor(y1); y += 1) if (inked(x, y)) hit.add(across ? x : y);
      return hit.size / Math.floor(across ? x1 - x0 : y1 - y0);
    };
    // Per column of `row`, the box its ink fills.
    const box = (column: number) => {
      let left = Infinity, right = -Infinity, top = Infinity, bottom = -Infinity;
      for (let y = Math.floor(row * cellHeight); y < Math.floor((row + 1) * cellHeight); y += 1)
        for (let x = Math.max(ring, Math.floor(column * cellWidth)); x < Math.floor((column + 1) * cellWidth); x += 1)
          if (inked(x, y)) { left = Math.min(left, x); right = Math.max(right, x); top = Math.min(top, y); bottom = Math.max(bottom, y); }
      return right < left ? { width: 0, height: 0 } : { width: right - left + 1, height: bottom - top + 1 };
    };
    return {
      boxes: Array.from({ length: columns }, (_, column) => box(column)),
      columns: Array.from({ length: columns }, (_, column) =>
        share(column * cellWidth, (column + 1) * cellWidth, row * cellHeight, (row + 1) * cellHeight, true)),
      rows: [row - 1, row].map((each) => share(0, columns * cellWidth, each * cellHeight, (each + 1) * cellHeight, false)),
    };
  }, { png, grid, row, columns });
}

async function start(page: Page) {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  const stop = () => {
    try { daemon?.stop(); } finally { herdr.stop(); }
  };
  try {
    daemon = await startHided(herdr, "terminal-fonts");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const view = page.locator('[data-pane-view][data-focused="true"]');
    const paneId = (await view.getAttribute("data-pane-view"))!;
    await view.locator(".xterm-helper-textarea").focus();
    for (const line of SAMPLE) {
      await page.keyboard.type(line);
      await page.keyboard.press("Enter");
    }
    await expect.poll(() => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", paneId), { timeout: 15_000 }).toContain("|RULER");
    return { page, paneId, view, stop };
  } catch (error) {
    stop();
    throw error;
  }
}

/** The row the sample line ending in `marker` is drawn on, and the grid. */
async function rowOf(page: Page, paneId: string, marker: string) {
  return page.evaluate(({ id, marker }) => {
    const lines = window.__hideProbe!.paneText(id).split("\n");
    // The terminal's own echo of the typed line, with the blank row before it.
    return { row: lines.findIndex((line) => line.includes(marker)), grid: window.__hideProbe!.paneGrid(id)! };
  }, { id: paneId, marker });
}

/** Two frames, so the WebGL canvas holds the rows the probe just read and the fitted fonts' redraw. */
const painted = (page: Page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));

test("a Nerd Font icon stays in its cell and a CJK glyph in its row", async ({ page }) => {
  const { paneId, view, stop } = await start(page);
  try {
    const screen = view.locator(".xterm-screen");
    const icons = await rowOf(page, paneId, "|ICONS");
    await painted(page);
    if (process.env.HIDE_E2E_SCREENSHOT_DIR) await view.screenshot({ path: path.join(process.env.HIDE_E2E_SCREENSHOT_DIR, "terminal-fonts-default.png") });
    // Icon, space, icon, space, bar.
    let ink = await inkOf(page, screen, icons.grid, icons.row, 5);
    await expect.poll(async () => {
      await painted(page);
      ink = await inkOf(page, screen, icons.grid, icons.row, 5);
      return ink.columns[0]!;
    }, { message: "the icon is drawn across its cell" }).toBeGreaterThan(0.5);
    expect(ink.columns[1], "nothing of the icon reaches the space after it").toBe(0);
    expect(ink.columns[2]).toBeGreaterThan(0.5);
    expect(ink.columns[3]).toBe(0);
    // The target is a circle in a square: drawn from the icon font it is as
    // high as it is wide, where a missing-glyph box is a tall rectangle.
    const target = ink.boxes[2]!;
    expect(Math.abs(target.height - target.width), `the icon is the target, ${target.width}x${target.height}`).toBeLessThanOrEqual(target.width * 0.2);
    const hangul = await rowOf(page, paneId, "|HANGUL");
    const cjk = await inkOf(page, screen, hangul.grid, hangul.row, 4);
    expect(cjk.rows[0], "nothing of the CJK glyphs rises into the blank row above").toBe(0);
  } finally {
    stop();
  }
});

test("a Terminal font this machine does not have is said so and keeps the default cell", async ({ page }) => {
  const { paneId, stop } = await start(page);
  try {
    const before = await page.evaluate((id) => window.__hideProbe!.paneGrid(id), paneId);
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    const field = page.locator('[data-terminal-font="true"]');
    await field.fill("No Such Font 0");
    await field.press("Enter");
    await expect(page.locator('[data-terminal-font-missing="true"]')).toBeVisible();
    await expect(field).toHaveValue("No Such Font 0");
    await screenshot(page, "terminal-font-missing");
    expect(await page.evaluate((id) => window.__hideProbe!.paneGrid(id), paneId)).toEqual(before);
    await field.fill("");
    await field.press("Enter");
    await expect(page.locator('[data-terminal-font-missing="true"]')).toHaveCount(0);
  } finally {
    stop();
  }
});
