// One background candidate, with production record workers and private native
// conversations. GitHub and drafting stay controlled in this visual fixture.
import { expect, type ElectronApplication, type Locator } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { setAgentState } from "../../web/e2e/fixture-agents";
import { continueFixtureTranscript } from "../../web/e2e/pane-fixture";
import { FIRST_GOAL, NEXT_GOAL, nextWork, seedFirstWork, workFixture, workRepositoryReady } from "../../web/e2e/session-work-fixture";
import { chord } from "../../web/e2e/chords";
import { enterWorkspace, hoverSettledTaskRow, openTaskFolds, showExplorer, taskRow } from "../../web/e2e/wire";
import { fitWindow, launchShell, REPO, startStack, test } from "./fixture";
import { captureElectronWindow } from "./electron-window";
import { finishNative } from "./native-teardown";

/** This session's PRs and the child's, each in its own group; the settled ones open from their fold (PRD session-record-goal-flow B18, B19). */
async function expectRecordPrs(record: Locator, child: string, total: number) {
  await expect(record.locator(`[data-record-prs="${total}"]`)).toBeVisible({ timeout: 45_000 });
  const fold = record.locator("[data-record-settled]");
  if (await fold.count()) await fold.click();
  await expect(record.locator(`[data-record-pr-group="${child}"] [data-record-pr-row="34"]`)).toBeVisible();
}

