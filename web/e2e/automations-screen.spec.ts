// The automation screen on a real core through an isolated engine, hided and
// browser (PRD automations B1-B8, B11, B15-B17, B25-B27, B30, B38): the
// sidebar's 자동화 row, the table, the form the core reads as it is typed, and
// a run's terminal live, raised in Needs You and opened again. The run's agent
// is the fixture's `claude` stand-in, which reports its session as Claude Code
// does, so a finished run's conversation can be opened again.

import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { untilLogged, withControl } from "./daemon-actions";
import { startCaller, type CallerFixture } from "./engine-fixture";
import { afterCleanup } from "./worker-owned";
import { setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { sendEvent } from "./wire";

test.describe.configure({ timeout: 240_000 }); // wait-ok: a fixture with an agent pane, a daemon and a run's agent, started and stopped whole

type Stack = { fixture: CallerFixture; daemon: Daemon };

/** A Git project `fixture` with one agent, its `claude` reporting its session, and the page open on it. */
async function start(page: Page, label: string): Promise<Stack> {
  await page.setViewportSize({ width: 1440, height: 900 });
  const fixture = await startCaller();
  try {
    const daemon = await startHided(fixture, label);
    await open(page, daemon);
    return { fixture, daemon };
  } catch (error) {
    throw afterCleanup(error, () => fixture.stop());
  }
}

function stop(stack: Stack): void {
  try { stack.daemon.stop(); } finally { stack.fixture.stop(); }
}

async function open(page: Page, daemon: Daemon): Promise<void> {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expect(page.locator("[data-sidebar-automations]")).toBeVisible({ timeout: 30_000 }); // wait-ok: the first snapshot after hided starts, which draws the sidebar
}

/** Makes an automation through the same call the form sends, answered to this window. */
async function add(page: Page, daemon: Daemon, call: Record<string, unknown>): Promise<string> {
  const frame = await sendEvent(page, daemon, "automation_call", { request_id: `e2e-${Date.now()}`, call: { call: "add", schedule: { kind: "daily", time: "09:00" }, worktree: false, run_now: false, ...call } }, "automation_result");
  const answer = (frame?.payload ?? {}) as { ok?: boolean; value?: { id?: string }; code?: string };
  if (!answer.ok || !answer.value?.id) throw new Error(`the core refused the automation: ${answer.code}`);
  return answer.value.id;
}

async function choose(page: Page, field: string, option: Locator): Promise<void> {
  await page.locator(`[data-automation-field="${field}"]`).click();
  await option.click();
}

const screen = (page: Page) => page.locator("[data-automation-screen]");

/** `date` moved by `days` whole days, read as a calendar date with no clock or zone. */
function shiftDate(date: string, days: number): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(Date.UTC(year!, month! - 1, day! + days)).toISOString().slice(0, 10);
}

/** The number on the sidebar's Needs You entry, 0 while it draws none. */
async function askingCount(page: Page): Promise<number> {
  const count = page.locator("[data-overview-count]");
  if ((await count.count()) === 0) return 0;
  return Number((await count.textContent())?.match(/\d+/)?.[0] ?? 0);
}

/** The wall date and minute `ahead` minutes from now on this machine, whose zone hided reads a once time in. */
function wallMinute(ahead: number): { date: string; time: string } {
  const at = new Date(Date.now() + ahead * 60_000);
  const two = (n: number) => String(n).padStart(2, "0");
  return { date: `${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())}`, time: `${two(at.getHours())}:${two(at.getMinutes())}` };
}

