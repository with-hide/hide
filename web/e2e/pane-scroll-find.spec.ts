// Agent pane scroll beside another client, Cmd+F in the focused pane, and
// the lane staying on its tree when its last view closes (PRD
// web-pane-scroll-find-areas, right-lane B12),
// on an isolated pinned Herdr. The other client is a plain `herdr terminal
// session control` started before hided, holding control the way another
// client does on the operator's server; hided then falls back to observing.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, setLanePinned, showExplorer } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

function herdrCli(herdr: HerdrFixture, args: string[]): string {
  return execFileSync(herdr.bin, args, { env: herdr.env, encoding: "utf8", timeout: 30_000 });
}

/** Herdr's own scroll position for a pane, lines above the bottom. */
function scrollOffset(herdr: HerdrFixture, pane: string): number {
  const answer = herdr.run(["pane", "get", pane]) as { result: { pane: { scroll?: { offset_from_bottom: number } } } };
  return answer.result.pane.scroll?.offset_from_bottom ?? -1;
}

async function paneText(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
}

/** The last rows sit on adjacent lines: a frame drawn at another grid wraps each row onto two. */
const DRAWN_AT_GRID = /row-159 *\nrow-160/;

test("an observed agent pane scrolls, Cmd+F finds in the focused pane, and closing the last view leaves the tree in the lane", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  let other: ChildProcess | null = null;
  try {
    const [observed, focused] = herdr.panes;
    // Long history in both agent panes: the shim echoes what the PTY gets.
    const rows = Array.from({ length: 160 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`).join("\n");
    for (const pane of herdr.panes) herdrCli(herdr, ["pane", "send-text", pane, `${rows}\n`]);
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.txt"), "notes\n");
    // Another client holds control of the first pane, at its own grid.
    other = spawn(herdr.bin, ["terminal", "session", "control", observed, "--cols", "70", "--rows", "20"], {
      env: herdr.env,
      stdio: ["pipe", "ignore", "ignore"],
    });
    daemon = await startHided(herdr, "scroll-find");
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B1, B2: the pane is observed, not dropped; its wheel moves Herdr's
    // viewport and the frame at the view's own grid shows older rows.
    const observedView = page.locator(`[data-pane-view="${observed}"]`);
    await expect(observedView).toHaveAttribute("data-transport", "observing", { timeout: 20_000 });
    await expect.poll(() => paneText(page, observed), { timeout: 15_000 }).toContain("row-160");
    expect(scrollOffset(herdr, observed)).toBe(0);
    await observedView.locator("[data-terminal-host]").hover();
    await page.mouse.wheel(0, -900);
    await expect.poll(() => scrollOffset(herdr, observed), { timeout: 10_000 }).toBeGreaterThan(0);
    await expect.poll(() => paneText(page, observed), { timeout: 10_000 }).not.toContain("row-160");
    await expect(observedView.locator("header")).not.toContainText("starting");
    await screenshot(page, "pane-scroll-observed-up");
    await page.mouse.wheel(0, 4000);
    await expect.poll(() => scrollOffset(herdr, observed), { timeout: 10_000 }).toBe(0);
    await expect.poll(() => paneText(page, observed), { timeout: 10_000 }).toContain("row-160");

    // B4: with a document open beside the agents, Cmd+F in a focused pane
    // opens that pane's find bar, not the document's. The lane is pinned, so
    // it docks beside them (PRD right-lane B5).
    await showExplorer(page);
    await setLanePinned(page, true);
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await page.locator(`[data-explorer-row$="/notes.txt"]`).dblclick();
    await expect(page.locator("[data-view-area]")).toBeVisible();
    // Both panes narrow and redraw at the new grid, the observed one by
    // attaching again at it.
    for (const pane of herdr.panes) await expect.poll(() => paneText(page, pane), { timeout: 15_000 }).toMatch(DRAWN_AT_GRID);
    await page.locator(`[data-pane-view="${focused}"] [data-terminal-host]`).click();
    await expect(page.locator(`[data-pane-view="${focused}"]`)).toHaveAttribute("data-focused", "true");
    await page.keyboard.press(chord("find_in_pane"));
    const bar = page.locator("[data-find-bar]");
    await expect(bar).toBeVisible();
    await expect(bar.locator("input")).toBeFocused();
    await expect(page.locator(".cm-search")).toHaveCount(0);

    // B5: Enter searches and steps, Shift+Enter steps back, and the count is
    // the core's 1-based position.
    await bar.locator("input").fill("row-15");
    await page.keyboard.press("Enter");
    await expect(bar).toContainText(/1\/1[0-9]/);
    await page.keyboard.press("Enter");
    await expect(bar).toContainText(/2\/1[0-9]/);
    await page.keyboard.press("Shift+Enter");
    await expect(bar).toContainText(/1\/1[0-9]/);
    await screenshot(page, "pane-find-bar");
    await bar.locator("input").fill("no-such-row");
    await page.keyboard.press("Enter");
    await expect(bar).toContainText("0/0");
    expect(sent.get("pane_find") ?? 0).toBeGreaterThan(0);
    expect(last.get("pane_find")).toMatchObject({ pane_id: focused });

    // Escape ends the search and gives the keyboard back to the pane.
    await page.keyboard.press("Escape");
    await expect(bar).toHaveCount(0);
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.closest("[data-pane-view]")?.getAttribute("data-pane-view") ?? null))
      .toBe(focused);

    // B7: inside the View area Cmd+F stays the document's find.
    await page.locator("[data-editor-body] .cm-content").click();
    await page.keyboard.press(chord("find_in_pane"));
    await expect(page.locator(".cm-search")).toBeVisible();
    await expect(bar).toHaveCount(0);
    await page.keyboard.press("Escape");

    // Closing the last view keeps the docked lane with the Explorer alone in
    // its column, so the next file brings the View areas back beside it (PRD
    // lane-two-columns B12).
    await expect(workspace).toHaveAttribute("data-lane-view", "documents");
    const viewTab = page.locator('[data-view-tab-bar] [role="tab"][data-display]').first();
    await viewTab.hover();
    await viewTab.getByRole("button", { name: /Close view/ }).click();
    await expect(page.locator("[data-view-area]")).toHaveCount(0);
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(workspace).toHaveAttribute("data-lane-view", "none");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await screenshot(page, "view-region-gone");
    await page.locator(`[data-explorer-row$="/notes.txt"]`).dblclick();
    await expect(page.locator("[data-view-area]")).toBeVisible();
    await expect(page.locator("[data-agent-area]")).toBeVisible();
  } finally {
    other?.kill();
    daemon?.stop();
    herdr.stop();
  }
});
