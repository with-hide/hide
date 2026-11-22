// A call on this core's automations that the window waits for (PRD
// automations B6-B8, B17): the form's preview of a schedule, its make and its
// edit, and a finished run's conversation opened again, whose refusal the run
// view says.
// One `automation_call` event per call, answered by one `automation_result`
// frame under the same request id (`hided/src/automation_screen.rs`); a call
// still waiting when the socket closes is refused `socket_closed`. A press
// whose result the snapshot shows sends the window event instead
// (`actions.automation`).

import type { AutomationAgent, AutomationSchedule } from "./snapshot";
import { type CallAnswer, windowCalls } from "./windowCall";

/** The calls the form makes (`hide_core::automations::Call`). */
export type AutomationCall =
  | { call: "preview"; schedule: AutomationSchedule; name: string }
  | { call: "add"; name: string; task: string; schedule: AutomationSchedule; agent: AutomationAgent; folder?: string; worktree?: boolean; run_now?: boolean; example?: "report" | "briefing" | "ci" }
  | { call: "edit"; id: string; name?: string; task?: string; schedule?: AutomationSchedule; agent?: AutomationAgent; folder?: string | null; worktree?: boolean }
  | { call: "run_open"; id: string; run: number };

/**
 * What a preview answers: the choice the schedule reads as and its next time,
 * in the core's zone, and where an automation of the name with no folder
 * works, `…` where its id comes (null while the core has no home).
 */
export type SchedulePreview = { reads_as: AutomationSchedule; next_run_at_unix_ms: number | null; zone: string | null; own_folder: string | null };
/** What a make answers: the new automation, and the manual run it started (B8), or why that run stopped it (B23). */
export type AutomationMade = { id: string; name: string; folder: string; next_run_at_unix_ms: number | null; run?: number; paused?: string };
export type AutomationEdited = { id: string; next_run_at_unix_ms: number | null };

/** What an opening answers: the run whose conversation opens, or opened already. */
export type RunOpening = { id: string; run: number };

type Answers = { preview: SchedulePreview; add: AutomationMade; edit: AutomationEdited; run_open: RunOpening };

export type { CallAnswer } from "./windowCall";

const calls = windowCalls("automation_call", "automations_unavailable");

/** Sets where calls go and returns where they went before, so a window that answers them itself (the practice window) can put the old one back. */
export const configureAutomationCalls = calls.configure;

export function callAutomations<C extends AutomationCall>(call: C): Promise<CallAnswer<Answers[C["call"]]>> {
  return calls.call<Answers[C["call"]]>(call);
}

/** An `automation_result` frame; one no call waits for (its window reloaded) is dropped. */
export const receiveAutomationResult = calls.receive;

/** The socket closed: no answer comes for a call still waiting. */
export const refuseWaitingAutomationCalls = calls.refuseWaiting;
