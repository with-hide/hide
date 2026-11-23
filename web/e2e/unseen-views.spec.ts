import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { dumpOnFailure } from "./failure-dump";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { quietFor } from "./wait";
import { enterWorkspace, screenshot } from "./wire";

// A View an agent opens while the operator is elsewhere waits on its row as
// To look at (PRD unseen-agent-views): the row is in Needs You with the look
// mark and the View as a chip under it, and the pane's View list marks it.
// Pressing the row goes to the agent only and leaves the View to look at;
// pressing the chip opens the View in focus mode beside the agent's pane,
// with the keyboard on it, which sees it (operator decision 2026-11-23). The
// pane header's eye brings the View forward too. An open in the pane the
// operator is in is seen at once. Only this flow crosses the page, hided and
// the core: the chip's event and the core's focus mode are unit and core
// tests, while the keyboard landing on the View is the page's answer to the
// core's.

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

test("an agent's open the operator has not looked at waits on its row until its chip opens it in focus mode", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(engine.root, "fixture");
    fs.writeFileSync(path.join(fixture, "login.md"), "# Login\n");
    fs.writeFileSync(path.join(fixture, "notes.md"), "# Notes\n");
    fs.writeFileSync(path.join(fixture, "plan.md"), "# Plan\n");
    daemon = await startHided(engine, "unseen-views");
    const [pane, other] = engine.panes;
    const agent = await agentPane(engine, daemon.stateDir, pane);
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
    const chip = row.locator("[data-look-chip]");
    await expect(chip).toContainText("login.md");
    // B10: the pane's View list marks the View the operator has not looked at, and so does its `N views` chip.
    await expect(page.locator(`[data-pane-views="${pane}"]`)).toHaveAttribute("data-pane-views-unseen", "true");
    await page.locator(`[data-pane-views="${pane}"]`).click();
    await expect(page.locator(`[data-pane-views-list="${pane}"] [data-pane-view-unseen]`)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await screenshot(page, "unseen-view-row");

    // The row goes to the agent only: the View stays to look at, on no desk.
    await row.locator(`[data-agent-open="${pane}"]`).click();
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await quietFor(page, 1_000, "the row's press opens no View");
    await expect(page.locator('[data-view-tab-bar] [role="tab"][aria-label*="login.md"]')).toHaveCount(0);
    await expect(mark).toHaveAttribute("data-agent-status-mark", "look");
    await screenshot(page, "unseen-view-row-press");

    // The chip opens the View in focus mode: the agent's pane on the left, the View in front of its desk with the
    // keyboard, and the look goes once it is in front.
    await chip.click();
    await expect(page.locator(`[data-pair-pane="${pane}"]`)).toBeVisible();
    await expect(front(page)).toHaveAttribute("aria-label", /\/login\.md/);
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest("[data-view-area-id]") !== null), { message: "the keyboard on the View" }).toBe(true);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");
    await expect(chip).toHaveCount(0);
    await expect(page.locator(`[data-pane-views="${pane}"]`)).not.toHaveAttribute("data-pane-views-unseen", "true");
    await screenshot(page, "unseen-view-focus-mode");

    // B5: an open from the pane the operator is in, here the left of focus mode, comes to the front and is seen at once.
    await expect(page.locator(`[data-pane-view="${pane}"]`)).toHaveAttribute("data-focused", "true");
    await hide(["file", "open", "notes.md"]);
    await expect(front(page)).toHaveAttribute("aria-label", /\/notes\.md/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");

    // ⌘. leaves focus mode, the screen as it was before it: the lane folded, as the row's press left it.
    await page.keyboard.press(chord("pair_view"));
    await expect(page.locator("[data-pair-pane]")).toHaveCount(0);
    const documents = page.locator('[data-lane-tool="documents"]');
    await expect(documents).toHaveAttribute("aria-pressed", "false");

    // The pane header's eye brings the View to look at forward, without focus mode.
    await page.locator(`[data-pane-view="${other}"] .xterm-screen`).click();
    await expect(page.locator(`[data-pane-view="${other}"]`)).toHaveAttribute("data-focused", "true");
    await hide(["file", "open", "plan.md"]);
    await dumpOnFailure("unseen plan", async () => ({
      focused: await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view"),
      front: await front(page).getAttribute("aria-label"),
      row: await row.textContent(),
      snapshot: await withControl(daemon!, (control) => control.until((rest) => rest, "the failure snapshot")),
    }), () => expect(mark).toHaveAttribute("data-agent-status-mark", "look"));
    await page.locator(`[data-pane-view="${pane}"] button[data-pane-status-mark="look"]`).click();
    await expect(front(page)).toHaveAttribute("aria-label", /\/plan\.md/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
