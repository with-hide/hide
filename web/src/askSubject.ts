// The words for a question the core asks on a row for the agent
// (`AskSubject`, hide-plan B14). The core sends what is asked, never the
// sentence, so the band, the row, the bell and the phone say it in the
// interface language.

import type { TFunction } from "i18next";
import type { AskSubject } from "./snapshot";

export function askSubjectText(t: TFunction<"translation">, subject: AskSubject): string {
  switch (subject.cause) {
    case "plan_review":
      return t("plan.askReview", { count: subject.steps });
  }
}
