import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord, commandLabel, mod } from "./chords";
import { chooseLane, enterWorkspace, screenshot, showExplorer } from "./wire";

// A closed lane remembers the tool it last showed, and ⇧⌘L/R/G open its
// project and checkout tools. Each test starts one stack and asserts one contract.

const screen = (page: Page) => page.locator("[data-workspace-screen]");
const closed = (page: Page) => expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
const shows = (page: Page, tool: string) => expect(screen(page)).toHaveAttribute("data-lane-shows", tool);

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
}

test("⌘E reopens the tool the lane last showed, however it was closed, and Files for a lane that never showed one", async ({ page }) => {
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "lane-last-tool");
    await open(page, daemon);
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "files");
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);

    // The rail icon hints show their chord after the name and scope.
    await page.locator('[data-lane-tool="memory"]').hover();
    await expect(page.locator('[data-slot="tooltip-content"]')).toContainText(commandLabel("toggle_lane_memory"));
    await screenshot(page, "lane-last-tool-rail-hint");
    await page.locator('[data-lane-tool="memory"]').click();
    await shows(page, "memory");
    await screenshot(page, "lane-last-tool-memory-hint");

    // Each way of closing the lane keeps Memory for the next ⌘E.
    await page.locator("[data-lane-close]").click();
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "memory");
    await page.locator('[data-lane-tool="memory"]').click();
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "memory");
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await page.keyboard.press(chord("toggle_lane"));
    await shows(page, "memory");
    await screenshot(page, "lane-last-tool-reopened-memory");

    // The path menu's Show right lane returns to it too.
    await page.keyboard.press(chord("toggle_lane"));
    await closed(page);
    await chooseLane(page);
    await shows(page, "memory");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("⌘E bringing Files back with a view hands the keyboard to that view's document", async ({ page }) => {
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
    await shows(page, "files");
    await expect(editor).toBeFocused();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("⇧⌘L, ⇧⌘R and ⇧⌘G open Memory, PRs and Changes and close the tool showing, ⌥⌘1-4 do nothing, and the last tool survives a restart (B24)", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "lane-tool-chords");
    await open(page, daemon);
    for (const [tool, key] of [["memory", "L"], ["prs", "R"], ["changes", "G"]] as const) {
      const id = `toggle_lane_${tool}` as const;
      expect(commandLabel(id), tool).toMatch(new RegExp(`${key}$`));
      await page.keyboard.press(chord(id));
      await shows(page, tool);
      await expect(page.locator(`[data-lane-tool="${tool}"]`)).toHaveAttribute("aria-pressed", "true");
    }
    // The loop ended on Changes, so its chord closes the lane.
    await page.keyboard.press(chord("toggle_lane_changes"));
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
