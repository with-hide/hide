// One background candidate, with production record workers and private native
// conversations. GitHub and drafting stay controlled in this visual fixture.
import { expect, type ElectronApplication, type Locator } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { continueFixtureTranscript, setFixtureLifecycle, startHerdr } from "../../web/e2e/herdr-fixture";
import { FIRST_GOAL, MISSED, NEXT_GOAL, nextWork, seedFirstWork, workFixture, workRepositoryReady } from "../../web/e2e/session-work-fixture";
import { chord } from "../../web/e2e/chords";
import { enterWorkspace, openTaskFolds, setLanePinned, showExplorer, taskRow } from "../../web/e2e/wire";
import { fitWindow, isolate, launchShell, REPO, test } from "./fixture";
import { captureElectronWindow } from "./electron-window";

/** The approved record separates the session's PRs from a child's expandable record. */
async function expectRecordOutputs(record: Locator, child: string, own: number) {
  await record.getByRole("tab", { name: /^PR/ }).click();
  await expect(record.locator('[data-record-output="pr"]')).toHaveCount(own, { timeout: 45_000 });
  const row = record.locator(`[data-record-child="${child}"]`);
  await row.locator(":scope > button").click();
  await row.getByRole("tab", { name: /^PR/ }).click();
  await expect(row.locator('[data-record-output="pr"]')).toHaveCount(1, { timeout: 45_000 });
  await expect(record.locator('[data-record-output="pr"]')).toHaveCount(own + 1);
  await row.locator(":scope > button").click();
}

/** The record, its request-history tab, then the pane's checklist through the app menu's chord. */
async function captureRecordAndTaskList(app: ElectronApplication, pane: string, theme: string, facts: Record<string, unknown>) {
  const page = await app.firstWindow();
  const record = page.locator('aside[data-lane="detail"][data-detail="record"] [data-session-record="lane"]');
  await captureElectronWindow(app, `record-tasks-children-${theme}`, facts);
  await record.getByRole("tab", { name: /^(Request history|요청 기록)/ }).click();
  await record.locator("[data-record-words]").scrollIntoViewIfNeeded();
  await expect(record.locator("[data-record-request]").first()).toBeVisible();
  await captureElectronWindow(app, `record-words-${theme}`, facts);
  await page.locator(`[data-pane-view="${pane}"] .xterm-helper-textarea`).focus();
  await page.keyboard.press(chord("toggle_task_list", "electron"));
  const list = page.locator(`[data-pane-task-list="${pane}"]`);
  await expect(list.locator("[data-task-list-name]")).toHaveCount(3);
  await captureElectronWindow(app, `pane-task-list-${theme}`, facts);
  await page.keyboard.press(chord("toggle_task_list", "electron"));
  await expect(list).toHaveCount(0);
}

/** Agent summaries off keeps no tasks, so the pane head draws no chip (PRD task-split-checklist B9). */
async function captureSummariesOff(app: ElectronApplication, pane: string, theme: string, facts: Record<string, unknown>) {
  await expect((await app.firstWindow()).locator(`[data-pane-task-chip="${pane}"]`)).toHaveCount(0);
  await captureElectronWindow(app, `record-summaries-off-${theme}`, facts);
}

test("@platform session work records, missed lines, ended work and search render in both themes in a narrow native lane", async () => {
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
    await expect(record.locator('[data-record-work]').first().locator('[data-record-task]')).toHaveCount(1, { timeout: 45_000 });
    await expectRecordOutputs(record, child, 1);
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
    // The parent's three tasks, the second request's two deliverables one each; the child's is in its own record, behind its row (B20).
    await expect(record.locator('[data-record-task]')).toHaveCount(3, { timeout: 45_000 });
    await expectRecordOutputs(record, child, 2);
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
      await captureRecordAndTaskList(app, herdr.panes[0], theme, facts);
      await record.getByRole("tab", { name: /^(Findings|발견사항)/ }).click();
      const missed = record.locator('[data-record-missed] [data-missed-work]', { hasText: MISSED });
      // An open line in its own section offers 백로그로 and 무시, with no drafted form (PRD overview-device-tasks B43; PRD record-card-readability B12).
      await expect(missed.locator("[data-missed-backlog-add]")).toBeVisible();
      await captureElectronWindow(app, `record-missed-open-${theme}`, facts);
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
      await expect(record.locator('[data-record-task]')).toHaveCount(0);
      // The parent's two requests; the child's is in its own record, behind its row.
      await record.getByRole("tab", { name: /^(Request history|요청 기록)/ }).click();
      await expect(record.locator('[data-record-request]')).toHaveCount(2);
      await expect(record.getByRole('heading', { level: 2 })).toContainText('Claude');
      await expect(record.locator('[data-record-child]')).toContainText('Claude');
      await expectRecordOutputs(record, child, 2);
      await captureSummariesOff(app, herdr.panes[0], theme, facts);
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
