// What a sleeping agent's pane says (PRD agent-sleep B10-B14). The core owns
// the state and its plain reason; this only words the header caption and the
// age, so every rule is tested without a browser.

import type { TFunction } from "i18next";
import { activityAge, ageText } from "./projects";
import type { AgentSleep } from "./snapshot";
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
