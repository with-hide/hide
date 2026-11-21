import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutomationCall } from "../../automationCall";
import type { AutomationSchedule } from "../../snapshot";
import "../chapters";
import chapter from "../chapters/automation";
import type { CoreEvent } from "../world";
import { AUTOMATION_ID, AUTOMATION_WORLD, LEAD, NIGHT, RUN_PANE, type AutomationModel } from "./automation";

// A Tuesday evening in the machine's own zone: the next weekday 09:00 is tomorrow's.
const TUESDAY_EVENING = new Date(2026, 10, 17, 20, 0).getTime();

beforeEach(() => vi.useFakeTimers({ now: TUESDAY_EVENING, toFake: ["Date"] }));
afterEach(() => vi.useRealTimers());

const doneAt = (id: string, model: AutomationModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: AutomationModel, event: CoreEvent) => AUTOMATION_WORLD.apply(model, event)!;
const event = (kind: string, payload: Record<string, unknown> = {}): CoreEvent => ({ kind, payload });
const called = (model: AutomationModel, call: AutomationCall) => AUTOMATION_WORLD.call!(model, call);
const section = (model: AutomationModel) => AUTOMATION_WORLD.snapshot(model, Date.now()).automations!;
const rows = (model: AutomationModel) => AUTOMATION_WORLD.snapshot(model, Date.now()).rest.navigator!.agents!;
const cards = (model: AutomationModel) => AUTOMATION_WORLD.snapshot(model, Date.now()).rest.navigator!.devices![0]!.agent_scope.tasks!.cards;
const focusRun = (reveal: boolean) => event("focus_pane", { pane_id: RUN_PANE, origin: "operator", ...(reveal ? { reveal_unseen: true } : {}) });
const viewing = (on: boolean, extra: Record<string, unknown> = {}) => event("automation_run_viewing", { id: AUTOMATION_ID, run: 1, on, ...extra });
/** An agent's own turn ending: the beat the world asks for, played. */
const beat = (model: AutomationModel) => apply(model, AUTOMATION_WORLD.later!(model)!.event);

const WEEKDAYS_AT_NINE = { kind: "weekdays", time: "09:00" } as const;
const made = () => {
  const model = AUTOMATION_WORLD.initial();
  const call = called(model, { call: "add", name: "Morning issue triage", task: "Read the new issues.", schedule: WEEKDAYS_AT_NINE, agent: { kind: "claude" }, worktree: false, run_now: false });
  return call.model!;
};
/** The night begun and its first beat played: the run goes. */
const running = () => beat(apply(made(), event(NIGHT)));
/** The run finished, with its briefing waiting in Needs You. */
const finished = () => beat(running());

