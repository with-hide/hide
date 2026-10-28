import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

// A View an agent opens while the operator is elsewhere waits on its row as
// To look at (PRD unseen-agent-views): the row is in Needs You with the look
// mark, the pane's View list marks it, and pressing the row brings the View
// forward, which sees it. An open in the pane the operator is in is seen at once.

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

test("an agent's open the operator has not looked at waits on its row until the row brings it forward", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(herdr.root, "fixture");
    fs.writeFileSync(path.join(fixture, "login.md"), "# Login\n");
    fs.writeFileSync(path.join(fixture, "notes.md"), "# Notes\n");
    const [pane, other] = herdr.panes;
    daemon = await startHided(herdr, "unseen-views");
    const agent = await agentPane(herdr, daemon.stateDir, pane);
    const hide = async (args: string[]) => {
      const answer = await agent(args);
      expect(answer.status, answer.out).toBe(0);
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const row = page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();
    const mark = row.locator("[data-agent-status-mark]");

    // B1: with the keyboard in the other pane, the agent's open puts its row in Needs You as To look at.
    await page.locator(`[data-pane-view="${other}"] .xterm-screen`).click();
    await expect(page.locator(`[data-pane-view="${other}"]`)).toHaveAttribute("data-focused", "true");
    await hide(["file", "open", "login.md"]);
    await expect(mark).toHaveAttribute("data-agent-status-mark", "look");
    await expect(row).toContainText("login.md");
    // B10: the pane's View list marks the View the operator has not looked at.
    await page.locator(`[data-pane-views="${pane}"]`).click();
    await expect(page.locator(`[data-pane-views-list="${pane}"] [data-pane-view-unseen]`)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await screenshot(page, "unseen-view-row");

    // B3: pressing the row brings its pane and the View forward, and the look goes once it is in front.
    await row.locator(`[data-agent-open="${pane}"]`).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/login\.md/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");

    // B5: an open from the pane the operator is in comes to the front and is seen at once.
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await hide(["file", "open", "notes.md"]);
    await expect(front(page)).toHaveAttribute("aria-label", /\/notes\.md/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
