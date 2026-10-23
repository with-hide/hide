import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showExplorer } from "./wire";
import { chord } from "./chords";

test("close follows the keyboard and the lane never renders an empty surface", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1920, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.md"), "# Notes\n\n한글과 English\n");
    daemon = await startHided(herdr, "close-policy");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page);
    await showExplorer(page);
    const panes = page.locator("[data-pane-view]");
    const screen = page.locator("[data-workspace-screen]");
    const tree = page.locator('[data-lane="explorer"]');
    const file = page.locator('[data-explorer-row$="/notes.md"]');
    await file.click();
    const editor = page.locator("[data-editor-body] .cm-content");
    await expect(editor).toBeVisible();
    await editor.click();
    await screenshot(page, "close-view-before");
    // Browser column: Alt W; Electron uses the same command with Command W.
    // Closing the last view closes the viewing column and leaves the Explorer in its column (PRD lane-two-columns B12).
    await page.keyboard.press(chord("close_tab"));
    await expect(editor).toHaveCount(0);
    await expect(panes).toHaveCount(2);
    await expect(tree).toBeVisible();
    await expect(screen).toHaveAttribute("data-lane-view", "none");
    await expect(page.locator("[data-explorer-tree]")).toBeFocused();
    await expect(screen).toHaveAttribute("data-lane-mode", "overlay");
    await expect(page.locator("[data-lane-frame] [data-view-area]")).toHaveCount(0);
    await screenshot(page, "close-view-explorer-alone");

    // The tool column owns the keyboard: no View or terminal close is allowed (B16).
    await file.focus();
    await page.keyboard.press(chord("close_tab"));
    await expect(panes).toHaveCount(2);
    await expect(tree).toBeVisible();
    await file.click();
    await expect(editor).toBeVisible();
    await page.keyboard.press(chord("toggle_lane_explorer"));
    await expect(tree).toHaveCount(0);
    // ⌘E closes the lane and brings it back with its view (B2, B20; lane-two-columns B15).
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(editor).toBeVisible();
    await expect(tree).toHaveCount(0);

    // An unfocused rail control has no closable owner.
    await page.locator('[data-lane-tool="documents"]').focus();
    await page.keyboard.press(chord("close_tab"));
    await expect(panes).toHaveCount(2);
    await expect(editor).toBeVisible();
    // Escape closes the overlay; a terminal then owns close.
    await editor.click();
    await page.keyboard.press("Escape");
    await expect(screen).toHaveAttribute("data-lane-mode", "closed");
    const [first, second] = herdr.panes;
    await page.locator(`[data-terminal-host="${second}"]`).click();
    await page.keyboard.press(chord("close_tab"));
    const confirm = page.getByRole("button", { name: "Stop work and close" });
    if (await confirm.isVisible()) await confirm.click();
    await expect(page.locator(`[data-pane-view="${second}"]`)).toHaveCount(0, { timeout: 15_000 });
    await expect(page.locator(`[data-pane-view="${first}"]`)).toBeVisible();
    await expect(panes).toHaveCount(1);
    await screenshot(page, "close-pane-survivor");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
