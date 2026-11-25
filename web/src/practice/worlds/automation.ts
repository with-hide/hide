// The automation flow's world: one invented project with an agent at work,
// and the core's automations beside it. It answers the form's calls as the
// core does (a schedule read back with its next time, a make, an edit) and
// the screen's events (seen, shown, paused), and plays one automation through
// a night: made from a starting point, its scheduled run starts when the
// morning comes, the run's agent reads the issues and opens a briefing for
// the learner (a View), and the run's row stands in Needs You until the
// learner opens the run, where the terminal is on the left and the briefing
// on the desk on the right.
// The morning is the world's own beat (`later`), begun when the learner comes
// to that step (`enter`); a ▶ pressed before it starts the run at once.

import type { ProjectTasksScope } from "../../agentScope";
import type { AutomationCall } from "../../automationCall";
import type { AgentRow, Automation, AutomationAgent, AutomationRecord, AutomationSchedule, AutomationWeekday, PaneHeader, ViewDisplaySnapshot } from "../../snapshot";
import type { CoreEvent, PracticeWorld, WorldSnapshot } from "../world";
import { agent, applyDesk, checkoutId, counted, deskSnapshot, oneTabDesk, rowRequest, rowState, shellScreen, viewedFileFrames, withTasks, withViewedFile, type DeskModel, type PracticePlace, type ViewedFile } from "./kit";

const PLACE: PracticePlace = { project: "hide-practice", label: "hide-practice", branch: "main" };
/** The automation's own folder is a workspace of its own, which the core marks so that no list shows it. */
const RUN_PLACE: PracticePlace = { project: "automation-a1", label: "automation-a1", branch: "main" };
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export const LEAD = "lead";
/** The pane of the automation's run. */
export const RUN_PANE = "run1";
/** The one automation this world can make. */
export const AUTOMATION_ID = "a1";
const OWN_ROOT = "/Users/me/hide/automations";
const LEAD_TITLE = "Fix the checkout timeout";
const RUN_TITLE = "Morning briefing";
const BRIEFING_FILE = "morning-briefing.md";
/** The key of the briefing in the run's results: what a press on its row names. */
const BRIEFING_KEY = "briefing";

const BRIEFING = [
  "# Morning briefing",
  "",
  "Yesterday: 7 new issues, 4 pull requests, 9 notifications.",
  "",
  "## Look at first today",
  "",
  "- Issue 412: Checkout times out on slow cards (3 reports)",
  "- Pull request 87: Raise the payment timeout, waiting for your review",
  "- Issue 418: Refund email is sent twice",
  "",
  "## Can wait",
  "",
  "- 5 smaller issues and 8 notifications, grouped by area",
  "",
].join("\n");
const ANSWER = "Read yesterday's 7 issues, 4 pull requests and 9 notifications. Look first at issue 412 and pull request 87. The briefing is open for you.";

/**
 * Where the script stands: 0 before an automation exists, 1 made, 2 the
 * night (the learner came to the morning step), 3 the run going, 4 the run
 * finished with its briefing waiting to be looked at.
 */
export type AutomationStage = 0 | 1 | 2 | 3 | 4;

/** How long each stage lasts before the next, by the stage it ends: the night, then the run. */
export const BEATS: Partial<Record<AutomationStage, number>> = { 2: 1200, 3: 2400 };
const BEAT = "practice_beat";
/** What the morning step tells the world when it comes up: the night begins. */
export const NIGHT = "practice_night";

/** The automation the learner made, as the form sent it. */
type Made = { name: string; task: string; schedule: AutomationSchedule; agent: AutomationAgent; folder: string; paused: boolean; next: number | null };
/** The one run: whether it was asked for, and when it began and ended. */
type Run = { manual: boolean; startedAt: number; endedAt: number };