test("the 자동화 row opens an empty table, the form refuses what the core refuses, and an automation is made, paused, edited and removed", async ({ page }) => {
  const stack = await start(page, "automations-form");
  try {
    // B1, B2: the row opens the screen and reads as chosen; nothing is made yet.
    const row = page.locator("[data-sidebar-automations]");
    await row.click();
    await expect(row).toHaveAttribute("aria-current", "page");
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "table");
    await expect(page.locator('[data-automation-screen-state="empty"]')).toBeVisible();

    // B4, B5: the form opens in the automation's own folder, with no worktree to ask for.
    await page.locator("[data-automation-new]").first().click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "form");
    await expect(page.locator('[data-automation-field="worktree"]')).toHaveCount(0);
    const create = page.locator("[data-automation-create]");
    await expect(create).toBeDisabled();
    // B7: a name left empty is one red line under it once the field is left.
    await page.locator('[data-automation-field="task"]').click();
    await expect(page.locator('[data-automation-refusal="automation_name_missing"]')).toBeVisible();
    await page.locator('[data-automation-field="name"]').fill("아침 점검");
    await expect(page.locator('[data-automation-refusal="automation_name_missing"]')).toHaveCount(0);
    await page.locator('[data-automation-field="task"]').fill("의존성 점검");
    // B5: the core names the own folder by the name, under the home, its id still to come.
    await expect(page.locator("[data-automation-own-folder]")).toHaveText("~/hide/automations/아침-점검-…");
    // B6: the default schedule is read back with its next run.
    await expect(page.locator("[data-automation-preview]")).toBeVisible();
    await expect(create).toBeEnabled();

    // B6: every N minutes reads back as itself.
    await page.locator('[data-automation-schedule-kind="every_minutes"]').click();
    await expect(page.locator("[data-automation-preview]")).toContainText("Every 30 minutes");

    // B7: a cron that runs closer than five minutes and a week with no day are refused under the schedule, and nothing can be made.
    await page.locator('[data-automation-schedule-kind="cron"]').click();
    await page.locator('[data-automation-field="cron"]').fill("*/4 * * * *");
    await expect(page.locator('[data-automation-refusal="schedule_too_often"]')).toBeVisible();
    await expect(create).toBeDisabled();
    await page.locator('[data-automation-schedule-kind="weekly"]').click();
    await page.locator('[data-automation-day="mon"]').click();
    await expect(page.locator('[data-automation-refusal="schedule_days_missing"]')).toBeVisible();
    await expect(create).toBeDisabled();
    await page.locator('[data-automation-day="wed"]').click();
    await expect(create).toBeEnabled();

    // B5: a Git project offers a new worktree per run; the own folder does not.
    await choose(page, "folder", page.getByRole("option", { name: /fixture/ }));
    await expect(page.locator('[data-automation-field="worktree"]')).toBeVisible();
    await expect(page.locator("[data-automation-own-folder]")).toHaveCount(0);
    await choose(page, "folder", page.getByRole("option").first());
    await expect(page.locator('[data-automation-field="worktree"]')).toHaveCount(0);

    // B8, B11: made, it opens on its detail; the table holds its row.
    await create.click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");
    await expect(page.locator("[data-automation-task]")).toHaveText("의존성 점검");
    await page.locator("[data-automation-back]").click();
    const made = page.locator("[data-automation-row]");
    await expect(made).toHaveCount(1);
    await expect(made).toContainText("아침 점검");
    // B38: back on the table, the keys are on its first row, not lost to the page.
    await expect(made.nth(0).locator("[data-automation-open]")).toBeFocused();

    // B38: ↑ and ↓ move between rows, and Enter opens the focused one.
    await add(page, stack.daemon, { name: "저녁 정리", task: "로그 정리", agent: { kind: "claude" } });
    await expect(made).toHaveCount(2);
    await made.nth(0).locator("[data-automation-open]").focus();
    await page.keyboard.press("ArrowDown");
    await expect(made.nth(1).locator("[data-automation-open]")).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(made.nth(0).locator("[data-automation-open]")).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");

    // B25: paused, it reads paused in its detail and its row.
    const name = await page.locator("[data-automation-detail-head] h1").textContent();
    await page.locator("[data-automation-pause]").click();
    await expect(page.locator("[data-automation-resume]")).toBeVisible();
    await page.locator("[data-automation-back]").click();
    await expect(page.locator("[data-automation-row]", { hasText: name ?? "" })).toHaveAttribute("data-automation-row-paused", "operator");
    await page.locator("[data-automation-row]", { hasText: name ?? "" }).locator("[data-automation-open]").click();
    await page.locator("[data-automation-resume]").click();
    await expect(page.locator("[data-automation-pause]")).toBeVisible();

    // B26: the edit opens filled, and a saved name is the automation's.
    await page.locator("[data-automation-edit]").click();
    await expect(page.locator('[data-automation-field="name"]')).toHaveValue(name ?? "");
    await page.locator('[data-automation-field="name"]').fill("주간 점검");
    await page.locator("[data-automation-save]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");
    await expect(page.locator("[data-automation-detail-head] h1")).toHaveText("주간 점검");

    // B27: removing asks first; keeping it keeps it, and removing it takes its row.
    await page.locator("[data-automation-remove]").click();
    await page.locator("[data-automation-remove-cancel]").click();
    await expect(page.locator("[data-automation-remove-confirm]")).toHaveCount(0);
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");
    await page.locator("[data-automation-remove]").click();
    await page.locator("[data-automation-remove-yes]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "table");
    await expect(page.locator("[data-automation-row]")).toHaveCount(1);
    await expect(page.locator("[data-automation-row]", { hasText: "주간 점검" })).toHaveCount(0);
  } finally {
    stop(stack);
  }
});