describe("the automation flow's script (UI_BEHAVIOR, Practice window)", () => {
  it("starts with no automation and reads a schedule back with when it runs next", () => {
    const model = AUTOMATION_WORLD.initial();
    expect(section(model).items).toEqual([]);
    const answer = called(model, { call: "preview", schedule: WEEKDAYS_AT_NINE, name: "Morning issue triage" }).answer;
    if (!answer.ok) throw new Error(answer.code);
    const { reads_as, next_run_at_unix_ms, own_folder } = answer.value as { reads_as: unknown; next_run_at_unix_ms: number; own_folder: string };
    expect(reads_as).toEqual(WEEKDAYS_AT_NINE);
    expect(new Date(next_run_at_unix_ms)).toEqual(new Date(2026, 10, 18, 9, 0));
    expect(own_folder).toBe("/Users/me/hide/automations/Morning-issue-triage-…");
  });

  it("refuses a schedule as the core does, with the code the form words", () => {
    const model = AUTOMATION_WORLD.initial();
    const code = (schedule: AutomationSchedule) => {
      const answer = called(model, { call: "preview", schedule, name: "x" }).answer;
      return answer.ok ? null : answer.code;
    };
    expect(code({ kind: "weekly", days: [], time: "09:00" })).toBe("schedule_days_missing");
    expect(code({ kind: "every_minutes", minutes: 7 })).toBe("schedule_minutes_out_of_range");
    expect(code({ kind: "cron", expr: "every day" })).toBe("schedule_cron_invalid");
    expect(code({ kind: "cron", expr: "0 9 * * 1-5" })).toBeNull();
  });

  it("makes the automation on the call, and the screen then lists it with its next run", () => {
    const model = made();
    expect(doneAt("make", AUTOMATION_WORLD.initial())).toBe(false);
    expect(doneAt("make", model)).toBe(true);
    const [item] = section(model).items;
    expect(item).toMatchObject({ id: AUTOMATION_ID, name: "Morning issue triage", running: false, unseen: 0, reads_as: WEEKDAYS_AT_NINE });
    expect(new Date(item!.next_run_at_unix_ms!)).toEqual(new Date(2026, 10, 18, 9, 0));
    expect(item!.folder).toEqual({ path: "/Users/me/hide/automations/Morning-issue-triage-a1", default: true, worktree: false });
    // The practice holds one automation.
    const second = called(model, { call: "add", name: "Another", task: "Do it.", schedule: WEEKDAYS_AT_NINE, agent: { kind: "claude" } }).answer;
    expect(second).toEqual({ ok: false, code: "automation_cap" });
  });

  it("starts the night only when the morning step comes up, then runs by itself", () => {
    const model = made();
    expect(AUTOMATION_WORLD.later!(model)).toBeNull();
    const night = apply(model, event(NIGHT));
    expect(night.stage).toBe(2);
    expect(AUTOMATION_WORLD.later!(night)).not.toBeNull();
    // Told again, or before an automation exists, the night does not begin twice.
    expect(AUTOMATION_WORLD.apply(night, event(NIGHT))).toBeNull();
    expect(AUTOMATION_WORLD.apply(AUTOMATION_WORLD.initial(), event(NIGHT))).toBeNull();
  });

  it("shows the run going: its card running at the time the schedule names, and no row for it in any list", () => {
    const model = running();
    expect(doneAt("morning", model)).toBe(false);
    const [item] = section(model).items;
    expect(item!.running).toBe(true);
    const [run] = item!.records;
    expect(run).toMatchObject({ kind: "run", state: "running", trigger: "scheduled", pane_id: RUN_PANE });
    const startedAt = new Date((run as { started_at_unix_ms: number }).started_at_unix_ms);
    // The night's 09:00, drawn on the day the learner's own clock is on, so it reads as the schedule says.
    expect([startedAt.getHours(), startedAt.getMinutes()]).toEqual([9, 0]);
    expect(startedAt.getTime()).toBeLessThanOrEqual(Date.now());
    expect(new Date(item!.next_run_at_unix_ms!)).toEqual(new Date(2026, 10, 18, 9, 0));
    expect(AUTOMATION_WORLD.panes(model)).toEqual([LEAD, RUN_PANE]);
    expect(AUTOMATION_WORLD.screen(model, RUN_PANE, 80)).toContain("Reading the issues");
    expect(cards(model).map((card) => card.root)).toEqual([LEAD]);
  });

  it("ends the run with a briefing: the sidebar counts one, and the run stands in Needs You marked To look at", () => {
    const model = finished();
    expect(doneAt("morning", model)).toBe(true);
    expect(section(model).unseen).toBe(1);
    const [item] = section(model).items;
    expect(item).toMatchObject({ running: false, unseen: 1 });
    expect(item!.records[0]).toMatchObject({ state: "succeeded", seen: false });
    const row = rows(model).find((one) => one.pane_id === RUN_PANE)!;
    expect(row.mark).toBe("look");
    expect(row.look?.label).toBe("issue-briefing.md");
    // The row says whose it is, and the screen that owns it.
    expect(row.hide_ai).toEqual({ task: { kind: "automation", target: AUTOMATION_ID }, home: "automations" });
    expect(cards(model).map((card) => [card.root, card.state])).toEqual([[RUN_PANE, "needs_you"], [LEAD, "working"]]);
    expect(AUTOMATION_WORLD.screen(model, RUN_PANE, 80)).toContain("hide file open issue-briefing.md");
  });

  it("puts the run's folder in a workspace of its own that no list draws, its tab marked as an automation's", () => {
    const { rest } = AUTOMATION_WORLD.snapshot(finished(), Date.now());
    const folder = rest.navigator!.workspaces!.find((workspace) => workspace.id === "automation-a1")!;
    expect(folder.unlisted).toBe(true);
    expect(folder.checkouts.flatMap((checkout) => checkout.tabs).every((tab) => tab.automation === true)).toBe(true);
    expect(rest.navigator!.workspaces!.find((workspace) => workspace.id === "hide-practice")!.unlisted).toBeUndefined();
  });

  it("marks the run seen when its detail is opened, and the sidebar's number drops", () => {
    const model = apply(finished(), event("automation_seen", { id: AUTOMATION_ID }));
    expect(section(model).unseen).toBe(0);
    expect(section(model).items[0]!.records[0]).toMatchObject({ seen: true });
  });

  it("brings the briefing onto the run's desk when the screen shows the run, and takes the row out of Needs You", () => {
    const model = finished();
    // Before the screen shows the run, its desk is not the one in front and holds nothing.
    const before = AUTOMATION_WORLD.snapshot(model, Date.now());
    expect(before.rest.workspace_view!.layout!.display_count).toBe(0);
    expect(doneAt("result", model)).toBe(false);
    const shown = apply(apply(model, focusRun(true)), viewing(true));
    expect(doneAt("result", shown)).toBe(true);
    const after = AUTOMATION_WORLD.snapshot(shown, Date.now());
    expect(after.focused).toBe(RUN_PANE);
    expect(after.rest.navigator!.focused_workspace_id).toBe("automation-a1");
    expect(after.rest.workspace_view!.tray.tab_id).toBe("t1");
    expect(after.rest.workspace_view!.layout!.display_count).toBe(1);
    expect(after.frames!.documents!.file1!.contents_utf8).toContain("# Issue briefing");
    expect(cards(shown).map((card) => card.root)).toEqual([LEAD]);
    expect(rows(shown).find((one) => one.pane_id === RUN_PANE)!.mark).toBe("quiet");
  });

  it("holds the run's pane focus only while the screen shows it, leaving the briefing on its desk", () => {
    const shown = apply(apply(finished(), focusRun(true)), viewing(true));
    const left = apply(shown, viewing(false, { restore: LEAD }));
    const snapshot = AUTOMATION_WORLD.snapshot(left, Date.now());
    expect(snapshot.focused).toBe(LEAD);
    expect(left.gathered).toBe(true);
    // Shown again, the desk still holds it.
    const again = AUTOMATION_WORLD.snapshot(apply(left, focusRun(true)), Date.now());
    expect(again.rest.workspace_view!.layout!.display_count).toBe(1);
  });

  it("says the run viewed on the pane's focus alone, and asks nothing of a run that does not exist yet", () => {
    expect(doneAt("result", apply(finished(), focusRun(false)))).toBe(true);
    expect(AUTOMATION_WORLD.apply(made(), focusRun(true))).toBeNull();
    expect(AUTOMATION_WORLD.apply(made(), viewing(true))).toBeNull();
  });

  it("runs now when ▶ is pressed before the night, with the manual run's own tag", () => {
    const model = apply(made(), event("automation_run", { id: AUTOMATION_ID }));
    expect(section(model).items[0]!.records[0]).toMatchObject({ kind: "run", trigger: "manual", state: "running" });
    expect(doneAt("morning", beat(model))).toBe(true);
  });

  it("pauses and resumes, and a delete starts the practice over", () => {
    const paused = apply(made(), event("automation_pause", { id: AUTOMATION_ID }));
    expect(section(paused).items[0]).toMatchObject({ paused: { reason: "operator" }, next_run_at_unix_ms: null });
    expect(section(apply(paused, event("automation_resume", { id: AUTOMATION_ID }))).items[0]!.next_run_at_unix_ms).not.toBeNull();
    expect(section(apply(finished(), event("automation_remove", { id: AUTOMATION_ID }))).items).toEqual([]);
  });

  it("changes an edited automation's name and schedule, and reads the new schedule back", () => {
    const edited = called(made(), { call: "edit", id: AUTOMATION_ID, name: "Evening triage", schedule: { kind: "daily", time: "18:30" } });
    expect(edited.answer.ok).toBe(true);
    expect(section(edited.model!).items[0]).toMatchObject({ name: "Evening triage", reads_as: { kind: "daily", time: "18:30" } });
    // 18:30 has passed this evening, so the next one is tomorrow's.
    expect(new Date(section(edited.model!).items[0]!.next_run_at_unix_ms!)).toEqual(new Date(2026, 10, 18, 18, 30));
  });

  it("starts every step before the run with the shell on the lead's desk, and draws only the section the chapter needs", () => {
    const initial = AUTOMATION_WORLD.initial();
    expect(AUTOMATION_WORLD.panes(initial)).toEqual([LEAD]);
    expect(AUTOMATION_WORLD.snapshot(initial, Date.now()).rest.ui_state!.sidebar_lens).toBe("tasks");
    expect(chapter.steps.find((step) => step.id === "morning")!.enter).toEqual({ kind: NIGHT, payload: {} });
  });
});