export type AutomationModel = {
  desk: DeskModel;
  stage: AutomationStage;
  /** When each stage began, which paces the next beat. */
  times: Partial<Record<AutomationStage, number>>;
  made: Made | null;
  run: Run | null;
  /** Whether the automation's detail has been opened since the run finished. */
  seen: boolean;
  /** The run's terminal shows in the screen, and the core's focus is on its pane. */
  viewing: boolean;
  focus: boolean;
  /** What the run's agent opened is on the run's desk, and the run was dealt with. */
  gathered: boolean;
  resolved: boolean;
  /** The run's tab holds its summary card, placed when the screen first showed the run (the core keeps it on the desk after). */
  placed: boolean;
};

/** Whether an automation was made. */
export const isMade = (model: AutomationModel) => model.made !== null;
/** Whether the run finished and its row stands in Needs You, not yet looked at. */
export const runWaits = (model: AutomationModel) => model.stage >= 4 && !model.resolved;
/** Whether the screen shows the run: the learner opened it, or the core's focus is on its pane. */
export const runViewed = (model: AutomationModel) => model.viewing || model.focus;
/** Whether the run's agent has finished its work. */
export const runFinished = (model: AutomationModel) => model.stage >= 4;

function initial(): AutomationModel {
  const lead = agent({ pane_id: LEAD, identity_label: LEAD_TITLE, agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "4m" });
  return {
    desk: oneTabDesk({ id: LEAD, agent: lead, title: LEAD_TITLE }),
    stage: 0, times: { 0: Date.now() }, made: null, run: null, seen: false, viewing: false, focus: false, gathered: false, resolved: false, placed: false,
  };
}

const since = (model: AutomationModel, stage: AutomationStage) => model.times[stage] ?? Date.now();
const to = (model: AutomationModel, stage: AutomationStage): AutomationModel => ({ ...model, stage, times: { ...model.times, [stage]: Date.now() } });

// --- Schedules ---------------------------------------------------------------------------------

const DAY_INDEX: Record<AutomationWeekday, number> = { sun: 0, mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, sat: 6 };
const WEEKDAYS: readonly number[] = [1, 2, 3, 4, 5];
/** What the form offers for minutes and hours, as the core's schedule allows them. */
const MINUTES = [5, 10, 15, 30];
const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;

type Read = { ok: true; reads_as: AutomationSchedule } | { ok: false; code: string };

/** What the core reads a schedule as, or why it refuses it. A cron line that equals a choice is that choice. */
function read(schedule: AutomationSchedule): Read {
  switch (schedule.kind) {
    case "daily":
    case "weekdays":
      return TIME.test(schedule.time) ? { ok: true, reads_as: schedule } : { ok: false, code: "schedule_time_invalid" };
    case "weekly":
      if (!TIME.test(schedule.time)) return { ok: false, code: "schedule_time_invalid" };
      return schedule.days.length > 0 ? { ok: true, reads_as: schedule } : { ok: false, code: "schedule_days_missing" };
    case "every_minutes":
      return MINUTES.includes(schedule.minutes) ? { ok: true, reads_as: schedule } : { ok: false, code: "schedule_minutes_out_of_range" };
    case "every_hours":
      if (!TIME.test(schedule.time)) return { ok: false, code: "schedule_time_invalid" };
      return schedule.hours >= 1 && schedule.hours <= 12 ? { ok: true, reads_as: schedule } : { ok: false, code: "schedule_hours_out_of_range" };
    case "cron": {
      // Only the two lines that read as a choice, since the practice has no cron parser of its own.
      const match = /^(\d{1,2}) (\d{1,2}) \* \* (\*|1-5)$/.exec(schedule.expr);
      if (!match) return { ok: false, code: "schedule_cron_invalid" };
      const time = `${match[2]!.padStart(2, "0")}:${match[1]!.padStart(2, "0")}`;
      return TIME.test(time) ? { ok: true, reads_as: { kind: match[3] === "*" ? "daily" : "weekdays", time } } : { ok: false, code: "schedule_cron_invalid" };
    }
    case "once": {
      // A once schedule reads as itself while its time is ahead, as the core's does (mission-104).
      if (!TIME.test(schedule.time)) return { ok: false, code: "schedule_time_invalid" };
      const at = onceAt(schedule);
      if (Number.isNaN(at)) return { ok: false, code: "schedule_date_invalid" };
      return at > Date.now() ? { ok: true, reads_as: schedule } : { ok: false, code: "schedule_once_past" };
    }
  }
}

