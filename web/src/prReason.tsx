import type { HTMLAttributes, ReactNode } from "react";
import { useElapsed } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";
import type { PrGroupId, PrReason } from "./agentScope";
import { GROUP_TONE, reasonText, splitReason } from "./prReasonText";

export { GROUP_TONE };
import { cn } from "./lib/utils";

// A pull request's reason words (PRD work-tab-pr-lens D-16, B24): the second
// line of a list row and the reason box of the card say the same thing in
// the same colour. The core decides the kind and the values
// (`herdr-core/src/agent_state/work/board.rs`); this only words them, with
// the times counted on the window's one clock.

/**
 * One reason as a line: the leading word in the text colour, the rest in the
 * group's tone. `noAgent` appends `맡은 에이전트 없음` for a blocked row (B10).
 */
export function ReasonLine({ reason, group, noAgent = false, approvals = null, className, ...rest }: { reason: PrReason; group: PrGroupId; noAgent?: boolean; approvals?: ReactNode; className?: string } & HTMLAttributes<HTMLSpanElement>) {
  const { t } = useInterfaceTranslation();
  const time = useElapsed(reason.since_unix_ms);
  const [lead, tail] = splitReason(reasonText(t, reason, time));
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
