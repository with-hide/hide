import { CheckCircle2Icon, CircleDotIcon, TriangleAlertIcon } from "lucide-react";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { useElapsed } from "./components/elapsed";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { approvalCount, initials, reviewerMarks, type PrReview } from "./prReview";
import type { PrReviewer, PrReviewerState } from "./snapshot";

// A pull request's reviewers as the PR list row and the PR card draw them
// (PRD pr-review-status D-02, D-03, D-09): a two-letter circle with a state
// dot, `n/m` in GitHub's colour, and on the card one line per reviewer.

type T = ReturnType<typeof useInterfaceTranslation>["t"];

/** The dot's look per state: filled green, filled red, a hollow yellow ring; the rest have none (B2). */
const DOT: Partial<Record<PrReviewerState, string>> = {
  approved: "bg-pr-mergeable",
  changes_requested: "bg-pr-failed",
  requested: "border border-pr-pending bg-background",
};

/** The colour of a state's word on the card. */
const WORD: Record<PrReviewerState, string> = {
  approved: "text-pr-mergeable",
  changes_requested: "text-pr-failed",
  requested: "text-pr-pending",
  commented: "text-muted-foreground",
  dismissed: "text-muted-foreground",
};

function reviewerName(reviewer: PrReviewer, t: T): string {
  return reviewer.team ? t("work.review.team", { name: reviewer.login }) : reviewer.login;
}

/** Every reviewer and their state, for a row's tooltip (B2). */
function reviewersText(review: PrReview, t: T): string {
  const lines = review.reviewers.map((reviewer) => `${reviewerName(reviewer, t)} · ${t(`work.review.state.${reviewer.state}`)}`);
  if (review.more > 0) lines.push(t("work.review.more", { count: review.more }));
  return lines.join(", ");
}

/** A reviewer's circle: the login's two letters, and the state's dot at its corner. */
export function ReviewerMark({ reviewer, className }: { reviewer: PrReviewer; className?: string }) {
  const dot = DOT[reviewer.state];
  return (
    <span className={cn("relative inline-flex size-(--size-icon-lg) shrink-0 items-center justify-center rounded-full bg-secondary text-micro leading-none font-medium tracking-tight text-secondary-foreground", reviewer.state === "dismissed" && "opacity-(--opacity-dimmed)", className)} data-reviewer-mark={reviewer.state} aria-hidden="true">
      {initials(reviewer.login)}
      {dot ? <span className={cn("absolute -right-xxs -bottom-xxs size-(--size-flow-bar) rounded-full ring-1 ring-background", dot)} /> : null}
    </span>
  );
}

/** A list row's reviewer marks, at most three then `+N`, with every reviewer in the tooltip (B2, B17). */
export function ReviewerMarks({ review }: { review: PrReview }) {
  const { t } = useInterfaceTranslation();
  const { shown, more } = reviewerMarks(review);
  if (shown.length === 0 && more === 0) return null;
  const text = reviewersText(review, t);
  return (
    <Hint label={text}>
      <span role="img" aria-label={`${t("work.review.marks")}: ${text}`} className="pointer-events-auto relative flex shrink-0 items-center gap-xxs" data-pr-reviewers={review.reviewers.length}>
        {shown.map((reviewer) => <ReviewerMark key={`${reviewer.team}:${reviewer.login}`} reviewer={reviewer} />)}
        {more > 0 ? <span className="font-mono text-caption text-muted-foreground">+{more}</span> : null}
      </span>
    </Hint>
  );
}

/** `승인 n/m` in GitHub's colour (B3), or nothing when the count is unknown (B11). */
export function ApprovalCount({ review, className }: { review: PrReview; className?: string }) {
  const { t } = useInterfaceTranslation();
  const count = approvalCount(review);
  if (!count) return null;
  return <span className={cn(count.approved ? "text-pr-mergeable" : "text-pr-pending", className)} data-pr-approvals={`${count.approvals}/${count.required}`} data-pr-approvals-approved={count.approved ? "true" : undefined}>{t("work.review.count", count)}</span>;
}

/** One reviewer's line on the card: who, the state's word, and when (B5, B6). */
function ReviewerLine({ reviewer }: { reviewer: PrReviewer }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(reviewer.at_unix_ms);
  const when = elapsed === null ? null : t(reviewer.state === "requested" ? "work.review.waiting" : "work.review.ago", { time: elapsed });
  return (
    <li className={cn("flex min-w-0 items-center gap-xs text-micro", reviewer.state === "dismissed" && "opacity-(--opacity-dimmed)")} data-pr-card-reviewer={reviewer.login} data-pr-card-reviewer-state={reviewer.state}>
      <ReviewerMark reviewer={reviewer} />
      <span className="min-w-0 truncate text-foreground">{reviewerName(reviewer, t)}</span>
      <span className={cn("shrink-0", WORD[reviewer.state])}>{t(`work.review.state.${reviewer.state}`)}</span>
      {when ? <span className="min-w-0 shrink truncate text-muted-foreground">· {when}</span> : null}
    </li>
  );
}

/**
 * The card's review block, the CI block's twin (B5, B6, B9, B12): `리뷰 ·
 * 승인 n/m` (or `리뷰` and GitHub's word while the count is unknown), one
 * line per reviewer, `요청한 사람 없음` when review is required and nobody
 * is in it, and the pre-check as its secondary button.
 */
export function ReviewBlock({ review, onPreCheck }: { review: PrReview; onPreCheck: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const tone = review.decision === "approved" ? "text-success" : review.decision === "changes_requested" ? "text-destructive" : review.decision === "review_required" ? "text-pr-pending" : "text-foreground";
  const Icon = review.decision === "approved" ? CheckCircle2Icon : review.decision === "changes_requested" ? TriangleAlertIcon : CircleDotIcon;
  const decisionWord = review.decision === "approved" ? "board.review.approved" : review.decision === "changes_requested" ? "board.review.changes" : review.decision === "review_required" ? "board.review.required" : null;
  const count = approvalCount(review);
  return (
    <div className="min-w-0 flex-1" data-pr-card-review="true" data-pr-card-review-decision={review.decision ?? "none"}>
      <p className={cn("flex items-start gap-xs text-caption font-medium", tone)}>
        <Icon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0" />
        <span className="min-w-0">
          {t("work.review.title")}
          {count ? <> · <ApprovalCount review={review} /></> : decisionWord ? ` · ${t(decisionWord)}` : null}
        </span>
      </p>
      {review.reviewers.length > 0 ? (
        <ul className="mt-xs flex min-w-0 flex-col gap-xxs" aria-label={t("work.review.marks")} data-pr-card-reviewers={review.reviewers.length}>
          {review.reviewers.map((reviewer) => <ReviewerLine key={`${reviewer.team}:${reviewer.login}`} reviewer={reviewer} />)}
          {review.more > 0 ? <li className="text-micro text-muted-foreground">{t("work.review.more", { count: review.more })}</li> : null}
        </ul>
      ) : null}
      {review.nobodyAsked ? <p className="mt-xs text-micro text-pr-pending" data-pr-card-nobody-asked="true">{t("work.review.nobodyAsked")}</p> : null}
      {onPreCheck ? <Button variant="ghost" size="sm" className="mt-xs -ml-sm" onClick={onPreCheck} data-pr-card-pre-check="true">{t("work.review.preCheck")}</Button> : null}
    </div>
  );
}