/** A once schedule's time on this machine's clock. */
function onceAt(schedule: Extract<AutomationSchedule, { kind: "once" }>): number {
  return new Date(`${schedule.date}T${schedule.time}:00`).getTime();
}

/** The next time `schedule` fires after `from`, in this machine's zone. */
function nextAfter(schedule: AutomationSchedule, from: number): number | null {
  const at = (days: number, time: string) => {
    const [hours, minutes] = time.split(":").map(Number);
    const day = new Date(from);
    day.setDate(day.getDate() + days);
    day.setHours(hours!, minutes!, 0, 0);
    return day.getTime();
  };
  switch (schedule.kind) {
    case "daily":
    case "weekdays":
    case "weekly": {
      for (let days = 0; days < 8; days += 1) {
        const when = at(days, schedule.time);
        const weekday = new Date(when).getDay();
        const allowed = schedule.kind === "daily" || (schedule.kind === "weekdays" ? WEEKDAYS.includes(weekday) : schedule.days.some((day) => DAY_INDEX[day] === weekday));
        if (when > from && allowed) return when;
      }
      return null;
    }
    case "every_minutes": {
      const step = schedule.minutes * MINUTE;
      return (Math.floor(from / step) + 1) * step;
    }
    case "every_hours": {
      const step = schedule.hours * HOUR;
      const base = at(0, schedule.time);
      return base + (Math.floor((from - base) / step) + 1) * step;
    }
    case "cron": {
      const reading = read(schedule);
      return reading.ok && reading.reads_as.kind !== "cron" ? nextAfter(reading.reads_as, from) : null;
    }
    case "once": {
      const when = onceAt(schedule);
      return when > from ? when : null;
    }
  }
}

/** Where the automation works: its own folder under the root, named for the automation, unless a project was chosen. */
const ownFolder = (name: string) => `${OWN_ROOT}/${name.trim().replace(/\s+/g, "-")}-${AUTOMATION_ID}`;

// --- The run -----------------------------------------------------------------------------------

/**
 * The run begins. A scheduled one is at its time, which the practice's clock
 * has not reached: the time is drawn a whole number of days back, as the day
 * the night ended on, so it reads as its schedule says (`오늘 09:00`) while
 * the learner's own clock stays where it is. An interval has no such time and
 * begins now.
 */
function startRun(model: AutomationModel, manual: boolean): AutomationModel {
  const made = model.made!;
  const now = Date.now();
  const interval = made.schedule.kind === "every_minutes" || made.schedule.kind === "every_hours";
  const due = made.next ?? now;
  const startedAt = manual || interval ? now : due - Math.max(0, Math.ceil((due - now) / DAY)) * DAY;
  const run = { manual, startedAt, endedAt: startedAt + 4 * MINUTE };
  return to({ ...model, run, made: { ...made, next: nextAfter(made.schedule, Math.max(startedAt, now)) } }, 3);
}

// --- Events and calls --------------------------------------------------------------------------

