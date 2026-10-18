// The device Overview's backlog and the pane head's record button (PRD
// overview-device-tasks B22-B24, B26-B32), on an isolated Herdr and hided.
import { expect, test } from "@playwright/test";
import { labelAgent, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { openOverview } from "./overview-entry";
import { FIRST_GOAL } from "./session-work-fixture";
import { enterWorkspace, showTool } from "./wire";

test.describe.configure({ timeout: 180_000 });

test("@platform a folder's memo is added from the toolbar, opens in the detail, and 되돌리기 deletes it", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "overview-backlog");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await openOverview(page, "backlog");
    await page.locator("[data-backlog-add]").click();
    const quick = page.locator("[data-quick-add]");
    const input = quick.locator("[data-quick-add-input]");
    await expect(input).toBeVisible({ timeout: 30_000 });
    await expect(page.locator("[data-backlog-destination]").first()).toHaveAttribute("data-backlog-destination", "local");
    await input.fill("폴더 메모");
    await input.press("Enter");
    await expect(quick.locator("[data-quick-add-made]")).toBeVisible();
    await expect(quick).toHaveCount(0, { timeout: 10_000 });
    // The local default creates one row without a GitHub write.
    const row = page.locator("[data-backlog-row]", { hasText: "폴더 메모" });
    await expect(row).toBeVisible({ timeout: 30_000 });
    await expect(row).toContainText("L-1");
    await expect(page.locator('[data-main-tab="backlog"] [data-tab-count]')).toHaveText("1");
    await expect(page.locator("[data-backlog-table] thead")).toHaveCount(1);
    // All-project scope omits projects without matching rows and offers
    // one global no-results state. Clearing restores the same memo (B12).
    await page.locator("[data-backlog-query]").fill("no-such-backlog-item");
    await expect(page.locator("[data-backlog-row]")).toHaveCount(0);
    await expect(page.locator("[data-backlog-section]")).toHaveCount(0);
    await expect(page.locator("[data-backlog-empty]")).toHaveCount(1);
    await page.locator("[data-backlog-empty]").getByRole("button", { name: "Clear filters", exact: true }).click();
    await expect(row).toBeVisible();
    // B31, B32: the row opens its panel, whose 되돌리기 deletes the memo.
    await row.getByRole("button").first().click();
    const item = page.locator("[data-overview-panel] [data-backlog-item]");
    await expect(item).toBeVisible();
    await item.locator("[data-item-undo]").click();
    await expect(row).toHaveCount(0, { timeout: 30_000 });
  } finally { daemon?.stop(); herdr.stop(); }
});

test("a record shown in the lane's 보기 slot stays through pin, unpin and a window too narrow to dock the lane", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "record-remount");
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = herdr.panes[0]!;
    labelAgent(herdr, pane, { task: FIRST_GOAL, at: Date.now(), end: "done" });
    // The lane has a tool to pin, then the record takes its slot (PRD sidebar-task-lens B15).
    await showTool(page, "files");
    await page.locator(`[data-pane-view="${pane}"] [data-pane-record]`).click({ timeout: 45_000 });
    const record = page.locator('aside[data-lane="detail"] [data-session-record="lane"]');
    await expect(record).toBeVisible({ timeout: 30_000 });
    // Each change redraws the lane in another frame; the record it shows is not left by that.
    const mode = page.locator("[data-workspace-screen]");
    for (const state of ["docked", "overlay"]) {
      await page.locator("[data-lane-more]").click();
      await page.locator('[data-lane-menu-item="pin"]').click();
      await expect(mode).toHaveAttribute("data-lane-mode", state);
      await expect(record).toBeVisible();
    }
    const size = page.viewportSize()!;
    await page.setViewportSize({ width: 640, height: size.height });
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "overlay");
    await expect(record).toBeVisible();
    await page.setViewportSize(size);
    await expect(record).toBeVisible();
  } finally { daemon?.stop(); herdr.stop(); }
});

test("@platform an agent pane's head opens its session record in the lane and leaves the keyboard in the terminal", async ({ page }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(herdr, "pane-record");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = herdr.panes[0]!;
    // An agent pane whose session is known has the button. The fixture's
    // pane starts on its own session and the label moves it to another, so
    // the presses wait for the button to carry the labelled one; a press
    // before the move lands would open one record and the next the other.
    const labelled = labelAgent(herdr, pane, { task: FIRST_GOAL, at: Date.now(), end: "done" });
    const button = page.locator(`[data-pane-view="${pane}"] [data-pane-record="${labelled}"]`);
    await expect(button).toBeVisible({ timeout: 45_000 });
    const terminal = page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`);
    await terminal.focus();
    // B22: the lane opens on the record in its 보기 slot and the terminal keeps the keyboard.
    await button.click();
    await expect(page.locator('aside[data-lane="detail"] [data-session-record="lane"]')).toBeVisible({ timeout: 30_000 });
    await expect(terminal).toBeFocused();
    // B23: the same button again puts the slot away (PRD sidebar-task-lens B15).
    await button.click();
    await expect(page.locator('[data-session-record="lane"]')).toHaveCount(0);
    await expect(terminal).toBeFocused();
    // B24: a shell pane's head stays plain.
    await page.getByRole("button", { name: /^New tab/ }).first().click();
    const shell = page.locator("[data-pane-view]:visible");
    await expect(shell).toHaveCount(1, { timeout: 30_000 });
    expect(herdr.panes).not.toContain(await shell.getAttribute("data-pane-view"));
    await expect(shell.locator("[data-pane-record]")).toHaveCount(0);
  } finally { daemon?.stop(); herdr.stop(); }
});
