import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { focusPane, focusTab, newTab, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showExplorer } from "./wire";

// Agent opens and the desks of Agent tabs (PRD lane-desk-pair-view B1, B9):
// only the agent in the pane the operator watches puts a View on the desk in
// front; an agent beside it leaves the desk, its front tab, the Agent tab and
// the lane as they were, its View waiting in its pane's list; and each Agent
// tab shows its own desk.

test.describe.configure({ timeout: 180_000 }); // wait-ok: two agents start and report their sessions before the first step

const agentTab = (page: Page, id: string) => page.locator(`[data-agent-tab-bar] [data-tab="${id}"]`);
const viewTabs = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"]');
const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');
const screen = (page: Page) => page.locator("[data-workspace-screen]");

test("a background agent's open leaves the desk in front as it was, and each Agent tab shows its own desk", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    for (const name of ["a.md", "b.md"]) fs.writeFileSync(path.join(fixture, name), `${name}\n`);
    daemon = await startHided(engine, "view-bookmarks");
    const [watched, beside] = engine.panes;
    // Both panes of the first tab run an agent; the operator stays in the first.
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
    await showExplorer(page);

    // The watched agent's open comes up on its tab's desk beside the Explorer.
    await open(watchedAgent, "a.md");
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(viewTabs(page)).toHaveCount(1);
    const mode = (await screen(page).getAttribute("data-lane-mode"))!;

    // B1: the agent beside it opens b.md. It waits in that pane's list; the
    // desk, its front tab, the Agent tab in front and the lane stay as they were.
    await open(besideAgent, "b.md");
    await expect(page.locator(`[data-pane-views="${beside}"]`)).toHaveText(/1 view/);
    await expect(viewTabs(page)).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await expect(agentTab(page, engine.tab)).toHaveAttribute("aria-selected", "true");
    await expect(screen(page)).toHaveAttribute("data-lane-mode", mode);
    await screenshot(page, "view-bookmarks-background-open");

    // B9: the second Agent tab shows its own desk, which holds nothing yet,
    // and the first tab's desk comes back as it was.
    await agentTab(page, second.tab).click();
    await expect(agentTab(page, second.tab)).toHaveAttribute("aria-selected", "true");
    await expect(viewTabs(page)).toHaveCount(0);
    await agentTab(page, engine.tab).click();
    await expect(agentTab(page, engine.tab)).toHaveAttribute("aria-selected", "true");
    await expect(viewTabs(page)).toHaveCount(1);
    await expect(front(page)).toHaveAttribute("aria-label", /\/a\.md/);
    await screenshot(page, "view-bookmarks-tab-1-back");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