test("a run started now is live in the screen and typed into, asks in Needs You under its automation, and stops by the operator", async ({ page }) => {
  const stack = await start(page, "automations-run");
  try {
    await page.locator("[data-sidebar-automations]").click();
    // B2, B4: a starting point of the empty screen opens the form filled with its name, task and schedule.
    await page.locator('[data-automation-example="briefing"]').click();
    await expect(page.locator('[data-automation-field="name"]')).toHaveValue("Morning briefing");
    await expect(page.locator("[data-automation-preview]")).toContainText("Weekdays 09:00");
    await page.locator('[data-automation-field="name"]').fill("야간 점검");
    await page.locator('[data-automation-field="task"]').fill("의존성 점검");
    // B8, B16: made and run at once, it opens on the manual run's terminal, live and taking keys.
    await page.locator("[data-automation-create-run]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "run");
    await expect(page.locator('[data-automation-run-head="1"] [data-automation-run-tag="manual"]')).toBeVisible();
    const terminal = page.locator('[data-automation-run-terminal="pane"]');
    await expect(terminal).toBeVisible({ timeout: 60_000 }); // wait-ok: the run's agent started by the core and its pane attached to the screen
    const pane = await terminal.getAttribute("data-automation-run-pane");
    if (!pane) throw new Error("the run's terminal names no pane");
    await expect(terminal.locator(".xterm-rows")).toContainText("claude fixture ready", { timeout: 30_000 }); // wait-ok: the fixture's claude stand-in printed its ready line in the run's pane
    await terminal.locator(".xterm-screen").click();
    await page.keyboard.type("hello run");
    await expect(terminal.locator(".xterm-rows")).toContainText("hello run");

    // B30: its own folder is a workspace the core lists, and no project row in the sidebar.
    await withControl(stack.daemon, (control) => control.until((rest) => ((rest.navigator as { workspaces?: { unlisted?: boolean }[] } | undefined)?.workspaces ?? []).some((workspace) => workspace.unlisted) || undefined, "the automation's own folder listed unlisted", 30_000));
    await expect(page.locator("nav[data-sidebar] [data-project]")).toHaveCount(1);

    // B15: the run's agent held at a permission prompt raises it in Needs You under its automation, and the row opens the run.
    await setAgentState(stack.daemon, pane, "blocked");
    const raised = page.locator(`[data-project-list] [data-raised-group="needs_you"] [data-pane="${pane}"]`);
    await expect(raised).toHaveCount(1, { timeout: 30_000 }); // wait-ok: the core read the blocked state and raised the run in Needs You
    await expect(raised.locator("[data-agent-task]")).toContainText("야간 점검");
    // It wears the automation's repeat arrow in its turn badge (operator decision 2026-11-21).
    await expect(raised.locator('[data-agent-status-mark][data-mark-automation="repeat"]')).toHaveCount(1);
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    await raised.locator("[data-agent-open]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "run");

    // B16: [■ 멈추기] ends the run as the operator's stop.
    await page.locator('[data-automation-run-stop="1"]').click();
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    const run = page.locator('[data-automation-record="run"][data-automation-record-number="1"]');
    await expect(run).toHaveAttribute("data-automation-run-state", "stopped", { timeout: 30_000 }); // wait-ok: the core recorded the stop the run's agent answered
    await expect(run).toHaveAttribute("data-automation-run-reason", "stopped_by_person");
  } finally {
    stop(stack);
  }
});

test("a finished run whose terminal the core closed opens its conversation again in the screen", async ({ page }) => {
  const stack = await start(page, "automations-reopen");
  try {
    const id = await add(page, stack.daemon, { name: "야간 점검", task: "의존성 점검", agent: { kind: "claude" }, run_now: true });
    await page.locator("[data-sidebar-automations]").click();
    await page.locator(`[data-automation-row="${id}"] [data-automation-open]`).click();
    const run = page.locator('[data-automation-record="run"][data-automation-record-number="1"]');
    await run.click();
    const terminal = page.locator('[data-automation-run-terminal="pane"]');
    await expect(terminal).toBeVisible({ timeout: 60_000 }); // wait-ok: the run's agent started by the core and its pane attached to the screen
    const pane = await terminal.getAttribute("data-automation-run-pane");
    if (!pane) throw new Error("the run's terminal names no pane");
    // B17: the run works and finishes its turn; looked at and no longer shown, its terminal closes.
    await setAgentState(stack.daemon, pane, "working");
    await setAgentState(stack.daemon, pane, "idle");
    await expect(page.locator('[data-automation-run-head="1"]')).toHaveAttribute("data-automation-run-state", "succeeded", { timeout: 60_000 }); // wait-ok: the run's agent answered and the core recorded the run as done
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    await untilLogged(stack.daemon, { kind: "automation.terminal_closed", pane_id: pane }, 1, "the run's terminal closed", 60_000);
    // Pressed again, it opens the conversation the stand-in reported, reading as opened.
    await run.click();
    await expect(page.locator('[data-automation-run-head="1"] [data-automation-run-tag="opened"]')).toBeVisible({ timeout: 60_000 }); // wait-ok: the core opened the finished run's conversation again
    await expect(terminal.locator(".xterm-rows")).toContainText("claude fixture ready", { timeout: 30_000 }); // wait-ok: the resumed claude stand-in printed its ready line
  } finally {
    stop(stack);
  }
});

test("a run's screen draws its terminal beside the run's desk, which leads with a fixed summary tab and lists what its agent opened", async ({ page }) => {
  const stack = await start(page, "automations-pair");
  try {
    const project = fs.realpathSync.native(path.join(stack.fixture.root, "fixture"));
    const report = path.join(project, "report.md");
    fs.writeFileSync(report, "# 오늘의 보고서\n");
    const id = await add(page, stack.daemon, { name: "보고서", task: "보고서를 열어 줘", agent: { kind: "claude" }, folder: project, run_now: true });
    await page.locator("[data-sidebar-automations]").click();
    await page.locator(`[data-automation-row="${id}"] [data-automation-open]`).click();
    await page.locator('[data-automation-record="run"][data-automation-record-number="1"]').click();
    const terminal = page.locator('[data-automation-run-terminal="pane"]');
    await expect(terminal).toBeVisible({ timeout: 60_000 }); // wait-ok: the run's agent started by the core and its pane attached to the screen
    const pane = await terminal.getAttribute("data-automation-run-pane");
    if (!pane) throw new Error("the run's terminal names no pane");

    // Its agent opened nothing yet: the run's desk holds its summary alone, first and fixed (no close button), its task shown, and the 결과 heading with its quiet line.
    const desk = page.locator('[data-automation-run-side="desk"]');
    const summaryTab = desk.locator('[role="tab"][data-tab-kind="card"]');
    await expect(summaryTab).toHaveCount(1, { timeout: 30_000 }); // wait-ok: the core placed the run's summary card on its desk as the screen showed the run
    await expect(desk.locator('[data-card="automation_run"]')).toContainText("보고서를 열어 줘");
    await expect(desk.locator("[data-automation-run-results-empty]")).toBeVisible();
    await expect(summaryTab.locator("button")).toHaveCount(0);

    // The agent opens its report: it comes in front as a tab after the summary, which stays first and unclosable; the terminal stays on the left.
    const opened = await stack.fixture.runAs(pane)(["file", "open", report]);
    expect(opened.status, opened.out).toBe(0);
    const fileTab = desk.locator('[role="tab"][data-tab-kind="file"]');
    await expect(fileTab).toHaveAttribute("aria-selected", "true", { timeout: 30_000 }); // wait-ok: the core placed the agent's View on the run's desk, in front
    await expect(desk).toContainText("오늘의 보고서", { timeout: 30_000 }); // wait-ok: the document read from the run's checkout
    await expect(desk.locator('[role="tab"]').first()).toHaveAttribute("data-tab-kind", "card");
    await expect(summaryTab).toHaveAttribute("aria-selected", "false");
    await expect(summaryTab.locator("button")).toHaveCount(0);
    await expect(terminal).toBeVisible();

    // The summary lists what the agent opened; the file's tab closed, its row opens it again in front.
    await summaryTab.click();
    const row = desk.locator('[data-automation-run-result-kind="file"]');
    await expect(row).toContainText("report.md");
    await fileTab.hover();
    await fileTab.locator("button").click();
    await expect(fileTab).toHaveCount(0);
    await expect(row).toHaveCount(1);
    await row.locator("[data-automation-run-result-open]").click();
    await expect(fileTab).toHaveAttribute("aria-selected", "true", { timeout: 30_000 }); // wait-ok: the core opened the closed View again from what it kept
    await expect(desk).toContainText("오늘의 보고서");

    // Its tab stays out of the project's tab row and the sidebar's tree (B12), and leaving the run gives the focus back to the pane the operator was on.
    type Rest = { focused?: { pane_id?: string }; workspace_view?: { agent_layout?: { groups?: { tabs: string[] }[] } }; navigator?: { workspaces?: { checkouts: { tabs: { id?: string; panes: { id: string }[] }[] }[] }[] } };
    const runTab = (rest: Rest) => rest.navigator?.workspaces?.flatMap((workspace) => workspace.checkouts).flatMap((checkout) => checkout.tabs).find((tab) => tab.panes.some((row) => row.id === pane))?.id;
    const hidden = await withControl(stack.daemon, (control) => control.until((rest) => {
      const tab = runTab(rest as Rest);
      const rows = (rest as Rest).workspace_view?.agent_layout?.groups?.flatMap((group) => group.tabs) ?? [];
      return tab && rows.length > 0 && !rows.includes(tab) ? tab : undefined;
    }, "the run's tab left out of the tab row", 30_000));
    await expect(page.locator(`[data-sidebar-tab-group="${hidden}"]`)).toHaveCount(0);
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    await withControl(stack.daemon, (control) => control.until((rest) => ((rest as Rest).focused?.pane_id === stack.fixture.pane ? true : undefined), "the focus back on the operator's pane", 30_000));
  } finally {
    stop(stack);
  }
});

// mission-104 S-1 to S-4: a once automation made from the form a week ahead
// waits in its own group; one due at the next minute runs once, stands in
// Needs You with the clock until its run is opened, and folds with the done.
test("a once automation waits in its group, runs once at its minute, stands in Needs You with the clock until its run is opened, and folds under the done ones", async ({ page }) => {
  test.setTimeout(360_000); // wait-ok: a once time is a wall minute, so the run waits up to two minutes for it
  const stack = await start(page, "automations-once");
  try {
    await page.locator("[data-sidebar-automations]").click();
    await page.locator("[data-automation-new]").first().click();
    await page.locator('[data-automation-field="name"]').fill("Webhook check");
    await page.locator('[data-automation-field="task"]').fill("Read the webhook logs");
    // S-2: once comes first and opens on tomorrow; a past date is the core's refusal and nothing can be made.
    await expect(page.locator("[data-automation-schedule-kind]").first()).toHaveAttribute("data-automation-schedule-kind", "once");
    await page.locator('[data-automation-schedule-kind="once"]').click();
    // Every date below is counted from the one the form opened on, never from this test's own clock, so a day turning
    // between two reads moves both alike (docs/TESTING.md).
    const dateField = page.locator('[data-automation-field="date"]');
    await expect(dateField).toHaveValue(/^\d{4}-\d{2}-\d{2}$/);
    const opened = await dateField.inputValue();
    const create = page.locator("[data-automation-create]");
    await dateField.fill(shiftDate(opened, -3));
    await expect(page.locator('[data-automation-refusal="schedule_once_past"]')).toBeVisible();
    await expect(create).toBeDisabled();
    // A week ahead is six days after tomorrow, and reads back as its date, 7 days off.
    await page.locator('[data-automation-once-quick="7"]').click();
    const week = shiftDate(opened, 6);
    await expect(dateField).toHaveValue(week);
    await expect(page.locator('[data-automation-preview="once"]')).toContainText(`${week.slice(5)} 09:00 once · in 7d`);
    await create.click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");
    await page.locator("[data-automation-back]").click();

    // S-1: only the groups that hold something are drawn; the once one wears its date.
    await expect(page.locator("[data-automation-group]")).toHaveCount(1);
    const waiting = page.locator('[data-automation-group="once"]');
    await expect(waiting).toContainText(week.slice(5));

    // One due at the next minute (the minute after if this one is nearly over), and a repeating one.
    const due = wallMinute(new Date().getSeconds() < 45 ? 1 : 2);
    const id = await add(page, stack.daemon, { name: "Migration check", task: "Check the tables", agent: { kind: "claude" }, schedule: { kind: "once", ...due } });
    await add(page, stack.daemon, { name: "Morning issues", task: "Sort issues", agent: { kind: "claude" } });
    await expect(page.locator("[data-automation-group]")).toHaveCount(2);
    await expect(waiting.locator("[data-automation-open]")).toHaveCount(2);

    // Its run's agent starts at that minute, works and finishes its turn, with no one looking at it.
    const pane = await withControl(stack.daemon, (control) => control.until((rest) => ((rest.navigator as { agents?: { pane_id: string; hide_ai?: { task?: { target?: string } } }[] } | undefined)?.agents ?? []).find((row) => row.hide_ai?.task?.target === id)?.pane_id, "the once run's agent", 180_000));
    // The count is read before the run's agent moves, since the turn's end is what puts the line in Needs You.
    const askingBefore = await askingCount(page);
    await setAgentState(stack.daemon, pane, "working");
    await setAgentState(stack.daemon, pane, "idle");

    // S-4: its line stands in Needs You with the clock and its run's result, and the count of Needs You takes it in.
    const look = page.locator(`[data-project-list] [data-automation-look="${id}"]`);
    await expect(look).toHaveCount(1, { timeout: 60_000 }); // wait-ok: the core recorded the run as ended and put its line in Needs You
    await expect(look.locator('[data-mark-automation="once"]')).toHaveCount(1);
    await expect(look.locator('[data-automation-look-line="ok"]')).toContainText("Automation · once · Done");
    await expect.poll(() => askingCount(page)).toBe(askingBefore + 1);

    // S-1: done, it leaves the waiting group for the fold, closed until pressed.
    await expect(waiting.locator("[data-automation-open]")).toHaveCount(1);
    const toggle = page.locator("[data-automation-done-toggle]");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(page.locator(`[data-automation-done-row="${id}"]`)).toHaveCount(0);
    await toggle.click();
    await expect(page.locator(`[data-automation-done-row="${id}"]`)).toHaveAttribute("data-automation-done-tone", "ok");

    // S-1, B38: the arrows walk the drawn order, 반복 then 한 번 then the fold and its open rows, and back.
    const repeating = page.locator('[data-automation-group="repeating"] [data-automation-open]');
    await repeating.first().focus();
    await page.keyboard.press("ArrowDown");
    await expect(waiting.locator("[data-automation-open]")).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(toggle).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(page.locator(`[data-automation-done-row="${id}"]`)).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(toggle).toBeFocused();

    // S-4: the line opens that run, and opened, the core takes the line away.
    await look.locator(`[data-automation-look-open="${id}"]`).click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "run");
    await expect(page.locator('[data-automation-run-head="1"]')).toBeVisible();
    await expect(look).toHaveCount(0);
    // S-3: its detail says done, with no pause to press.
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    await expect(page.locator('[data-automation-next="done"]').first()).toBeVisible();
    await expect(page.locator("[data-automation-pause]")).toHaveCount(0);
  } finally {
    stop(stack);
  }
});
