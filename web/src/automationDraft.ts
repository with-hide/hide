// The automation form's values (PRD automations B4-B8, B26): what it opens
// on, the call a press sends, and which refusal a field shows. The core reads
// and refuses every value (`hide_core::automations::checked_*`); the form only
// holds what the operator typed and words the core's answer, with the two
// empty fields it can see for itself, so [만들기] waits for nothing to say so.

import type { AutomationCall } from "./automationCall";
import { BUILT_IN_PROFILE } from "./agentProfiles";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import type { Automation, AutomationAgent, AutomationSchedule, AutomationWeekday } from "./snapshot";

export type ScheduleKind = AutomationSchedule["kind"];

/** Every schedule field the form keeps, so switching the kind back finds what was typed (B6). */
export type ScheduleDraft = { kind: ScheduleKind; time: string; days: AutomationWeekday[]; minutes: number; hours: number; expr: string };

export type FormDraft = {
  name: string;
  task: string;
  agent: { kind: ProviderKind; profile: string; model: string | null; effort: string | null };
  /** A project's folder; null works in the automation's own folder (B5). */
  folder: string | null;
  worktree: boolean;
  schedule: ScheduleDraft;
};

const NEW_SCHEDULE: ScheduleDraft = { kind: "daily", time: "09:00", days: ["mon"], minutes: 30, hours: 1, expr: "0 9 * * 1-5" };

function scheduleDraft(schedule: AutomationSchedule): ScheduleDraft {
  switch (schedule.kind) {
    case "daily":
    case "weekdays":
      return { ...NEW_SCHEDULE, kind: schedule.kind, time: schedule.time };
    case "weekly":
      return { ...NEW_SCHEDULE, kind: "weekly", time: schedule.time, days: schedule.days };
    case "every_minutes":
      return { ...NEW_SCHEDULE, kind: "every_minutes", minutes: schedule.minutes };
    case "every_hours":
      return { ...NEW_SCHEDULE, kind: "every_hours", time: schedule.time, hours: schedule.hours };
    case "cron":
      return { ...NEW_SCHEDULE, kind: "cron", expr: schedule.expr };
  }
}

/**
 * The starting points the empty screen offers (PRD automations B2, B4): a
 * schedule the form opens on, with the name and task the screen words; the
 * operator reads, changes and makes it like any new automation.
 */
export type AutomationExample = "report" | "briefing" | "ci";
export const AUTOMATION_EXAMPLES: Record<AutomationExample, AutomationSchedule> = {
  report: { kind: "weekly", days: ["mon"], time: "09:00" },
  briefing: { kind: "weekdays", time: "09:00" },
  ci: { kind: "every_hours", hours: 6, time: "00:00" },
};
export const AUTOMATION_EXAMPLE_KEYS = Object.keys(AUTOMATION_EXAMPLES) as AutomationExample[];

/** A new automation's form opened on an example: the agent last chosen, the example's words and schedule. */
export function exampleDraft(example: AutomationExample, lastChoice: AutomationAgent | undefined, words: { name: string; task: string }): FormDraft {
  return { ...draftOf(null, lastChoice), name: words.name, task: words.task, schedule: scheduleDraft(AUTOMATION_EXAMPLES[example]) };
}

function agentDraft(agent: AutomationAgent | undefined): FormDraft["agent"] {
  const kind = PROVIDER_KINDS.find((one) => one === agent?.kind) ?? PROVIDER_KINDS[0];
  if (!kind) throw new Error("No start adapter declared");
  // The last choice of another agent is not this one's (D-07).
  const same = kind === agent?.kind;
  return { kind, profile: (same && agent?.profile) || BUILT_IN_PROFILE, model: (same && agent?.model) || null, effort: (same && agent?.effort) || null };
}

/** A new automation's form opens on the agent last chosen (B4); an edit opens on the automation's own values (B26). */
export function draftOf(automation: Automation | null, lastChoice: AutomationAgent | undefined): FormDraft {
  if (!automation) return { name: "", task: "", agent: agentDraft(lastChoice), folder: null, worktree: false, schedule: NEW_SCHEDULE };
  return {
    name: automation.name,
    task: automation.task,
    agent: agentDraft(automation.agent),
    folder: automation.folder.default ? null : automation.folder.path,
    worktree: automation.folder.worktree,
    schedule: scheduleDraft(automation.schedule),
  };
}

/** The schedule the fields say, in the shape the core reads (B6). */
export function draftSchedule(schedule: ScheduleDraft): AutomationSchedule {
  switch (schedule.kind) {
    case "daily":
    case "weekdays":
      return { kind: schedule.kind, time: schedule.time };
    case "weekly":
      return { kind: "weekly", days: schedule.days, time: schedule.time };
    case "every_minutes":
      return { kind: "every_minutes", minutes: schedule.minutes };
    case "every_hours":
      return { kind: "every_hours", hours: schedule.hours, time: schedule.time };
    case "cron":
      return { kind: "cron", expr: schedule.expr.trim() };
  }
}

/** The agent as a run starts it: the built-in profile, the agent's own model and effort are sent as none. */
export function draftAgent(agent: FormDraft["agent"]): AutomationAgent {
  return {
    kind: agent.kind,
    ...(agent.profile !== BUILT_IN_PROFILE ? { profile: agent.profile } : {}),
    ...(agent.model ? { model: agent.model } : {}),
    ...(agent.effort ? { effort: agent.effort } : {}),
  };
}

