// The terminal probe a desktop e2e reads a pane's screen through, and the
// pointer moves that find and hover a link on it; shared by the specs that
// click terminal links (`terminal-links.spec.ts`, `outside-files.spec.ts`).

import { expect, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { printFileCommand, type HerdrFixture } from "../../web/e2e/herdr-fixture";

export type Probe = { paneId: () => string | null; paneText: (id: string) => string; paneGrid: (id: string) => { cols: number; rows: number } | null; diagnostics?: () => string[] };

/** Reloads the shell with the terminal probe, the only way to read what xterm draws through WebGL. */
export async function withProbe(page: Page): Promise<void> {
  await page.evaluate(() => {
    const url = new URL(location.href);
    url.searchParams.set("probe", "1");
    location.href = url.href;
  });
  await expect
    .poll(() => page.evaluate(() => typeof (window as { __hideProbe?: unknown }).__hideProbe).catch(() => "reloading"), { timeout: 20_000 })
    .toBe("object");
}

/** Prints `lines` in the fixture pane through a program, using the pane's shell. */
export async function printInPane(herdr: HerdrFixture, page: Page, paneId: string, lines: string[], last: string): Promise<void> {
  // Leave the old row before replacing its text so xterm requests fresh links
  // when the next real hover enters it, including after an empty/error answer.
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  // The first column is covered by the pane resize grab, so use the middle.
  const away = await cellPoint(page, paneId, { row: grid.rows - 1, column: Math.floor(grid.cols / 2) });
  expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest(".xterm") != null, away)).toBe(true);
  await page.mouse.move(away.x, away.y);
  const file = path.join(herdr.root, `print-${Date.now()}.txt`);
  fs.writeFileSync(file, `${lines.join("\n")}\n`);
  const command = printFileCommand(herdr.shell, file);
  const sent = spawnSync(herdr.bin, ["pane", "run", herdr.panes[0], command], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status, sent.stderr).toBe(0);
  await expect.poll(() => page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id), paneId), { timeout: 20_000 }).toContain(last);
}

/** Characters xterm draws two cells wide here: Hangul, Han, kana, CJK punctuation and fullwidth forms. */
const WIDE = /[\p{scx=Hangul}\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}\u3000-\u303F\uFF01-\uFF60]/u;

/** Where `text` is on the screen of `paneId`: its row and the column of its `offset`th character. */
export async function cellOf(page: Page, paneId: string, text: string, offset = 0): Promise<{ row: number; column: number }> {
  const lines = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneText(id).split("\n"), paneId);
  const row = lines.findIndex((line) => line.includes(text));
  expect(row, `${text} is not on the screen:\n${lines.join("\n")}`).toBeGreaterThanOrEqual(0);
  const prefix = lines[row]!.slice(0, lines[row]!.indexOf(text)) + text.slice(0, offset);
  const column = [...prefix].reduce((width, char) => width + (WIDE.test(char) ? 2 : 1), 0);
  return { row, column };
}

/** The window point at the middle of a cell of `paneId`'s screen. */
export async function cellPoint(page: Page, paneId: string, cell: { row: number; column: number }): Promise<{ x: number; y: number }> {
  const grid = await page.evaluate((id) => (window as unknown as { __hideProbe: Probe }).__hideProbe.paneGrid(id)!, paneId);
  const box = (await page.locator(`[data-terminal="${paneId}"] .xterm-screen`).boundingBox())!;
  return { x: box.x + ((cell.column + 0.5) * box.width) / grid.cols, y: box.y + ((cell.row + 0.5) * box.height) / grid.rows };
}

export async function pointOf(page: Page, paneId: string, text: string, { offset = 0 } = {}): Promise<{ x: number; y: number }> {
  return cellPoint(page, paneId, await cellOf(page, paneId, text, offset));
}

/** Moves over a point and waits until xterm shows a link there. */
export async function hoverLink(page: Page, paneId: string, point: { x: number; y: number }): Promise<void> {
  await page.mouse.move(point.x - 3, point.y);
  await page.mouse.move(point.x, point.y);
  await expect(page.locator(`[data-terminal="${paneId}"] .xterm-screen`)).toHaveClass(/xterm-cursor-pointer/, { timeout: 10_000 });
}
