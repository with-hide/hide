// The words for a question the core asks on a row for the agent
// (`AskSubject`, hide-plan B14). The core sends what is asked, never the
// sentence, so the band, the row, the bell and the phone say it in the
// interface language.

import type { TFunction } from "i18next";
import { attentionReasonText } from "./missions";
import type { AskSubject } from "./snapshot";

/** What is asked: a plan review's words, a mission decision's own line, a mission node waiting on the operator (`<node> · <why>`), or a brief or letter the agent has not confirmed (`input_unconfirmed`). */
export function askSubjectText(t: TFunction<"translation">, subject: AskSubject): string {
  switch (subject.cause) {
    case "plan_review":
      return t("plan.askReview", { count: subject.steps });
    case "mission_decision":
      return subject.line;
    case "mission_attention":
      return `${subject.node} · ${attentionReasonText(subject.reason, null, t)}`;
    case "input_unconfirmed": {
      // Not received only when the composer was read still holding it; else not confirmed.
      const what = t(`agents.input.${subject.input}${subject.held ? "Held" : ""}`);
      return subject.spent ? t("agents.input.spent", { what }) : what;
    }
  }
}

/** The word a notification about a subject leads with (`판단`, needs-you-cleanup D-01; `확인 필요`, PRD mission-node-simple B14), or null to keep the status word; a row's line names no verb. */
export function askSubjectLead(t: TFunction<"translation">, subject: AskSubject | null | undefined): string | null {
  if (subject?.cause === "mission_decision") return t("mission.askDecision");
  if (subject?.cause === "mission_attention") return t("mission.attention");
  return null;
}
