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
import type { DispatchFn } from "./ws";

/** The calls the form makes (`hide_core::automations::Call`). */
export type AutomationCall =
  | { call: "preview"; schedule: AutomationSchedule; name: string }
  | { call: "add"; name: string; task: string; schedule: AutomationSchedule; agent: AutomationAgent; folder?: string; worktree?: boolean; run_now?: boolean }
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

/** The core's answer, or the code it refused the call with (`schedule_too_often`, `automation_name_taken`, `socket_closed`, ...). */
export type CallAnswer<T> = { ok: true; value: T } | { ok: false; code: string };

let dispatchFn: DispatchFn | null = null;
const pending = new Map<string, (answer: CallAnswer<unknown>) => void>();
let nextRequest = 1;

/** Sets where calls go and returns where they went before, so a window that answers them itself (the practice window) can put the old one back. */
export function configureAutomationCalls(dispatch: DispatchFn | null): DispatchFn | null {
  const before = dispatchFn;
  dispatchFn = dispatch;
  return before;
}

export function callAutomations<C extends AutomationCall>(call: C): Promise<CallAnswer<Answers[C["call"]]>> {
  return new Promise((resolve) => {
    const request_id = `automation-${nextRequest}`;
    nextRequest += 1;
    if (!dispatchFn) return resolve({ ok: false, code: "socket_closed" });
    pending.set(request_id, resolve as (answer: CallAnswer<unknown>) => void);
    if (dispatchFn({ schema_version: 2, kind: "automation_call", payload: { request_id, call } }) === false) {
      pending.delete(request_id);
      resolve({ ok: false, code: "socket_closed" });
    }
  });
}

/** An `automation_result` frame; one no call waits for (its window reloaded) is dropped. */
export function receiveAutomationResult(payload: { request_id: string; ok: boolean; value?: unknown; code?: string }): void {
  const resolve = pending.get(payload.request_id);
  if (!resolve) return;
  pending.delete(payload.request_id);
  resolve(payload.ok ? { ok: true, value: payload.value } : { ok: false, code: payload.code ?? "automations_unavailable" });
}

/** The socket closed: no answer comes for a call still waiting. */
export function refuseWaitingAutomationCalls(): void {
  for (const [requestId, resolve] of pending) {
    pending.delete(requestId);
    resolve({ ok: false, code: "socket_closed" });
  }
}
