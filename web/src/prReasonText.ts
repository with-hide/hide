// A pull request's reason words (PRD work-tab-pr-lens D-16, B24), apart from
// the component that draws them so the phone page can use them without the
// desktop's shell. The core decides the kind and the values
// (`herdr-core/src/agent_state/work/board.rs`); this only words them.

import type { TFunction } from "i18next";
import type { PrGroupId, PrReason } from "./agentScope";

/** The tone a group's reason is drawn in; the leading word stays in the text colour. */
export const GROUP_TONE: Record<PrGroupId, string> = {
  needs_you: "text-warning",
  working: "text-agent-working",
  blocked: "text-pr-failed",
  waiting: "text-subtle-foreground",
  drafts: "text-subtle-foreground",
  bots: "text-subtle-foreground",
  merged: "text-pr-merged",
};

/** The reason as text, with `time` already counted. */
export function reasonText(t: TFunction<"translation">, reason: PrReason, time: string | null): string {
  const when = time ?? "-";
  switch (reason.kind) {
    case "merge":
      return reason.text ? t("work.reason.mergeFailing", { name: reason.text }) : t("work.reason.merge");
    case "update":
      return t("work.reason.update", { base: reason.base ?? "" });
    case "assign_reviewer":
      return t("work.reason.assign_reviewer");
    case "assign_reviewer_optional":
      return t("work.reason.assign_reviewer_optional");
    case "review":
      return reason.name ? t("work.reason.review", { name: reason.name, time: when }) : t("work.reason.reviewAnon", { time: when });
    case "team_review":
      return reason.name ? t("work.reason.team_review", { name: reason.name, time: when }) : t("work.reason.team_reviewAnon", { time: when });
    case "reply":
      return reason.text ? t("work.reason.reply", { name: reason.name ?? "", text: reason.text }) : t("work.reason.replyAnon", { name: reason.name ?? "" });
    case "check":
      return t("work.reason.check", { name: reason.name ?? "", time: when });
    case "working":
      return t("work.reason.working", { name: reason.name ?? "", time: when });
    case "conflict":
      return t("work.reason.conflict", { base: reason.base ?? "" });
    case "failed":
      if (!reason.name) return t("work.reason.failedUnnamed");
      return (reason.count ?? 1) > 1 ? t("work.reason.failedMore", { name: reason.name, count: (reason.count ?? 1) - 1 }) : t("work.reason.failed", { name: reason.name });
    case "changes":
      return t("work.reason.changes");
    case "ci":
      return t("work.reason.ci", { count: reason.count ?? 1 });
    case "waiting_review":
      return reason.name ? t("work.reason.waiting_review", { name: reason.name, time: when }) : t("work.reason.waiting_reviewAnon", { time: when });
    case "draft":
      return t("work.reason.draft", { time: when });
    case "queued":
      return t("work.reason.queued", { count: reason.count ?? 1 });
    case "merged":
      return t("work.reason.merged", { time: when });
    case "checking":
      return t("work.reason.checking");
    case "unjudged":
      switch (reason.text) {
        case "account":
          return t("work.reason.unjudged.account");
        case "review_requests":
          return t("work.reason.unjudged.review_requests");
        case "merge_queue":
          return t("work.reason.unjudged.merge_queue");
        default:
          throw new Error(`Core PR reason unjudged names no known question: ${reason.text}`);
      }
  }
}

/** The leading word of a reason and the rest, split at the first ` · `. */
export function splitReason(text: string): [string, string] {
  const at = text.indexOf(" · ");
  return at < 0 ? ["", text] : [text.slice(0, at), text.slice(at)];
}
