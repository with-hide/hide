import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { chord } from "./chords";
import { focusPane, focusTab, newTab, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showDocuments, showTool } from "./wire";

// The desk of an Agent tab on the screen (PRD lane-desk-pair-view B2, B3,
// B8, B9, B11, B13, B14, B30): the watched agent's open waits on a closed
// lane, counted on the rail; a pane's Views list puts what it holds on the
// desk; moving between the tab's panes moves nothing on its desk; another tab
// shows its own; a tab's ✕ takes a View off the desk only; ⌘E closes and
// brings back the desk; and Escape in a document's editor is the editor's,
// so Escape then Tab leaves it and the next Escape closes the lane.

test.describe.configure({ timeout: 180_000 }); // wait-ok: two agents start and report their sessions before the first step

const agentTab = (page: Page, id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
const deskTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const screen = (page: Page) => page.locator("[data-workspace-screen]");
const deskButton = (page: Page) => page.locator('[data-lane-tool="documents"]');
const terminal = (page: Page, pane: string) => page.locator(`[data-pane-view="${pane}"] [data-terminal-host]`);

test("an Agent tab's desk holds what the operator put on it, whichever pane they move to, and its rail button counts it", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    for (const name of ["a.md", "b.md", "c.md"]) fs.writeFileSync(path.join(fixture, name), `${name}\n`);
    daemon = await startHided(engine, "lane-desk");
    const [watched, beside] = engine.panes;
    const besideAgent = await agentPane(engine, daemon.stateDir, beside);
    const watchedAgent = await agentPane(engine, daemon.stateDir, watched);
    const second = await withControl(daemon, async (control) => {
      const made = await newTab(control, engine, "second");
      await focusTab(control, engine, engine.tab);
      await focusPane(control, watched);
      return made;
    });
    const open = async (agent: typeof watchedAgent, name: string) => {
      const answer = await agent(["file", "open", name]);
      expect(answer.status, answer.out).toBe(0);
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    // An empty desk's button is off and says so.
    await expect(deskButton(page)).toHaveAttribute("aria-disabled", "true");

    // B2: the watched agent's open waits on the closed lane; the rail counts it, naming this tab.
    await open(watchedAgent, "a.md");
    await expect(page.locator('[data-lane-tool="documents"]')).toHaveAttribute("aria-description", "1 view open");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(deskButton(page)).toHaveAttribute("aria-disabled", "false");
    await deskButton(page).hover();
    await expect(page.getByRole("tooltip")).toContainText(/Documents · .+ · 1 view open/);
    await showDocuments(page);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    // B13: the tab is its mark and name, no agent's name after it; the path line's logo names the opener.
    await expect(front(page)).toHaveText("a.md");
    await expect(page.locator(`[data-view-area] [data-view-opener="${watched}"]`)).toBeVisible();
    await screenshot(page, "lane-desk-opened");

    // B3: the agent beside it opens b.md, which waits in its pane's list; a row of that list puts it on this desk in front.
    await open(besideAgent, "b.md");
    const chip = page.locator(`[data-pane-views="${beside}"]`);
    await expect(chip).toHaveText(/1 view/);
    await expect(deskTabs(page)).toHaveCount(1);
    await chip.click();
    await page.getByRole("menuitem", { name: /b\.md/ }).click();
    await expect(deskTabs(page)).toHaveCount(2);
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(chip).toHaveText(/1 view/);
    await expect(page.locator('[data-lane-tool="documents"]')).toHaveAttribute("aria-description", "2 views open");
    await screenshot(page, "lane-desk-two-tabs");

    // B8: moving between the tab's panes leaves its desk, its front tab and the lane as they were.
    const mode = (await screen(page).getAttribute("data-lane-mode"))!;
    // The open lane covers the right of the pane beside, so the press lands on its left edge.
    await terminal(page, beside).click({ position: { x: 20, y: 200 } });
    await terminal(page, watched).click();
    await expect(deskTabs(page)).toHaveCount(2);
    await expect(front(page)).toHaveAttribute("aria-label", /\/b\.md/);
    await expect(screen(page)).toHaveAttribute("data-lane-mode", mode);

    // B11: a desk tab's ✕ takes an agent's View off the desk; its pane's list still holds it.
    await front(page).locator("button").click();
    await expect(deskTabs(page)).toHaveCount(1);
    await expect(chip).toHaveText(/1 view/);
    await expect(page.locator('[data-lane-tool="documents"]')).toHaveAttribute("aria-description", "1 view open");

    // B9: the second Agent tab shows its own desk, empty, its button off; the first tab's comes back as it was.
    await agentTab(page, second.tab).click();
    await expect(agentTab(page, second.tab)).toHaveAttribute("aria-selected", "true");
    await expect(deskTabs(page)).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-lane-view", "none");
    await expect(deskButton(page)).toHaveAttribute("aria-disabled", "true");
    await screenshot(page, "lane-desk-second-tab");
    await agentTab(page, engine.tab).click();
    await expect(deskTabs(page)).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(page.locator('[data-lane-tool="documents"]')).toHaveAttribute("aria-description", "1 view open");

    // A file the operator opens from Explorer joins the same desk, in front.
    await showTool(page, "explorer");
    await page.locator('[data-explorer-row$="/c.md"]').click();
    await expect(deskTabs(page)).toHaveCount(2);
    await expect(front(page)).toHaveAttribute("aria-label", /\/c\.md/);
    await expect(page.locator('[data-lane-tool="documents"]')).toHaveAttribute("aria-description", "2 views open");
    await screenshot(page, "lane-desk-explorer");

    // B30: ⌘E closes the lane and brings the desk back.
    await terminal(page, watched).click();
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-view", "documents");
    await expect(front(page)).toHaveAttribute("aria-label", /\/c\.md/);

    // Escape in the editor is the editor's: Tab then leaves it, and Escape from there closes the lane.
    const editor = page.locator("[data-view-area] .cm-content").first();
    await editor.click();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "docked");
    await page.keyboard.press("Tab");
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest(".cm-editor") === null)).toBe(true);
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await expect(page.locator(`[data-pane-view="${watched}"] .xterm-helper-textarea`)).toBeFocused();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("a crowded desk reads its front tab whole and shrinks the others, and the rail's desk button leaves the keyboard on that tab, whose Escape closes the lane", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const names = ["release-checklist-for-the-desk.md", "pair-view-notes-and-questions.md", "card-tabs-on-a-desk-draft.md", "rail-and-path.md"];
    for (const name of names) fs.writeFileSync(path.join(engine.root, "fixture", name), `# ${name}\n`);
    daemon = await startHided(engine, "lane-desk-crowded");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showTool(page, "explorer");
    // A double click keeps each open, so the four stay as four tabs.
    for (const [index, name] of names.entries()) {
      await page.locator(`[data-explorer-row$="/${name}"]`).dblclick();
      await expect(deskTabs(page)).toHaveCount(index + 1);
    }
    // D-30: the front tab's name is whole while it fits the selected maximum (240px); a tab behind it is cut or down to
    // its marks, its tooltip naming it whole.
    const cut = (tab: ReturnType<typeof front>) => tab.locator("[data-tab-title]").evaluate((title: HTMLElement) => title.offsetParent === null || title.scrollWidth > title.clientWidth);
    await expect(front(page)).toContainText(names[3]!);
    await expect.poll(() => cut(front(page))).toBe(false);
    expect(await cut(deskTabs(page).first())).toBe(true);
    await deskTabs(page).first().hover();
    await expect(page.getByRole("tooltip")).toContainText(names[0]!);
    await screenshot(page, "lane-desk-crowded");

    // B30: ⌘E closes the lane; the rail's desk button brings the desk back with the keyboard on its front tab, and Escape there closes it.
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
    await deskButton(page).click();
    await expect(front(page)).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", "closed");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
