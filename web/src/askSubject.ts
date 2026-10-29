// The words for a question the core asks on a row for the agent
// (`AskSubject`, hide-plan B14). The core sends what is asked, never the
// sentence, so the band, the row, the bell and the phone say it in the
// interface language.

import type { TFunction } from "i18next";
import type { AskSubject } from "./snapshot";

/** What is asked: a plan review's words, or a mission decision's own line. */
export function askSubjectText(t: TFunction<"translation">, subject: AskSubject): string {
  switch (subject.cause) {
    case "plan_review":
      return t("plan.askReview", { count: subject.steps });
    case "mission_decision":
      return subject.line;
  }
}

/** The word a subject's line leads with in place of the row's own (`판단 · <line>`, needs-you-cleanup D-01), or null to keep the row's. */
export function askSubjectLead(t: TFunction<"translation">, subject: AskSubject | null | undefined): string | null {
  return subject?.cause === "mission_decision" ? t("mission.askDecision") : null;
}
