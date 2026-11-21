// A turn that ended blocked or unfinished draws its mark on a real daemon
// (PRD agent-blocked-state B1-B4, status-marks B5-B8): the turn badge in Needs
// You, dimmed once read, and a plain dot to read for work left, each with its
// word leading the sentence.
import { expect, test, type Page } from "@playwright/test";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { continueFixtureTranscript, type FixtureLabel } from "./pane-fixture";
import { agentIn, focusTab, sendKeys, withControl } from "./daemon-actions";
import { elsewhereTab, labelAgentPane, setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";

test.describe.configure({ timeout: 150_000 });

const sidebarRow = (page: Page, pane: string) => page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();

/**
 * A finished turn the operator has not seen, labelled `label`: the agent in
 * `pane` takes the label's session, works while the turn's request is
 * appended to its transcript, then rests while `elsewhere` is the tab
 * shown, and its row reads unread. The core reads a transcript's new turn on
 * the agent's next state change, so the label's end lands on this turn; a
 * session taken during the turn would instead rest the agent as seen. A turn
 * finished unseen is published as stopped, which `setAgentState`'s idle does
 * not read, so the rest is typed here and waited for on the unread row.
 */
async function finishTurnUnseen(fixture: AgentFixture, daemon: Daemon, pane: string, elsewhere: string, label: FixtureLabel) {
  await withControl(daemon, (control) => focusTab(control, fixture, elsewhere));
  const session = await labelAgentPane(fixture, daemon, pane, label);
  await setAgentState(daemon, pane, "working");
  continueFixtureTranscript(fixture, session, label);
  await withControl(daemon, async (control) => {
    sendKeys(control, pane, "\x1b]0;✳ Claude Code\x07");
    await control.until((rest) => (agentIn(rest, pane)?.unread === true && agentIn(rest, pane)?.activity !== "working") || undefined, `${pane}'s finished turn unread`, 20_000);
  });
}

/** A block and an unfinished turn, each finished unseen in one tab, as the fixture's two agents. */
async function finishTwoTurns(fixture: AgentFixture, daemon: Daemon, [blocked, unfinished]: [string, string]) {
  await finishTurnUnseen(fixture, daemon, blocked, await elsewhereTab(fixture, daemon), { task: "디스크 정리 작업", progress: "디스크가 가득 차 멈췄어요", end: "blocked" });
  await finishTurnUnseen(fixture, daemon, unfinished, await elsewhereTab(fixture, daemon), { task: "설정 화면 스위치 추가", progress: "테스트 3개 남음", end: "unfinished" });
}

/**
 * The engine reports its changes to the core in the order it made them, so
 * once `pane` shows working here, every state the engine held before that
 * change (the seen it gave a tab) has reached the core and been projected.
 */
async function afterNextChange(daemon: Daemon, page: Page, pane: string) {
  await setAgentState(daemon, pane, "working");
  await expect(sidebarRow(page, pane).locator('[data-agent-status="working"]')).toBeVisible({ timeout: 30_000 });
}

/** The row is read: its mark dimmed and it is not among the Needs You rows. */
async function expectRead(page: Page, pane: string, mark: string) {
  await expect(sidebarRow(page, pane).locator(`[data-agent-status="${mark}"]`)).toHaveClass(/opacity-\(--opacity-read-status\)/);
  await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${pane}"]`)).toHaveCount(0);
}

test("a blocked turn draws the turn mark in Needs You that dims once read, and an unfinished turn a plain dot to read outside it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "blocked-state");
    const [blocked, unfinished] = fixture.panes;
    await finishTwoTurns(fixture, daemon, fixture.panes);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const row = (pane: string) => sidebarRow(page, pane);
    await expect(row(blocked).locator('[data-agent-status="error"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${blocked}"]`)).toBeVisible();
    // Work left is a plain dot to read with its own word, never the green finish (status-marks D-03, D-04).
    await expect(row(unfinished).locator('[data-agent-status="unfinished"]')).toHaveAttribute("data-agent-status-mark", "unread");
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${unfinished}"]`)).toHaveCount(0);
    await expect(row(unfinished)).toContainText("Unfinished");
    await row(blocked).locator("[data-agent-open]").first().click();
    await expect(page.locator(`[data-pane-view="${blocked}"]`)).toHaveAttribute("data-focused", "true");
    // Read (B2): the block leaves Needs You for Seen, its turn mark dims and its cause stays.
    const mark = row(blocked).locator('[data-agent-status="error"]');
    await expect(mark).toHaveClass(/opacity-\(--opacity-read-status\)/);
    await expect(page.locator(`[data-raised-group="needs_you"] [data-pane="${blocked}"]`)).toHaveCount(0);
    await expect(row(blocked)).toContainText("디스크가 가득 차 멈췄어요");
    // Still read after the engine's next change reaches the core (#901): the click's
    // assertions above hold on the frame the click drew, and a revert would
    // show on the projection that change brings.
    await afterNextChange(daemon, page, unfinished);
    await expectRead(page, blocked, "error");
    await expect(row(blocked)).toContainText("디스크가 가득 차 멈췄어요");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
