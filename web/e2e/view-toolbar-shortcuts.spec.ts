import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, setLanePinned, showExplorer } from "./wire";
import { chord } from "./chords";

test("View toolbar shortcuts keep New tab, Close, recent tabs and Find in the View", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    for (const name of ["a.txt", "b.txt"]) fs.writeFileSync(path.join(herdr.root, "fixture", name), "sample text\n");
    daemon = await startHided(herdr, "view-toolbar-shortcuts");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    await setLanePinned(page, true);
    for (const name of ["a.txt", "b.txt"]) await page.locator(`[data-explorer-row$="/${name}"]`).dblclick();
    await page.locator(".cm-content").click();
    const views = page.locator('[data-view-tab-bar] [role="tab"]');
    const agents = page.locator('[data-agent-tab-bar] [role="tab"]');
    await expect(agents).toHaveCount(1);
    await page.keyboard.press(chord("new_tab"));
    const address = page.getByRole("textbox", { name: "Page address" });
    await expect(address).toBeFocused();
    await page.keyboard.press(chord("new_tab"));
    await expect(views).toHaveCount(4);
    await expect(agents).toHaveCount(1);
    await expect(address).toBeFocused();
    await expect(page.locator('[data-view-area-id][data-keyboard-area="true"]')).toHaveCount(0);
    await page.keyboard.press(chord("close_tab"));
    await expect(views).toHaveCount(3);
    await address.click();
    await page.keyboard.press(chord("recent_area_tab"));
    await expect(views.filter({ hasText: "b.txt" })).toHaveAttribute("aria-selected", "true");
    await page.locator("[data-document-header]").getByRole("button", { name: "Find in document", exact: true }).click();
    const search = page.locator('.cm-search input[name="search"]');
    await expect(search).toBeFocused();
    await search.fill("sample");
    await page.keyboard.press(chord("find_in_pane"));
    await expect(search).toBeFocused();
    await expect(page.locator("[data-find-bar]")).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
