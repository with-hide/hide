import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, showExplorer } from "./wire";
import { chord } from "./chords";

test("View close follows MRU, preserves background input and hands collapsed areas their keyboard", async ({ page }) => {
  test.setTimeout(90_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    for (const name of ["a.txt", "b.txt", "c.txt"]) fs.writeFileSync(path.join(herdr.root, "fixture", name), `${name}\n`);
    daemon = await startHided(herdr, "view-close-focus");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    await page.locator("[data-lane-pin]").click();
    for (const name of ["a.txt", "b.txt", "c.txt"]) await page.locator(`[data-explorer-row$="/${name}"]`).dblclick();
    const tabs = page.locator('[data-view-tab-bar] [role="tab"]');
    const tab = (name: string) => tabs.filter({ hasText: name });
    const editor = page.locator(".cm-content");
    await tab("b.txt").click();
    await tab("a.txt").click();
    await editor.click();
    await page.keyboard.press(chord("close_tab"));
    await expect(tab("b.txt")).toHaveAttribute("aria-selected", "true");
    await expect(editor).toBeFocused();
    // A background X must not win the window's capture-phase pointer handler.
    await tab("c.txt").hover();
    await tab("c.txt").getByRole("button", { name: "Close view c.txt" }).click();
    await expect(tabs).toHaveCount(1);
    await expect(editor).toBeFocused();
    await page.keyboard.type("kept-input");
    await expect(editor).toContainText("kept-input");

    // Split the only other document into a second area, then close it there.
    await page.locator("[data-lane-expand]").click();
    await page.locator('[data-explorer-row$="/c.txt"]').dblclick();
    await tab("c.txt").click({ button: "right" });
    await page.getByRole("menuitem", { name: "Split", exact: true }).hover();
    await page.getByRole("menuitem", { name: "Right", exact: true }).click();
    await expect(page.locator("[data-view-area]")).toHaveCount(2);
    const right = page.locator("[data-view-area]").last();
    await right.locator(".cm-content").click();
    await page.keyboard.press(chord("close_tab"));
    await expect(page.locator("[data-view-area]")).toHaveCount(1);
    await expect(tab("b.txt")).toHaveAttribute("aria-selected", "true");
    await expect(editor).toBeFocused();
    await expect(editor).toContainText("kept-input");
    // With the Explorer off the final close returns to the live Agent pane.
    await page.locator("[data-lane-tree-toggle]").click();
    await editor.click();
    await page.keyboard.press(chord("close_tab"));
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator("[data-terminal-host] textarea:focus")).toHaveCount(1);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