/** The worktree box shows only for a Git project (B5), so a folder that is not one sends none. */
function worktreeOf(draft: FormDraft, git: boolean): boolean {
  return draft.folder !== null && git && draft.worktree;
}

export function addCall(draft: FormDraft, git: boolean, runNow: boolean): Extract<AutomationCall, { call: "add" }> {
  return {
    call: "add",
    name: draft.name.trim(),
    task: draft.task.trim(),
    schedule: draftSchedule(draft.schedule),
    agent: draftAgent(draft.agent),
    ...(draft.folder !== null ? { folder: draft.folder } : {}),
    worktree: worktreeOf(draft, git),
    run_now: runNow,
  };
}

/** An edit sends what changed and leaves the rest as it is (B26); none changed sends nothing. */
export function editCall(automation: Automation, draft: FormDraft, git: boolean): Extract<AutomationCall, { call: "edit" }> | null {
  const before = draftOf(automation, undefined);
  const changes: Extract<AutomationCall, { call: "edit" }> = { call: "edit", id: automation.id };
  if (draft.name.trim() !== automation.name) changes.name = draft.name.trim();
  if (draft.task.trim() !== automation.task) changes.task = draft.task.trim();
  const schedule = draftSchedule(draft.schedule);
  if (JSON.stringify(schedule) !== JSON.stringify(draftSchedule(before.schedule))) changes.schedule = schedule;
  const agent = draftAgent(draft.agent);
  if (JSON.stringify(agent) !== JSON.stringify(draftAgent(before.agent))) changes.agent = agent;
  if (draft.folder !== before.folder) changes.folder = draft.folder;
  // The worktree is read again only when its fields moved: a folder the form no longer lists reads as no Git project, which no edit of the name may turn into a change.
  if (draft.folder !== before.folder || draft.worktree !== before.worktree) {
    const worktree = worktreeOf(draft, git);
    if (worktree !== automation.folder.worktree) changes.worktree = worktree;
  }
  return Object.keys(changes).length > 2 ? changes : null;
}

/** What the form refuses before asking the core: an empty name or task (B7). */
export function emptyField(draft: FormDraft): "automation_name_missing" | "automation_task_missing" | null {
  if (!draft.name.trim()) return "automation_name_missing";
  if (!draft.task.trim()) return "automation_task_missing";
  return null;
}

/** Which field a refusal stands under: the schedule's codes under the schedule, the rest under the field they name, else above the buttons. */
export function refusalField(code: string): "name" | "task" | "agent" | "folder" | "schedule" | "form" {
  if (code.startsWith("schedule_")) return "schedule";
  if (code.startsWith("automation_name_")) return "name";
  if (code.startsWith("automation_task_")) return "task";
  if (code === "automation_agent_unknown") return "agent";
  if (code === "automation_folder_unknown" || code === "automation_worktree_needs_git" || code === "automation_home_unknown") return "folder";
  return "form";
}

const REFUSAL_KEYS = {
  schedule_time_invalid: "automationScreen.refusal.schedule_time_invalid",
  schedule_days_missing: "automationScreen.refusal.schedule_days_missing",
  schedule_minutes_out_of_range: "automationScreen.refusal.schedule_minutes_out_of_range",
  schedule_hours_out_of_range: "automationScreen.refusal.schedule_hours_out_of_range",
  schedule_cron_invalid: "automationScreen.refusal.schedule_cron_invalid",
  schedule_too_often: "automationScreen.refusal.schedule_too_often",
  schedule_never: "automationScreen.refusal.schedule_never",
  automation_name_missing: "automationScreen.refusal.automation_name_missing",
  automation_name_invalid: "automationScreen.refusal.automation_name_invalid",
  automation_name_taken: "automationScreen.refusal.automation_name_taken",
  automation_task_missing: "automationScreen.refusal.automation_task_missing",
  automation_task_too_long: "automationScreen.refusal.automation_task_too_long",
  automation_agent_unknown: "automationScreen.refusal.automation_agent_unknown",
  automation_folder_unknown: "automationScreen.refusal.automation_folder_unknown",
  automation_worktree_needs_git: "automationScreen.refusal.automation_worktree_needs_git",
  automation_home_unknown: "automationScreen.refusal.automation_home_unknown",
  automation_cap: "automationScreen.refusal.automation_cap",
} as const;

/** The words of a refusal; a code the operator can do nothing about here reads as "not now". */
export function refusalKey(code: string): (typeof REFUSAL_KEYS)[keyof typeof REFUSAL_KEYS] | "automationScreen.refusal.other" {
  return Object.hasOwn(REFUSAL_KEYS, code) ? REFUSAL_KEYS[code as keyof typeof REFUSAL_KEYS] : "automationScreen.refusal.other";
}

/**
 * `~/hide/automations/<이름>-…`: the folder the core's preview names for an
 * automation with no folder (B5), under the root the section names, with the
 * home shortened to `~` on every system; any other path is shown whole.
 */
export function shownOwnFolder(root: string | undefined, path: string): string {
  if (!root || !path.startsWith(`${root}/`) || !/[\\/]hide[\\/]automations$/.test(root)) return path;
  return `~/hide/automations${path.slice(root.length)}`;
}
