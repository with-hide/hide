// Agent pane scroll beside another client, Cmd+F in the focused pane, and
// the lane staying on its tree when its last view closes (PRD
// web-pane-scroll-find-areas, right-lane B12).

import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { sendKeys, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, setLanePinned, showExplorer } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

async function paneText(page: Page, pane: string): Promise<string> {
  return page.evaluate((id) => window.__hideProbe?.paneText(id) ?? "", pane);
}

/** The last rows sit on adjacent lines: a frame drawn at another grid wraps each row onto two. */
const DRAWN_AT_GRID = /row-159 *\nrow-160/;

type Docked = { page: Page; daemon: Daemon; fixture: AgentFixture; focused: string; workspace: Locator; sent: Map<string, number>; last: Map<string, Record<string, unknown>> };

/**
 * The fixture's two agents with long history, the Explorer pinned beside them
 * and `notes.txt` open in a View area, the keyboard in the second pane.
 */
async function dockedWithDocument(page: Page, label: string, use: (docked: Docked) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1680, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(fixture.root, "fixture", "notes.txt"), "notes\n");
    daemon = await startHided(fixture, label);
    const focused = fixture.panes[1];
    // Long history in both agent panes: the shim echoes what the PTY gets.
    const rows = Array.from({ length: 160 }, (_, index) => `row-${String(index + 1).padStart(3, "0")}`).join("\n");
    await withControl(daemon, async (control) => {
      for (const pane of fixture.panes) sendKeys(control, pane, `${rows}\n`);
    });
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    for (const pane of fixture.panes) await expect.poll(() => paneText(page, pane), { timeout: 15_000 }).toContain("row-160");

    // The lane is pinned, so it docks beside the agents (PRD right-lane B5).
    await showExplorer(page);
    await setLanePinned(page, true);
    const workspace = page.locator("[data-workspace-screen]");
    await expect(workspace).toHaveAttribute("data-lane-mode", "docked");
    await page.locator(`[data-explorer-row$="/notes.txt"]`).dblclick();
    await expect(page.locator("[data-view-area]")).toBeVisible();
    // Both panes narrow and redraw at the new grid.
    for (const pane of fixture.panes) await expect.poll(() => paneText(page, pane), { timeout: 15_000 }).toMatch(DRAWN_AT_GRID);
    await page.locator(`[data-pane-view="${focused}"] [data-terminal-host]`).click();
    await expect(page.locator(`[data-pane-view="${focused}"]`)).toHaveAttribute("data-focused", "true");
    await use({ page, daemon, fixture, focused, workspace, sent, last });
  } finally {
    daemon?.stop();
    fixture.stop();
  }
}

test("Cmd+F in a focused pane opens that pane's find bar, not the document's, and in the View area stays the document's", async ({ page }) => {
  await dockedWithDocument(page, "pane-find-bar", async ({ focused }) => {
    // B4: with a document open beside the agents, Cmd+F in a focused pane
    // opens that pane's find bar, not the document's.
    await page.keyboard.press(chord("find_in_pane"));
    const bar = page.locator("[data-find-bar]");
    await expect(bar).toBeVisible();
    await expect(bar.locator("input")).toBeFocused();
    await expect(page.locator(".cm-search")).toHaveCount(0);

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
  });
});

test("the find bar steps through matches with Enter and Shift+Enter and counts from 1", async ({ page }) => {
  await dockedWithDocument(page, "pane-find-count", async ({ focused, sent, last }) => {
    await page.keyboard.press(chord("find_in_pane"));
    const bar = page.locator("[data-find-bar]");
    await expect(bar).toBeVisible();

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
  });
});

test("closing the last view leaves the Explorer alone in the docked lane", async ({ page }) => {
  await dockedWithDocument(page, "pane-last-view", async ({ workspace }) => {
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
  });
});
