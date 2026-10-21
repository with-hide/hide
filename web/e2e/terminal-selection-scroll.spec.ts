// A drag selection in an agent pane keeps the text it covers while the wheel
// scrolls Herdr's viewport (the pane has no local scrollback, so the frame is
// redrawn in place under the selection), on an isolated pinned Herdr.

import { expect, test, type Page } from "@playwright/test";
import { mod } from "./chords";
import { execFileSync } from "node:child_process";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 120_000 });
test.use({ actionTimeout: 15_000 });

function herdrCli(herdr: HerdrFixture, args: string[]): string {
  return execFileSync(herdr.bin, args, { env: herdr.env, encoding: "utf8", timeout: 30_000 });
}

const paneText = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
const selection = (page: Page, pane: string) => page.evaluate((id) => window.__hideProbe?.paneSelection(id) ?? null, pane);

test("a selection stays on the text it covers when the wheel scrolls the pane", async ({ page, context }) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const [pane] = herdr.panes;
    const rows = Array.from({ length: 160 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`).join("\n");
    herdrCli(herdr, ["pane", "send-text", pane, `${rows}\n`]);
    daemon = await startHided(herdr, "selection-scroll");
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
    await expect.poll(() => paneText(page, pane), { timeout: 10_000 }).not.toBe(textBefore);
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
    herdr.stop();
  }
});