function apply(model: AutomationModel, event: CoreEvent): AutomationModel | null {
  const p = event.payload;
  switch (event.kind) {
    case NIGHT:
      return model.stage === 1 ? to(model, 2) : null;
    case BEAT:
      if (p.stage !== model.stage + 1 || BEATS[model.stage] === undefined) return null;
      return model.stage === 2 ? startRun(model, false) : to({ ...model, gathered: model.gathered || model.focus, resolved: model.viewing }, 4);
    case "automation_seen":
      return model.stage >= 4 && !model.seen ? { ...model, seen: true } : null;
    // The run's terminal shows, or no longer does; a finished run shown is dealt with, and its row leaves Needs You.
    case "automation_run_viewing": {
      if (model.stage < 3 || typeof p.on !== "boolean") return null;
      const viewing = p.on;
      const resolved = model.resolved || (viewing && model.stage >= 4);
      const focus = viewing || typeof p.restore !== "string" ? model.focus : false;
      const placed = model.placed || viewing;
      return viewing === model.viewing && resolved === model.resolved && focus === model.focus && placed === model.placed ? null : { ...model, viewing, resolved, focus, placed };
    }
    // A press on a result row brings that View in front on the run's desk, as the core does with one the agent opened.
    case "automation_run_result_open":
      return model.stage >= 4 && model.placed && p.key === BRIEFING_KEY && !model.gathered ? { ...model, gathered: true } : null;
    // The window moves the core's focus to the run's pane; with `reveal_unseen` what the run opened comes onto its desk.
    case "focus_pane": {
      if (p.pane_id === RUN_PANE) {
        if (model.stage < 3) return null;
        const gathered = model.gathered || (p.reveal_unseen === true && model.stage >= 4);
        return model.focus && gathered === model.gathered ? null : { ...model, focus: true, gathered };
      }
      const desk = applyDesk(model.desk, event);
      return desk && { ...model, desk, focus: false };
    }
    case "automation_run":
      return model.made && !model.made.paused && model.stage <= 2 ? startRun(model, true) : null;
    case "automation_pause":
      return model.made && !model.made.paused ? { ...model, made: { ...model.made, paused: true } } : null;
    case "automation_resume":
      return model.made?.paused ? { ...model, made: { ...model.made, paused: false, next: nextAfter(model.made.schedule, Date.now()) } } : null;
    case "automation_remove":
      return model.made ? { ...initial(), desk: model.desk } : null;
    default: {
      const desk = applyDesk(model.desk, event);
      return desk && { ...model, desk };
    }
  }
}

type Answered = ReturnType<NonNullable<PracticeWorld<AutomationModel>["call"]>>;
const refused = (code: string): Answered => ({ model: null, answer: { ok: false, code } });

function call(model: AutomationModel, request: AutomationCall): Answered {
  switch (request.call) {
    case "preview": {
      const reading = read(request.schedule);
      if (!reading.ok) return refused(reading.code);
      const slug = request.name.trim().replace(/\s+/g, "-");
      const value = { reads_as: reading.reads_as, next_run_at_unix_ms: nextAfter(reading.reads_as, Date.now()), zone: null, own_folder: `${OWN_ROOT}/${slug}${slug ? "-" : ""}…` };
      return { model: null, answer: { ok: true, value } };
    }
    case "add": {
      // The practice holds one automation.
      if (model.made) return refused("automation_cap");
      const reading = read(request.schedule);
      if (!reading.ok) return refused(reading.code);
      const made: Made = { name: request.name, task: request.task, schedule: request.schedule, agent: request.agent, folder: request.folder ?? ownFolder(request.name), paused: false, next: nextAfter(reading.reads_as, Date.now()) };
      const added = to({ ...model, made }, 1);
      const started = request.run_now ? startRun(added, true) : added;
      const value = { id: AUTOMATION_ID, name: made.name, folder: made.folder, next_run_at_unix_ms: made.next, ...(request.run_now ? { run: 1 } : {}) };
      return { model: started, answer: { ok: true, value } };
    }
    case "edit": {
      if (!model.made || request.id !== AUTOMATION_ID) return refused("automations_unavailable");
      const schedule = request.schedule ?? model.made.schedule;
      const reading = read(schedule);
      if (!reading.ok) return refused(reading.code);
      const made: Made = { ...model.made, name: request.name ?? model.made.name, task: request.task ?? model.made.task, schedule, agent: request.agent ?? model.made.agent, next: nextAfter(reading.reads_as, Date.now()) };
      return { model: { ...model, made }, answer: { ok: true, value: { id: AUTOMATION_ID, next_run_at_unix_ms: made.next } } };
    }
    // The run's pane stands open here, so an opening has nothing to start.
    case "run_open":
      return model.made && model.run && request.id === AUTOMATION_ID ? { model: null, answer: { ok: true, value: { id: request.id, run: request.run } } } : refused("automations_unavailable");
  }
}

