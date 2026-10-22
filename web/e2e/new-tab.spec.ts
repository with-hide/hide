import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showExplorer } from "./wire";
import { chord } from "./chords";

test("View new tab replaces itself with a file or changed-file diff", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env });
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    git("init", "-q"); git("add", ".");
    git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "Initial files");
    daemon = await startHided(herdr, "new-tab");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    // The first view gives the panel a View strip.
    await page.locator('[data-explorer-row$="/note.txt"]').click();
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(1);
    await page.locator('[data-view-new-tab]').click();
    const body = page.locator('[data-new-tab-page]');
    await expect(body.getByRole("heading", { name: "Open" })).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Page address" })).toBeFocused();
    await expect(page.getByRole("textbox", { name: "Page address" })).toHaveValue("");
    await expect(body.getByRole("button", { name: "Diff", exact: true })).toHaveCount(0);
    const displayId = await page.locator('[data-browser-display]').getAttribute("data-browser-display");
    await screenshot(page, "new-tab-clean");
    await body.getByRole("button", { name: "File" }).click();
    await page.locator('[data-palette-input]').fill("note.txt");
    await page.locator('[data-palette-row$="/note.txt"]').click();
    await expect(body).toHaveCount(0);
    // Coalesce the existing view in this area into the tab the user chose.
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(1);
    await expect(page.locator(`[role="tab"][data-display="${displayId}"]`)).toContainText("note.txt");
    await screenshot(page, "new-tab-file");

    fs.writeFileSync(path.join(root, "note.txt"), "changed\n");
    await page.locator('[data-view-new-tab]').click();
    await expect(body.getByRole("button", { name: "Diff", exact: true })).toBeVisible({ timeout: 15_000 });
    const diffId = await page.locator('[data-browser-display]').getAttribute("data-browser-display");
    await screenshot(page, "new-tab-changed");
    await body.getByRole("button", { name: "Diff", exact: true }).click();
    await expect(page.locator('[data-palette="Open diff"]')).toBeVisible();
    await page.locator('[data-palette-row$="/note.txt"]').click();
    await expect(body).toHaveCount(0);
    await expect(page.locator(`[role="tab"][data-display="${diffId}"]`)).toHaveAttribute("data-tab-kind", "diff");

    await page.locator('[data-view-new-tab]').click();
    const emptyId = await page.locator('[data-browser-display]').getAttribute("data-browser-display");
    await page.locator(`[role="tab"][data-display="${emptyId}"]`).click({ button: "right" });
    await page.getByRole("menuitem", { name: "New tab", exact: true }).click();
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(4);
    const address = page.getByRole("textbox", { name: "Page address" });
    await address.fill("https://example.com"); await address.press("Enter");
    await expect(page.locator('[data-browser-address]')).toHaveText("example.com");
    // Cmd+P remains a file palette outside the new-tab page.
    await page.keyboard.press(chord("open_file"));
    await expect(page.locator('[data-palette="Open file"]')).toBeVisible();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("New tab chord opens where the keyboard is: its View area, its pane's Agent area, else the active area", async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const root = path.join(herdr.root, "fixture");
    fs.writeFileSync(path.join(root, "note.txt"), "original\n");
    const second = herdr.run(["tab", "create", "--workspace", herdr.workspace, "--cwd", root, "--no-focus"]) as { result: { tab: { tab_id: string } } };
    const secondId = second.result.tab.tab_id;
    daemon = await startHided(herdr, "new-tab-keyboard");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const agentTab = (id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
    await agentTab(secondId).click({ button: "right" });
    await page.locator('[data-menu-group="split"]').hover();
    await page.locator('[data-menu-item="split_right"]').click();
    const areas = page.locator("[data-agent-area-id]");
    await expect(areas).toHaveCount(2);
    const [leftId, rightId] = await areas.evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-agent-area-id")!));
    const left = page.locator(`[data-agent-area-id="${leftId}"]`);
    const right = page.locator(`[data-agent-area-id="${rightId}"]`);
    const agentTabs = page.locator('[data-agent-tab-bar] [role="tab"]');
    await expect(agentTabs).toHaveCount(2);

    // The keyboard in a panel View area: the View strip's New tab, in that area.
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/note.txt"]').click();
    const editor = page.locator("[data-editor-body] .cm-content");
    await editor.click();
    const viewTabs = page.locator('[data-view-tab-bar] [role="tab"]');
    await expect(viewTabs).toHaveCount(1);
    await page.keyboard.press(chord("new_tab"));
    await expect(viewTabs).toHaveCount(2);
    await expect(page.getByRole("textbox", { name: "Page address" })).toBeFocused();
    await expect(page.getByRole("textbox", { name: "Page address" })).toHaveValue("");
    await expect(agentTabs).toHaveCount(2);
    await screenshot(page, "new-tab-chord-view");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");

    // The keyboard in an Agent pane: a tab at the end of the area showing that
    // pane. Focusing a pane activates its area, and the core's area focus moves
    // the keyboard with it, so this is the active area too, as before.
    await right.locator("[data-terminal-host]").first().click();
    await page.keyboard.press(chord("new_tab"));
    await expect(right.locator('[role="tab"]')).toHaveCount(2);
    await expect(left.locator('[role="tab"]')).toHaveCount(1);
    await left.locator("[data-terminal-host]").first().click();
    await page.keyboard.press(chord("new_tab"));
    await expect(left.locator('[role="tab"]')).toHaveCount(2);
    await expect(right.locator('[role="tab"]')).toHaveCount(2);
    await screenshot(page, "new-tab-chord-pane");

    // Nowhere in particular: the Agent column's active area, as before.
    await agentTab(secondId).click();
    await expect(right).toHaveAttribute("data-active-area", "true");
    await page.locator('[data-lane-tool="memory"]').focus();
    await page.keyboard.press(chord("new_tab"));
    await expect(right.locator('[role="tab"]')).toHaveCount(3);
    await expect(left.locator('[role="tab"]')).toHaveCount(2);
    await expect(viewTabs).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
