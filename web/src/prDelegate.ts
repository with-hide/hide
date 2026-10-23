// Handing a pull request to an agent (PRD overview-lenses-prs D-12, D-46):
// the first prompt the dialog opens with, from what the core read of the pull
// request. Pure, so the rule reads and tests on its own; the operator edits
// the prompt before starting, and it is never cut (PRD Risks).

import type { TFunction } from "i18next";
import type { PrFeedback } from "./snapshot";

/**
 * Which pull request and branch to fix, then each failed check by name with
 * its link, then each change request as its reviewer wrote it. A section with
 * nothing in it is left out.
 */
export function delegatePrompt(pr: { number: number; title: string; branch: string }, feedback: Pick<PrFeedback, "failed_checks" | "change_requests">, t: TFunction<"translation">): string {
  const lines: string[] = [t("prWork.prompt.fix", { number: String(pr.number), branch: pr.branch, title: pr.title })];
  if (feedback.failed_checks.length > 0) {
    lines.push("", t("prWork.prompt.failedChecks"));
    for (const check of feedback.failed_checks) lines.push(`- ${check.name ?? t("githubReason.unnamedCheck")}${check.url ? ` ${check.url}` : ""}`);
  }
  const requests = feedback.change_requests.filter((request) => request.body.trim());
  if (requests.length > 0) {
    lines.push("", t("prWork.prompt.changeRequests"));
    for (const request of requests) lines.push(`- ${request.author ?? t("prWork.prompt.reviewer")}: ${request.body.trim()}`);
  }
  return lines.join("\n");
}
