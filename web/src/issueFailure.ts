import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { issueReasonsEnglish } from "./i18n/resources/issueReasons";
import type { IssueFailure } from "./snapshot";

/**
 * Why a Local issue or a pull request's issue failed, in the operator's language, or null when there is no failure or
 * this build does not word its code: the headline then speaks alone and the code stays in `data-` and the log.
 */
export function issueFailureText(failure: IssueFailure | null | undefined, t: TFunction<"translation">): string | null {
  const key = `issueReason.${failure?.code}`;
  if (!failure || !(key in issueReasonsEnglish)) return null;
  return t(key as MessageKey, { limit: failure.limit });
}
