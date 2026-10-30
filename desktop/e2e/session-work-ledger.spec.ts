// One background candidate, with production record workers and private native
// conversations. GitHub and drafting stay controlled in this visual fixture.
import { expect, type ElectronApplication, type Locator } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { continueFixtureTranscript, setFixtureLifecycle, startHerdr } from "../../web/e2e/herdr-fixture";
import { FIRST_GOAL, NEXT_GOAL, nextWork, seedFirstWork, workFixture, workRepositoryReady } from "../../web/e2e/session-work-fixture";
import { chord } from "../../web/e2e/chords";
import { enterWorkspace, openTaskFolds, setLanePinned, showExplorer, taskRow } from "../../web/e2e/wire";
import { fitWindow, isolate, launchShell, REPO, test } from "./fixture";
import { captureElectronWindow } from "./electron-window";

/** This session's PRs and the child's, each in its own group; the settled ones open from their fold (PRD session-record-goal-flow B18, B19). */
async function expectRecordPrs(record: Locator, child: string, total: number) {
  await expect(record.locator(`[data-record-prs="${total}"]`)).toBeVisible({ timeout: 45_000 });
  const fold = record.locator("[data-record-settled]");
  if (await fold.count()) await fold.click();
  await expect(record.locator(`[data-record-pr-group="${child}"] [data-record-pr-row="34"]`)).toBeVisible();
}

test("@platform session records by goal flow, ended work and search render in both themes in a narrow native lane", async () => {
  test.setTimeout(240_000);
  const herdr = await startHerdr();
  const github = workFixture(herdr);
  const run = isolate(herdr, "session-record");
  Object.assign(run.env, github.env);
  let app: ElectronApplication | undefined;
  const failures: unknown[] = [];
  try {
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 1440, height: 1000 });
    await workRepositoryReady(run.env.HIDE_STATE_DIR!);
    await enterWorkspace(page, "fixture");
    // The lane pinned, so the 보기 slot stays docked through the sheets below (PRD right-lane B5).
    await showExplorer(page);
    await setLanePinned(page, true);
    const first = Date.now();
    github.clock(first, first);
    const { session, child } = seedFirstWork(herdr, first);
    const root = await taskRow(page, herdr.panes[0], FIRST_GOAL);
    const tasks = page.locator("[data-task-lens]");
    await root.hover();
    await page.locator(`[data-task-record="${herdr.panes[0]}"]`).click();
    // The record opens in the lane's 보기 slot (PRD sidebar-task-lens B15).
    const record = page.locator('aside[data-lane="detail"][data-detail="record"] [data-session-record="lane"]');
    await expect(record.locator("[data-record-flow]")).toHaveCount(1, { timeout: 45_000 });
    // The record reads the session's one list (status-model, One list per session): #32, which the pane holds on
    // the checkout's branch, joins this session's and its child's from the start.
    await expectRecordPrs(record, child, 3);
    // The rail keeps a button for the slot: pressed while the viewing column draws it, folding it when pressed then, and pressing it again comes back (B46; PRD lane-two-columns B11).
    const slotButton = page.locator('[data-lane-tool="detail"][data-detail-slot="record"]');
    await expect(slotButton).toHaveAttribute("aria-pressed", "true");
    await slotButton.click();
    await expect(record).toHaveCount(0);
    await expect(slotButton).toHaveAttribute("aria-pressed", "false");
    await slotButton.click();
    await expect(record).toBeVisible();
    await expect(slotButton).toHaveAttribute("aria-pressed", "true");
    await page.locator(`[data-pane-view="${herdr.panes[0]}"] .xterm-helper-textarea`).focus();
    await page.keyboard.type(NEXT_GOAL);
    await page.keyboard.press("Enter");
    const second = Date.now();
    github.clock(first, second);
    continueFixtureTranscript(herdr, session, nextWork(second));
    await setFixtureLifecycle(herdr, herdr.panes[0], "working");
    await expect(root).toContainText(NEXT_GOAL, { timeout: 45_000 });
    await setFixtureLifecycle(herdr, herdr.panes[0], "idle");
    // The new goal opens a second flow, the agent's question waits on top, and the child's PR stays in its group (B3, B11, B18).
    await expect(record.locator("[data-record-flow]")).toHaveCount(2, { timeout: 45_000 });
    await expect(record.locator("[data-record-ask]")).toHaveCount(1, { timeout: 45_000 });
    await expectRecordPrs(record, child, 3);
    // The child is a row of the parent's record (B20); the capture keeps the parent in front.
    await expect(record.locator(`[data-record-child="${child}"]`)).toBeVisible();
    const fixtureIdentity = { head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: REPO, encoding: "utf8" }).trim(), home: run.env.HOME, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, daemon: run.daemonPid(), parent: herdr.panes[0], child, provider: "scripted", github: "scripted" };
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
    // Resolve from the row's ⋯ menu puts the task under 쉬는 중 once it is saved (B18).
    await root.hover();
    await page.locator(`[data-task-more="${herdr.panes[0]}"]`).click();
    await page.locator(`[data-task-menu="${herdr.panes[0]}"] [data-menu-item="resolve"]`).click();
    await expect(tasks.locator('[data-task-fold="rest"]')).toBeVisible({ timeout: 30_000 });
    await openTaskFolds(page);
    await expect(root).toHaveAttribute("data-task-state", "resting");
    for (const theme of ["light", "dark"] as const) {
      await page.locator('[data-open-settings]').click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.keyboard.press("Escape");
      await captureElectronWindow(app, `record-ended-${theme}`, { ...fixtureIdentity, theme, tools: await page.locator("[data-lane-frame]").boundingBox() });
    }
  } catch (error) { failures.push(error); }
  finally {
    try { await app?.close(); } catch (error) { failures.push(error); }
    try { run.cleanup(); } catch (error) { failures.push(error); }
    try { herdr.stop(); } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "native session record or teardown failed");
});
