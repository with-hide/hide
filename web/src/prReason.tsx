import type { HTMLAttributes, ReactNode } from "react";
import { useElapsed } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";
import type { PrGroupId, PrReason } from "./agentScope";
import { cn } from "./lib/utils";

// A pull request's reason words (PRD work-tab-pr-lens D-16, B24): the second
// line of a list row and the reason box of the card say the same thing in
// the same colour. The core decides the kind and the values
// (`herdr-core/src/agent_state/work/board.rs`); this only words them, with
// the times counted on the window's one clock.

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
export function reasonText(t: ReturnType<typeof useInterfaceTranslation>["t"], reason: PrReason, time: string | null): string {
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
function split(text: string): [string, string] {
  const at = text.indexOf(" · ");
  return at < 0 ? ["", text] : [text.slice(0, at), text.slice(at)];
}

/**
 * One reason as a line: the leading word in the text colour, the rest in the
 * group's tone. `noAgent` appends `맡은 에이전트 없음` for a blocked row (B10).
 */
export function ReasonLine({ reason, group, noAgent = false, approvals = null, className, ...rest }: { reason: PrReason; group: PrGroupId; noAgent?: boolean; approvals?: ReactNode; className?: string } & HTMLAttributes<HTMLSpanElement>) {
  const { t } = useInterfaceTranslation();
  const time = useElapsed(reason.since_unix_ms);
  const [lead, tail] = split(reasonText(t, reason, time));
  const tone = reason.kind === "checking" || reason.kind === "unjudged" ? "text-muted-foreground" : GROUP_TONE[group];
  // `승인 n/m` follows the leading word of a review wait, whose rest is who
  // and how long, and closes every other reason (PRD pr-review-status B3).
  const early = approvals !== null && lead !== "" && reason.kind === "waiting_review";
  return (
    <span className={cn("min-w-0 truncate text-micro", className)} data-pr-reason={reason.kind} {...rest}>
      {lead ? <span className="font-medium text-foreground">{lead}</span> : null}
      {early ? <> · {approvals}</> : null}
      <span className={tone}>{tail}</span>
      {approvals !== null && !early ? <> · {approvals}</> : null}
      {noAgent ? <span className="text-muted-foreground"> · {t("work.noAgent")}</span> : null}
    </span>
  );
}
