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
    await choose(page, "schedule-kind", page.locator('[data-automation-schedule-kind="every_minutes"]'));
    await expect(page.locator("[data-automation-preview]")).toContainText("Every 30 minutes");

    // B7: a cron that runs closer than five minutes and a week with no day are refused under the schedule, and nothing can be made.
    await choose(page, "schedule-kind", page.locator('[data-automation-schedule-kind="cron"]'));
    await page.locator('[data-automation-field="cron"]').fill("*/4 * * * *");
    await expect(page.locator('[data-automation-refusal="schedule_too_often"]')).toBeVisible();
    await expect(create).toBeDisabled();
    await choose(page, "schedule-kind", page.locator('[data-automation-schedule-kind="weekly"]'));
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
    await page.locator('[data-automation-example="issues"]').click();
    await expect(page.locator('[data-automation-field="name"]')).toHaveValue("Morning issue triage");
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

test("a run's screen draws its terminal beside the run's summary, then beside what its agent opened", async ({ page }) => {
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

    // Its agent opened nothing yet: the right side is the run's summary, its task first.
    await expect(page.locator('[data-automation-run-side="summary"]')).toContainText("보고서를 열어 줘");

    // The agent opens its report: the run's desk takes the right side with it in front, the terminal still on the left.
    const opened = await stack.fixture.runAs(pane)(["file", "open", report]);
    expect(opened.status, opened.out).toBe(0);
    const desk = page.locator('[data-automation-run-side="desk"]');
    await expect(desk).toBeVisible({ timeout: 30_000 }); // wait-ok: the core placed the agent's View on the run's desk, in front
    await expect(desk).toContainText("오늘의 보고서", { timeout: 30_000 }); // wait-ok: the document read from the run's checkout
    await expect(terminal).toBeVisible();

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