// --- The rows ----------------------------------------------------------------------------------

const BRIEFING_VIEW = "view-briefing";
const runTitleRequest = (model: AutomationModel, at: number) => rowRequest(model.made!.task, RUN_TITLE, at, null);

/** The run agent's row as the core sends it: out of the lists while it works, in Needs You marked `볼 것` once its briefing waits, and put away once the run was looked at. */
function runRow(model: AutomationModel): AgentRow {
  const made = model.made!;
  const base = {
    pane_id: RUN_PANE, identity_label: RUN_TITLE, agent_kind: made.agent.kind, session_id: "practice-run",
    hide_ai: { task: { kind: "automation", target: AUTOMATION_ID }, home: "automations" as const },
  };
  const began = since(model, 3);
  if (model.stage < 4) {
    return agent({ ...base, group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m", unlisted: true, state: rowState("working", true, began), request: runTitleRequest(model, began) });
  }
  const at = since(model, 4);
  if (model.resolved) {
    return agent({ ...base, group: "seen", mark: "quiet", status_code: "done", activity: "stopped", elapsed: "1m", unlisted: true, resolved: { at_unix_ms: at, source: "operator" }, state: rowState("quiet", true, at), request: runTitleRequest(model, began) });
  }
  return agent({
    ...base, group: "needs_you", mark: "look", status_code: "done", activity: "stopped", unread: true, elapsed: "4s",
    state: rowState("look", true, at), request: runTitleRequest(model, began),
    look: { group: "done", count: 1, view_id: BRIEFING_VIEW, label: BRIEFING_FILE, workspace_id: RUN_PLACE.project, checkout_id: checkoutId(RUN_PLACE) },
  });
}

function headerOf(model: AutomationModel, pane: string): PaneHeader {
  if (pane === LEAD) return { working: true, notice: null, band: null, request: { text: LEAD_TITLE, images: 0 }, progress: "Editing src/payment/timeout.ts" };
  if (model.stage >= 4) return { working: false, notice: null, band: null, request: { text: RUN_TITLE, images: 0 }, finished: true, progress: "The briefing is ready" };
  return { working: true, notice: null, band: null, request: { text: RUN_TITLE, images: 0 }, progress: "Reading yesterday's issues, pull requests and notifications" };
}

/** The 태스크 list: the lead at work, and the run once it waits in 내 차례. */
function tasks(model: AutomationModel, nowMs: number): ProjectTasksScope {
  const recency = new Date(nowMs).toISOString();
  const lead = { root: LEAD, state: "working" as const, members: [LEAD], checkouts: [checkoutId(PLACE)], recency, stage: 1 as const };
  if (!runWaits(model)) return { cards: [lead], needs_you: 0, recency };
  return { cards: [{ root: RUN_PANE, state: "needs_you", members: [RUN_PANE], checkouts: [checkoutId(RUN_PLACE)], recency, stage: 1 }, lead], needs_you: 1, recency };
}

// --- The automation ----------------------------------------------------------------------------

function record(model: AutomationModel, run: Run): AutomationRecord {
  const done = model.stage >= 4;
  return {
    kind: "run", number: 1, trigger: run.manual ? "manual" : "scheduled", ...(run.manual ? {} : { due_unix_ms: run.startedAt }), started_at_unix_ms: run.startedAt,
    ...(done ? { ended_at_unix_ms: run.endedAt } : {}), state: done ? "succeeded" : "running", attempt: 1, pane_id: RUN_PANE, opened: false, resumable: true,
    conversation_missing: false, seen: model.seen || !done, ...(done ? { answer: ANSWER } : {}),
    // What the agent opened rides the run only while the screen shows it (`results`, operator choice 2026-11-24).
    ...(done && model.viewing ? { results: [{ key: BRIEFING_KEY, kind: "file" as const, label: BRIEFING_FILE }] } : {}),
  };
}

const unseenOf = (model: AutomationModel) => (model.stage >= 4 && !model.seen ? 1 : 0);

function automation(model: AutomationModel): Automation {
  const made = model.made!;
  const reading = read(made.schedule);
  return {
    id: AUTOMATION_ID, name: made.name, task: made.task, schedule: made.schedule, reads_as: reading.ok ? reading.reads_as : made.schedule,
    next_run_at_unix_ms: made.paused ? null : made.next, agent: made.agent, folder: { path: made.folder, default: made.folder.startsWith(`${OWN_ROOT}/`), worktree: false },
    ...(made.paused ? { paused: { reason: "operator" as const, at_unix_ms: since(model, 1) } } : {}),
    // The practice's automations repeat (S-8), so none is ever a once automation done.
    running: model.stage === 3, done: false, unseen: unseenOf(model), records: model.run ? [record(model, model.run)] : [],
  };
}

// --- The snapshot ------------------------------------------------------------------------------

/** The run tab's desk: its summary card first and fixed, then what its agent opened, which is in front once it is there. */
type WorldView = NonNullable<WorldSnapshot["rest"]["workspace_view"]>;

function runDeskView(view: WorldView, briefing: ViewedFile | null): WorldView {
  const summary: ViewDisplaySnapshot = { id: "summary1", tab_id: null, path: "", label: "", kind: "card", committed: null, preview: false, state: "open", reason: null, card: { kind: "automation_run", automation: AUTOMATION_ID, run: 1 } };
  const opened = briefing ? withViewedFile({ ...view, open: true }, briefing) : { ...view, open: true, tray: { ...view.tray, shown: true } };
  const files = opened.layout?.root && "area" in opened.layout.root ? opened.layout.root.area.displays : [];
  const displays = [summary, ...files];
  return {
    ...opened,
    tray: { ...opened.tray, shown: true, count: displays.length },
    layout: { root: { area: { id: "v1", active: (files.at(-1) ?? summary).id, displays } }, active_area: "v1", display_count: displays.length, limits: { areas: 4, depth: 3, displays: 32 } },
  };
}

function snapshot(model: AutomationModel, nowMs: number): WorldSnapshot {
  const runDesk = model.made && model.stage >= 3 ? oneTabDesk({ id: RUN_PANE, agent: runRow(model), title: RUN_TITLE }) : null;
  const desks = [{ place: PLACE, model: model.desk }, ...(runDesk ? [{ place: RUN_PLACE, model: runDesk }] : [])];
  const front = model.focus && runDesk ? RUN_PLACE.project : PLACE.project;
  const drawn = deskSnapshot(desks, front, nowMs);
  // An agent not at work is counted where its row stands; the run's tab is the automation's own, which no tab list draws.
  const lined = runWaits(model) ? counted(drawn.rest, "needs_you") : model.stage >= 4 ? counted(drawn.rest, "seen") : drawn.rest;
  const base = withTasks(lined, tasks(model, nowMs));
  const navigator = base.navigator!;
  const workspaces = navigator.workspaces!.map((workspace) => (workspace.id === RUN_PLACE.project
    ? { ...workspace, unlisted: true, checkouts: workspace.checkouts.map((checkout) => ({ ...checkout, tabs: checkout.tabs.map((tab) => ({ ...tab, automation: true })) })) }
    : workspace));
  const headers: Record<string, PaneHeader> = { [LEAD]: headerOf(model, LEAD), ...(runDesk ? { [RUN_PANE]: headerOf(model, RUN_PANE) } : {}) };
  // The window's focus on the run's pane makes its tab's desk the one in front; it holds what the run opened once the core brought it there.
  const briefing: ViewedFile = { path: `${model.made?.folder ?? OWN_ROOT}/${BRIEFING_FILE}`, label: BRIEFING_FILE, contents: BRIEFING, workspaceId: RUN_PLACE.project, checkoutId: checkoutId(RUN_PLACE) };
  const showsBriefing = model.focus && model.gathered && runDesk !== null;
  const showsSummary = model.focus && model.placed && runDesk !== null;
  return {
    ...drawn,
    rest: {
      ...base,
      navigator: { ...navigator, workspaces },
      ui_state: { ...base.ui_state!, sidebar_lens: "tasks" },
      workspace_view: showsSummary ? runDeskView(base.workspace_view!, showsBriefing ? briefing : null) : base.workspace_view,
      terminal: { headers },
    },
    focused: model.focus && runDesk ? RUN_PANE : drawn.focused,
    frames: showsBriefing ? viewedFileFrames(briefing) : {},
    automations: {
      items: model.made ? [automation(model)] : [],
      unseen: unseenOf(model),
      last_choice: { kind: "claude" },
      own_folder_root: OWN_ROOT,
    },
  };
}

// --- What the terminals show -------------------------------------------------------------------

const CLAUDE = "\x1b[38;2;215;119;87m";
const DIM = "\x1b[90m";
const BOLD = "\x1b[1m";
const GREEN = "\x1b[32m";
const RESET = "\x1b[0m";

function leadScreen(): string {
  return [
    "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}~/${PLACE.project} · ${PLACE.branch}${RESET}`,
    "", `${DIM}>${RESET} Checkout times out on slow cards. Fix it`, "",
    `${CLAUDE}●${RESET} The timeout is fixed at three seconds.`,
    `  ${DIM}⎿${RESET}  Editing src/payment/timeout.ts…`,
    "", `${CLAUDE}✻ ${DIM}Working… (esc to interrupt)${RESET}`,
  ].join("\r\n");
}

function runScreen(model: AutomationModel): string {
  const made = model.made!;
  const rows = [
    "", `${CLAUDE}✻${RESET} ${BOLD}Claude Code${RESET}`, `  ${DIM}${made.folder.replace(/^\/Users\/me/, "~")}${RESET}`,
    "", `${DIM}>${RESET} ${made.task}`, "",
    `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(gh issue list --search "created:>=yesterday")`,
    `  ${DIM}⎿${RESET}  7 issues opened since yesterday`,
    `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(gh pr list --search "updated:>=yesterday")`,
    `  ${DIM}⎿${RESET}  4 pull requests, 1 waiting for your review`,
    `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(gh api notifications)`,
    `  ${DIM}⎿${RESET}  9 notifications`,
  ];
  if (model.stage < 4) return [...rows, "", `${CLAUDE}✻ ${DIM}Reading yesterday… (esc to interrupt)${RESET}`].join("\r\n");
  rows.push(
    `${CLAUDE}●${RESET} ${BOLD}Write${RESET}(${BRIEFING_FILE})`, `  ${DIM}⎿${RESET}  Wrote 13 lines`,
    `${CLAUDE}●${RESET} ${BOLD}Bash${RESET}(hide file open ${BRIEFING_FILE})`, `  ${DIM}⎿${RESET}  Opened for you to look at`,
    "", `${CLAUDE}●${RESET} ${GREEN}Done.${RESET} ${ANSWER}`,
  );
  return rows.join("\r\n");
}

export const AUTOMATION_WORLD: PracticeWorld<AutomationModel> = {
  initial,
  apply,
  call,
  snapshot,
  // The night and the run end by themselves, as an agent at work would.
  later: (model) => {
    const wait = BEATS[model.stage];
    return wait === undefined ? null : { at: since(model, model.stage) + wait, event: { kind: BEAT, payload: { stage: model.stage + 1 } } };
  },
  panes: (model) => [LEAD, ...(model.made && model.stage >= 3 ? [RUN_PANE] : [])],
  screen: (model, paneId) => (paneId === LEAD ? leadScreen() : paneId === RUN_PANE && model.made ? runScreen(model) : shellScreen(PLACE)),
};
