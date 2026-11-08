// The words for a question the core asks on a row for the agent
// (`AskSubject`, hide-plan B14). The core sends what is asked, never the
// sentence, so the band, the row, the bell and the phone say it in the
// interface language.

import type { TFunction } from "i18next";
import { attentionReasonText } from "./missions";
import { humanTurnText } from "./humanTurn";
import type { AskSubject } from "./snapshot";

export { humanTurnText };

/** What is asked: a plan review's words, a mission decision's own line, a mission node waiting on the operator (`<node> · <why>`), a brief or letter the agent has not confirmed (`input_unconfirmed`), or a person's turn, `인증 · github.com/login` (PRD ask-human B9), whose device a row draws as its chip. */
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
    case "human_turn":
      return humanTurnText(t, subject);
  }
}

/** What a notification says a subject asks: the line's words, and for a person's turn the device it waits on, which a row draws as a chip (B13). */
export function askSubjectNotice(t: TFunction<"translation">, subject: AskSubject): string {
  const text = askSubjectText(t, subject);
  return subject.cause === "human_turn" && subject.device ? `${text} · ${subject.device}` : text;
}

/** The word a notification about a subject leads with (`판단`, needs-you-cleanup D-01; `확인 필요`, PRD mission-node-simple B14; `사람 차례`, PRD ask-human B13), or null to keep the status word; a row's line names no verb. */
export function askSubjectLead(t: TFunction<"translation">, subject: AskSubject | null | undefined): string | null {
  if (subject?.cause === "mission_decision") return t("mission.askDecision");
  if (subject?.cause === "mission_attention") return t("mission.attention");
  if (subject?.cause === "human_turn") return t("agents.human.mark");
  return null;
}
