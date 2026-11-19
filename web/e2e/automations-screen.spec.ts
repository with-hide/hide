// The automation screen on a real core through an isolated engine, hided and
// browser (PRD automations B1-B8, B11, B15-B18, B23, B25-B27, B30, B38): the
// sidebar's 자동화 row, the table, the form the core reads as it is typed, a
// run's terminal live and opened again, and an automation stopped for a
// person in Needs You. The run's agent is the fixture's `claude` stand-in,
// which reports its session as Claude Code does, so a finished run's
// conversation can be opened again; an automation whose project folder is
// removed from the disk stops for a person.

import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentIn, openProject, withControl } from "./daemon-actions";
import { startCaller, type CallerFixture } from "./engine-fixture";
import { afterCleanup } from "./worker-owned";
import { setAgentState } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { sendEvent } from "./wire";

test.describe.configure({ timeout: 240_000 });

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
  await expect(page.locator("[data-sidebar-automations]")).toBeVisible({ timeout: 30_000 });
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

    // B4, B5: the form opens in the automation's own folder, named under the home.
    await page.locator("[data-automation-new]").first().click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "form");
    await expect(page.locator("[data-automation-own-folder]")).toContainText("~/hide/automations/");
    await expect(page.locator('[data-automation-field="worktree"]')).toHaveCount(0);
    const create = page.locator("[data-automation-create]");
    await expect(create).toBeDisabled();
    await page.locator('[data-automation-field="name"]').fill("아침 점검");
    await page.locator('[data-automation-field="task"]').fill("의존성 점검");
    // B6: the default schedule is read back with its next run.
    await expect(page.locator("[data-automation-preview]")).toBeVisible();
    await expect(create).toBeEnabled();

    // B7: a cron under an hour and a week with no day are refused under the schedule, and nothing can be made.
    await choose(page, "schedule-kind", page.locator('[data-automation-schedule-kind="cron"]'));
    await page.locator('[data-automation-field="cron"]').fill("*/5 * * * *");
    await expect(page.locator('[data-automation-refusal="schedule_under_hour"]')).toBeVisible();
    await expect(create).toBeDisabled();
    await choose(page, "schedule-kind", page.locator('[data-automation-schedule-kind="weekly"]'));
    await page.locator('[data-automation-day="mon"]').click();
    await expect(page.locator('[data-automation-refusal="schedule_days_missing"]')).toBeVisible();
    await expect(create).toBeDisabled();
    await page.locator('[data-automation-day="wed"]').click();
    await expect(page.locator("[data-automation-preview]")).toBeVisible();
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

