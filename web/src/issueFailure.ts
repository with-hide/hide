import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { issueReasonsEnglish } from "./i18n/resources/issueReasons";
import type { CodedFailure } from "./snapshot";

/**
 * Why a Local issue or a pull request's issue failed, in the operator's language, or null when there is no failure or
 * this build does not word its code: the headline then speaks alone and the code stays in `data-` and the log.
 */
export function issueFailureText(failure: CodedFailure | null | undefined, t: TFunction<"translation">): string | null {
  const key = `issueReason.${failure?.code}`;
  if (!failure || !(key in issueReasonsEnglish)) return null;
  return t(key as MessageKey, { limit: failure.limit });
}

/**
 * Why an issue's body was not read, for the Start dialog's note: a Local issue's code with its reason, otherwise the
 * GitHub read's general line (`gh`'s own words stay in the core's `issue_detail.failed` diagnostic).
 */
export function issueBodyFailure(failure: string | null, t: TFunction<"translation">): string {
  const reason = issueFailureText(failure ? { code: failure } : null, t);
  return reason ? t("issue.bodyReadFailed", { message: reason }) : t("githubReason.issueBodyReadFailed");
}
