// The device Overview's backlog and the pane head's record button (PRD
// overview-device-tasks B22-B24, B26-B32), on an isolated engine and hided.
import { expect, test } from "@playwright/test";
import { tabsOf, withControl } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { labelAgentPane } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { waitForTerminal } from "./group-start-helper";
import { openOverview } from "./overview-entry";
import { FIRST_GOAL } from "./session-work-fixture";
import { enterWorkspace } from "./wire";

test.describe.configure({ timeout: 180_000 });

test("@platform a folder's memo is added from the toolbar, opens in the detail, and 되돌리기 deletes it", async ({ page }) => {
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(engine, "overview-backlog");
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
    // The core closes and reopens a Local issue from the same panel (with-hide/hide issue 2947): the
    // control flips with the core's answer, and the status chip follows it.
    await item.locator('[data-item-open-toggle="close"]').click();
    await expect(item.locator('[data-item-open-toggle="reopen"]')).toBeVisible();
    await expect(item.locator("[data-item-undo]")).toHaveCount(0);
    await item.locator('[data-item-open-toggle="reopen"]').click();
    await expect(item.locator('[data-item-open-toggle="close"]')).toBeVisible();
    await item.locator("[data-item-undo]").click();
    await expect(row).toHaveCount(0, { timeout: 30_000 });
  } finally { daemon?.stop(); engine.stop(); }
});

test("@platform an agent pane's head opens its session record in the lane and leaves the keyboard in the terminal", async ({ page }) => {
  const engine = await startFixture();
  let daemon: Daemon | undefined;
  try {
    daemon = await startHided(engine, "pane-record");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const pane = engine.panes[0]!;
    // An agent pane whose session is known has the button. The fixture's
    // pane starts on its own session and the label moves it to another, so
    // the presses wait for the button to carry the labelled one; a press
    // before the move lands would open one record and the next the other.
    const labelled = await labelAgentPane(engine, daemon, pane, { task: FIRST_GOAL, at: Date.now(), end: "done" });
    const button = page.locator(`[data-pane-view="${pane}"] [data-pane-record="${labelled}"]`);
    await expect(button).toBeVisible({ timeout: 45_000 });
    const terminal = page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`);
    await terminal.focus();
    // B22: the lane opens on the record as a desk tab and the terminal keeps the keyboard.
    await button.click();
    await expect(page.locator('[data-card] [data-session-record="lane"]')).toBeVisible({ timeout: 30_000 });
    await expect(terminal).toBeFocused();
    // B23: the same button again folds the desk (PRD sidebar-task-lens B15).
    await button.click();
    await expect(page.locator('[data-session-record="lane"]')).toHaveCount(0);
    await expect(terminal).toBeFocused();
    // B24: a shell pane's head stays plain.
    const tabsBefore = await withControl(daemon, (control) => control.until((rest) => tabsOf(rest, engine.workspace), "the checkout's tabs"));
    await page.getByRole("button", { name: /^New tab/ }).first().click();
    await expect(page.locator("[data-agent-tab-bar] [role=tab]")).toHaveCount(2);
    const shellTab = await withControl(daemon, (control) => control.until((rest) => tabsOf(rest, engine.workspace)?.find((id) => !tabsBefore.includes(id)), "the new shell tab"));
    await waitForTerminal(page, shellTab);
    const shell = page.locator("[data-pane-view]:visible");
    await expect(shell).toHaveCount(1, { timeout: 30_000 });
    expect(engine.panes).not.toContain(await shell.getAttribute("data-pane-view"));
    await expect(shell.locator("[data-pane-record]")).toHaveCount(0);
  } finally { daemon?.stop(); engine.stop(); }
});