// The repository and the fake gh exist before the daemon registers the checkout and starts with its PATH.
test("@platform session records by goal flow, ended work and search render in both themes in a narrow native lane", async () => {
  test.setTimeout(240_000);
  let github!: ReturnType<typeof workFixture>;
  const stack = await startStack("session-record", { family: true, prepare: (fixture, home) => { github = workFixture(fixture); Object.assign(home.env, github.env); } });
  const { fixture, run, daemon } = stack;
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 1440, height: 1000 });
    await workRepositoryReady(run.env.HIDE_STATE_DIR!);
    await enterWorkspace(page, "fixture");
    // The lane up on the Explorer, so the record opens beside it as a desk tab.
    await showExplorer(page);
    const first = Date.now();
    github.clock(first, first);
    const { session, child: delegated, pane } = await seedFirstWork(fixture, daemon, first);
    const child = delegated!;
    const root = await taskRow(page, pane, FIRST_GOAL);
    const tasks = page.locator("[data-task-lens]");
    // The core settles the root among the resting ones once its child has ended; a hover before that is lost when the row moves.
    await hoverSettledTaskRow(root, "resting");
    await page.locator(`[data-task-record="${pane}"]`).click();
    // The record opens as a tab on the desk (PRD sidebar-task-lens B15; lane-desk-pair-view D-13).
    const record = page.locator('[data-card="record"] [data-session-record="lane"]');
    await expect(record.locator("[data-record-flow]")).toHaveCount(1, { timeout: 45_000 });
    // The record reads the session's one list (status-model, One list per session): #32, which the pane holds on
    // the checkout's branch, joins this session's and its child's from the start.
    await expectRecordPrs(record, child, 3);
    // The rail's desk button is pressed while the desk shows the card, folds it when pressed, and brings it back (PRD lane-desk-pair-view B14).
    const deskButton = page.locator('[data-lane-tool="documents"]');
    await expect(deskButton).toHaveAttribute("aria-pressed", "true");
    await deskButton.click();
    // A folded desk stays laid out out of sight, as its documents do, so the card is kept but not drawn.
    await expect(record).toBeHidden();
    await expect(deskButton).toHaveAttribute("aria-pressed", "false");
    await deskButton.click();
    await expect(record).toBeVisible();
    await expect(deskButton).toHaveAttribute("aria-pressed", "true");
    await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
    await page.keyboard.type(NEXT_GOAL);
    await page.keyboard.press("Enter");
    const second = Date.now();
    github.clock(first, second);
    continueFixtureTranscript(fixture, session, nextWork(second));
    await setAgentState(daemon, pane, "working");
    await expect(root).toContainText(NEXT_GOAL, { timeout: 45_000 });
    await setAgentState(daemon, pane, "idle");
    // The new goal opens a second flow, the agent's question waits on top, and the child's PR stays in its group (B3, B11, B18).
    await expect(record.locator("[data-record-flow]")).toHaveCount(2, { timeout: 45_000 });
    await expect(record.locator("[data-record-ask]")).toHaveCount(1, { timeout: 45_000 });
    await expectRecordPrs(record, child, 3);
    // The child is a row of the parent's record (B20); the capture keeps the parent in front.
    await expect(record.locator(`[data-record-child="${child}"]`)).toBeVisible();
    const fixtureIdentity = { head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: REPO, encoding: "utf8" }).trim(), home: run.env.HOME, state: run.env.HIDE_STATE_DIR, daemon: run.daemonPid(), parent: pane, child, provider: "scripted", github: "scripted" };
    for (const theme of ["light", "dark"] as const) {
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.locator('[data-interface-language]').click();
      await page.locator('[data-language-option="ko"]').click();
      await page.keyboard.press("Escape");
      const facts = { ...fixtureIdentity, theme, tools: await page.locator("[data-lane-frame]").boundingBox() };
      await captureElectronWindow(app, `record-flows-children-${theme}`, facts);
      // A past flow opens to its requests on a click (B13).
      await record.locator("[data-record-flow-toggle]").first().click();
      await expect(record.locator("[data-record-flow]").first().locator("[data-record-request]").first()).toBeVisible();
      await captureElectronWindow(app, `record-flow-opened-${theme}`, facts);
      await record.locator("[data-record-flow-toggle]").first().click();
      // The pane head carries no task chip (B1).
      await expect(page.locator("[data-pane-task-chip]")).toHaveCount(0);
      await page.keyboard.press(chord("search", "electron"));
      await page.locator('[data-palette-input]').fill(NEXT_GOAL);
      await expect(page.locator('[data-palette-group="sessions"]')).toContainText(NEXT_GOAL, { timeout: 30_000 });
      await captureElectronWindow(app, `record-search-${theme}`, facts);
      await page.keyboard.press("Escape");
      const divider = page.locator('[data-lane-divider="lane"]');
      for (let step = 0; step < 6; step += 1) await divider.press("ArrowRight");
      await captureElectronWindow(app, `record-narrow-${theme}`, { ...facts, tools: await page.locator("[data-lane-frame]").boundingBox() });
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-settings-tab="hideAi"]').click();
      await page.locator('[data-ai-agent-summary="true"]').click();
      await page.keyboard.press("Escape");
      // Summaries off: the parent's two requests as sent, no flow (B28); the child's are in its own record, behind its row.
      await expect(record.locator("[data-record-flow]")).toHaveCount(0);
      await expect(record.locator("[data-record-raw-requests] [data-record-request]")).toHaveCount(2);
      await expect(record.getByRole('heading', { level: 2 })).toContainText('Claude');
      await expect(record.locator('[data-record-child]')).toContainText('Claude');
      await expectRecordPrs(record, child, 3);
      await captureElectronWindow(app, `record-summaries-off-${theme}`, facts);
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-settings-tab="hideAi"]').click();
      await page.locator('[data-ai-agent-summary="false"]').click();
      await page.keyboard.press("Escape");
    }
    // The summaries back on, the record's ask returns. Read in the pane the operator typed in, it keeps the row out of 내 차례:
    // the row rests at the top of 쉬는 중 with the hollow `!` until it is answered (status-model, A question still waiting).
    await expect(record.locator("[data-record-ask]")).toHaveCount(1, { timeout: 45_000 }); // wait-ok: the record's ask returns with the summaries
    await openTaskFolds(page);
    await expect(root).toHaveAttribute("data-task-read", "true", { timeout: 45_000 }); // wait-ok: the core lays the read ask on the row
    await expect(tasks.locator('[data-task-fold="rest"] [data-task-row]').first()).toHaveAttribute("data-task-row", pane);
    await expect(root.locator('[data-mark="turn"]')).toHaveAttribute("data-mark-read", "true");
    await hoverSettledTaskRow(root, "resting");
    // Resolve from the row's ⋯ menu takes the question away once it is saved (B18): the row rests, no longer a read ask.
    await page.locator(`[data-task-more="${pane}"]`).click();
    await page.locator(`[data-task-menu="${pane}"] [data-menu-item="resolve"]`).click();
    await expect(root).not.toHaveAttribute("data-task-read", "true", { timeout: 30_000 }); // wait-ok: the core saves the resolve and takes the ask away
    await expect(root).toHaveAttribute("data-task-state", "resting");
    for (const theme of ["light", "dark"] as const) {
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.keyboard.press("Escape");
      await captureElectronWindow(app, `record-ended-${theme}`, { ...fixtureIdentity, theme, tools: await page.locator("[data-lane-frame]").boundingBox() });
    }
  } catch (error) { failures.push(error); }
  await finishNative(failures, [["close the app", () => app?.close()], ["stop the stack", () => stack.stop()]]);
});
