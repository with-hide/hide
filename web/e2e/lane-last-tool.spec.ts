import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord, commandLabel, mod } from "./chords";
import { chooseLane, enterWorkspace, screenshot, showExplorer } from "./wire";

// A lane closed whole keeps its columns for the next ⌘E, a tool pressed into
// it opens alone, a lane whose last column closed comes back on the
// Explorer, ⇧⌘L opens the Memory tool, ⇧⌘R the sidebar's PR list and ⇧⌘G
// Changes (PRD lane-two-columns B14, B15, B24, B25). Each test starts one
// stack and asserts one contract.

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const closed = (page: Page) => expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
const shows = (page: Page, tool: string) => expect(screen(page)).toHaveAttribute("data-lane-shows", tool);

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
}

test("⌘E reopens the tool a whole-lane close kept, and the Explorer for a lane with no column to bring back", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "lane-last-tool");
    await open(page, daemon);
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "explorer");
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);

    // The rail icon hints show their chord after the name and scope.
    await page.locator('[data-lane-tool="changes"]').hover();
    await expect(page.locator('[data-slot="tooltip-content"]')).toContainText(commandLabel("toggle_lane_changes"));
    await screenshot(page, "lane-last-tool-rail-hint");
    await page.locator('[data-lane-tool="project"]').click();
    await shows(page, "project");
    await screenshot(page, "lane-last-tool-project");

    // ⌘E and the path menu close the lane whole, so the next ⌘E brings the Project tool back (B15).
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "project");
    await chooseLane(page);
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "project");
    await screenshot(page, "lane-last-tool-reopened-project");

    // The path menu's Show right lane returns to it too.
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await chooseLane(page);
    await shows(page, "project");

    // The icon, and the column's own ✕, close the tool column; with no column
    // left the lane closes, and the next ⌘E has nothing to bring back, so it
    // opens the Explorer (B14, B15).
    await page.locator('[data-lane-tool="project"]').click();
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "explorer");
    await page.locator('[data-lane-tool-close="explorer"]').click();
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "explorer");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("a tool pressed into a lane ⌘E closed opens alone, and the documents wait on their rail button (B15)", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.md"), "# Notes\n");
    daemon = await startHided(herdr, "lane-tool-alone");
    await open(page, daemon);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await shows(page, "explorer");

    // ⌘E closes both columns; the Memory icon then opens Memory alone.
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await page.locator('[data-lane-tool="memory"]').click();
    await shows(page, "memory");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await screenshot(page, "lane-tool-alone-memory");

    // ⌘E closes and reopens the lane as it was last shown: Memory alone.
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "memory");
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");

    // The document is kept, and its rail button brings it beside Memory.
    await expect(page.locator('[data-lane-tool="documents"] [data-lane-badge]')).toHaveAttribute("data-lane-badge", "1");
    await page.locator('[data-lane-tool="documents"]').click();
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await shows(page, "memory");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("⌘E bringing the documents back hands the keyboard to the shown document", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "fixture", "notes.md"), "# Notes\n");
    daemon = await startHided(herdr, "lane-toggle-keyboard");
    await open(page, daemon);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/notes.md"]').click();
    const editor = page.locator("[data-editor-body] .cm-content");
    await expect(editor).toBeFocused();

    // From a terminal, ⌘E brings the document back and the keyboard with it.
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    const terminal = page.locator("[data-pane-view] .xterm-helper-textarea").first();
    await terminal.focus();
    await expect(terminal).toBeFocused();
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(editor).toBeFocused();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("⇧⌘L opens the Project's Memory tool, ⇧⌘R the sidebar's PR list, ⇧⌘G Changes and closes it showing, ⌥⌘1-4 do nothing, and the last tool survives a restart (B24, B25)", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "lane-tool-chords");
    await open(page, daemon);
    expect(commandLabel("open_project_memory")).toMatch(/L$/);
    await page.keyboard.press(chord("open_project_memory"));
    await shows(page, "memory");
    // ⇧⌘L again leaves the Memory tool shown; it opens, never toggles.
    await page.keyboard.press(chord("open_project_memory"));
    await shows(page, "memory");
    expect(commandLabel("open_sidebar_prs")).toMatch(/R$/);
    await page.keyboard.press(chord("open_sidebar_prs"));
    await expect(page.locator('[data-sidebar-lens-option="prs"]')).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("[data-sidebar-prs]")).toBeVisible();
    await shows(page, "memory");
    expect(commandLabel("toggle_lane_changes")).toMatch(/G$/);
    await page.keyboard.press(chord("toggle_lane_changes"));
    await shows(page, "changes");
    await expect(page.locator('[data-lane-tool="changes"]')).toHaveAttribute("aria-pressed", "true");
    // The loop ended on Changes, so its chord closes the tool column, and
    // with it the lane; ⌘E then opens the Explorer, and ⇧⌘G brings Changes.
    await page.keyboard.press(chord("toggle_lane_changes"));
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "explorer");
    await page.keyboard.press(chord("toggle_lane_changes"));
    await shows(page, "changes");
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    // The old rail-position chords are unbound: had one opened a tool, ⌘E
    // would close the lane rather than bring Changes back.
    for (const digit of [1, 2, 3, 4]) await page.keyboard.press(mod(`Digit${digit}`, { alt: true }));
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "changes");
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);

    daemon = await daemon.restart();
    await open(page, daemon);
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "changes");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
