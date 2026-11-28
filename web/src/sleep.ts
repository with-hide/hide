// What a sleeping agent's pane says (PRD agent-sleep B10-B14). The core owns
// the state and its plain reason; this only words the header caption and the
// age, so every rule is tested without a browser.

import type { TFunction } from "i18next";
import { activityAge, ageText } from "./projects";
import type { Actions } from "./actions";
import type { AgentSleep, SleepingSession } from "./snapshot";
import { startReasonText } from "./startReason";

/** The pane header's caption beside the moon; a failed wake has no moon. */
export function sleepCaption(sleep: AgentSleep, nowMs: number, t: TFunction<"translation">): { text: string; moon: boolean } {
  if (sleep.state === "waking") return { text: t("panes.sleep.captionWaking"), moon: true };
  if (sleep.state === "failed") return { text: t("panes.sleep.captionFailed"), moon: false };
  const age = activityAge(sleep.since_unix_ms, nowMs);
  return { text: age && age.unit !== "now" ? t("panes.sleep.captionAge", { age: ageText(age, t) }) : t("panes.sleep.captionSleeping"), moon: true };
}

/** The line under Waking…: how old the conversation being resumed is. */
export function wakingLine(sleep: AgentSleep, nowMs: number, t: TFunction<"translation">): string {
  const age = activityAge(sleep.since_unix_ms, nowMs);
  return age && age.unit !== "now" ? t("panes.sleep.resumingAge", { age: ageText(age, t) }) : t("panes.sleep.resuming");
}

/** Why a wake failed, under its heading: the core's reason worded as every start reason is, or null when it gave none. */
export function wakeFailureReason(sleep: AgentSleep, t: TFunction<"translation">): string | null {
  return sleep.reason ? startReasonText(sleep.reason, t) : null;
}

/** The phases of a session whose sleep closed its pane, by the state its card draws (`SleepingSession.phase`). */
const SESSION_WAKING = new Set<SleepingSession["phase"]>(["saving_wake", "creating", "saving_start", "starting"]);

/** A session whose sleep closed its pane (Pi, OpenCode, omp), in the words of a pane that sleeps: its phase as a sleep state. */
export function sessionSleep(session: Pick<SleepingSession, "phase" | "since_unix_ms" | "reason">): AgentSleep {
  if (session.phase === "failed") return { state: "failed", since_unix_ms: session.since_unix_ms, reason: session.reason };
  return { state: SESSION_WAKING.has(session.phase) ? "waking" : "sleeping", since_unix_ms: session.since_unix_ms };
}

/** What a sleeping session draws, and what its card's presses do; null for a press its state does not offer. */
export type SleepView = { sleep: AgentSleep; onWake: (() => void) | null; onFresh: (() => void) | null };

/**
 * Whether a session sleeps and what its card offers, decided once for the
 * pane's own body, its conversation and the session beside Desk: the pane's
 * row while it sleeps in its pane (wakes there; a failed wake also starts a
 * new session), the sleeping session while its sleep closed its pane (wakes
 * in a new pane when the core offers it), or a Desk card that reads asleep
 * before either says more (nothing to press yet). Null while it is awake.
 */
export function conversationSleep(
  paneId: string,
  from: { row: { sleep?: AgentSleep | null } | null; job?: { state: string } | null; session?: SleepingSession | null },
  actions: Pick<Actions, "wakeAgent" | "wakeSleepingSession">,
  onSessionWake?: () => void,
): SleepView | null {
  const inPane = from.row?.sleep ?? null;
  if (inPane) return { sleep: inPane, onWake: inPane.state === "waking" ? null : () => actions.wakeAgent(paneId), onFresh: inPane.state === "failed" ? () => actions.wakeAgent(paneId, true) : null };
  const session = from.row ? null : from.session ?? null;
  if (session) {
    const sleep = sessionSleep(session);
    const wake = session.wake_available && sleep.state !== "waking" ? () => {
      actions.wakeSleepingSession(session.sleep_id);
      onSessionWake?.();
    } : null;
    return { sleep, onWake: wake, onFresh: null };
  }
  if (from.job?.state === "sleeping") return { sleep: { state: "sleeping", since_unix_ms: 0 }, onWake: null, onFresh: null };
  return null;
}