test("a run started now is live in the screen and typed into, asks in Needs You under its automation, stops by the operator, and opens again", async ({ page }) => {
  const stack = await start(page, "automations-run");
  try {
    await page.locator("[data-sidebar-automations]").click();
    await page.locator("[data-automation-new]").first().click();
    await page.locator('[data-automation-field="name"]').fill("야간 점검");
    await page.locator('[data-automation-field="task"]').fill("의존성 점검");
    await expect(page.locator("[data-automation-preview]")).toBeVisible();
    // B8, B16: made and run at once, it opens on the manual run's terminal, live and taking keys.
    await page.locator("[data-automation-create-run]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "run");
    await expect(page.locator('[data-automation-run-head="1"] [data-automation-run-tag="manual"]')).toBeVisible();
    const terminal = page.locator('[data-automation-run-terminal="pane"]');
    await expect(terminal).toBeVisible({ timeout: 60_000 });
    const pane = await terminal.getAttribute("data-automation-run-pane");
    if (!pane) throw new Error("the run's terminal names no pane");
    await expect(terminal.locator(".xterm-rows")).toContainText("claude fixture ready", { timeout: 30_000 });
    await terminal.locator(".xterm-screen").click();
    await page.keyboard.type("hello run");
    await expect(terminal.locator(".xterm-rows")).toContainText("hello run");

    // B30: its own folder is a workspace the core lists, and no project row in the sidebar.
    await withControl(stack.daemon, (control) => control.until((rest) => ((rest.navigator as { workspaces?: { unlisted?: boolean }[] } | undefined)?.workspaces ?? []).some((workspace) => workspace.unlisted) || undefined, "the automation's own folder listed unlisted", 30_000));
    await expect(page.locator("nav[data-sidebar] [data-project]")).toHaveCount(1);

    // The stand-in reported its session as Claude Code does, so the run has a conversation to open again (B17).
    await withControl(stack.daemon, (control) => control.until((rest) => agentIn(rest, pane)?.session_id || undefined, `the run's agent in ${pane} on its session`, 30_000));
    // B15: the run's agent held at a permission prompt raises it in Needs You under its automation, and the row opens the run.
    await setAgentState(stack.daemon, pane, "blocked");
    const raised = page.locator(`[data-project-list] [data-raised-group="needs_you"] [data-pane="${pane}"]`);
    await expect(raised).toHaveCount(1, { timeout: 30_000 });
    await expect(raised.locator("[data-agent-task]")).toContainText("야간 점검");
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    await raised.locator("[data-agent-open]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "run");

    // B16: [■ 멈추기] ends the run as the operator's stop.
    await page.locator('[data-automation-run-stop="1"]').click();
    await page.locator("[data-automation-run-head] [data-automation-back]").click();
    const run = page.locator('[data-automation-record="run"][data-automation-record-number="1"]');
    await expect(run).toHaveAttribute("data-automation-run-state", "stopped", { timeout: 30_000 });
    await expect(run).toHaveAttribute("data-automation-run-reason", "stopped_by_person");

    // B17: the finished run opens its conversation again in the screen, and reads as opened.
    await run.click();
    await expect(terminal).toBeVisible({ timeout: 60_000 });
    await expect(terminal.locator(".xterm-rows")).toContainText("claude fixture ready", { timeout: 30_000 });
    await expect(page.locator('[data-automation-run-head] [data-automation-run-tag="opened"]')).toBeVisible();
  } finally {
    stop(stack);
  }
});

test("an automation whose folder is gone stops with its own Needs You line, and 다시 켜기 takes the line away", async ({ page }) => {
  const stack = await start(page, "automations-ask");
  try {
    // B23: the project it works in is removed from the disk, so the run asked for now stops the automation for a person.
    const folder = path.join(stack.fixture.root, "reports");
    fs.mkdirSync(folder);
    await withControl(stack.daemon, (control) => openProject(control, folder, "reports"));
    await expect(page.locator("nav[data-sidebar] [data-project]", { hasText: "reports" })).toHaveCount(1, { timeout: 30_000 });
    const id = await add(page, stack.daemon, { name: "보고서 정리", task: "보고서 정리", agent: { kind: "claude" }, folder: fs.realpathSync.native(folder) });
    fs.rmSync(folder, { recursive: true });
    await sendEvent(page, stack.daemon, "automation_run", { id });
    const ask = page.locator(`[data-project-list] [data-raised-group="needs_you"] [data-automation-ask="${id}"]`);
    await expect(ask).toHaveCount(1, { timeout: 60_000 });
    await expect(ask).toHaveAttribute("data-automation-ask-reason", "folder_missing");
    await expect(ask).toContainText("보고서 정리");
    await expect(ask.locator('[data-automation-ask-line="folder_missing"]')).toBeVisible();

    // The row opens the automation, which reads paused with the run that failed.
    await ask.locator("[data-automation-ask-open]").click();
    await expect(screen(page)).toHaveAttribute("data-automation-screen", "detail");
    await expect(page.locator("[data-automation-resume]")).toBeVisible();
    await expect(page.locator('[data-automation-record="run"]').first()).toHaveAttribute("data-automation-run-state", "failed");

    // [다시 켜기] turns it on and the line goes.
    await ask.locator(`[data-automation-ask-resume="${id}"]`).click();
    await expect(ask).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-automation-pause]")).toBeVisible();
  } finally {
    stop(stack);
  }
});
