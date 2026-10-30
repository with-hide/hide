// A pull request's review as the PR list and the PR card draw it (PRD
// pr-review-status): who reviewed, who is still asked, and how many approvals
// are still missing. The core read each reviewer and the base's required
// count (`PullRequestWork.reviewers`, `required_approvals`); GitHub's own
// review decision says whether review still blocks. Nothing here guesses a
// number the core did not read: an unknown reviewer list draws nothing, and
// an unknown count draws no `n/m`.

import type { PrReviewer, PrReviewerState, PullRequest } from "./snapshot";

/** The reviewer marks a list row shows before `+N` (B2). */
export const REVIEWER_MARKS = 3;

/** The states a list row draws a mark for: the rest are in its tooltip only (B2). */
const MARKED: readonly PrReviewerState[] = ["approved", "changes_requested", "requested"];

export type PrReview = {
  reviewers: PrReviewer[];
  /** Reviewers past the core's cap. */
  more: number;
  /** Reviewers whose latest review is an approval (B8). */
  approvals: number;
  /** The approvals the base requires; null when unknown, never guessed (D-05). */
  required: number | null;
  /** GitHub's review decision: review decides the merge at all. */
  decision: PullRequest["review"];
  /** Review is required and nobody was asked or reviewed (B9). */
  nobodyAsked: boolean;
};

/**
 * The review a pull request shows, or null when nothing about review is
 * drawn: settled and draft pull requests (D-13), reviewers the core could not
 * read (B15), and review that is neither required nor started (B4, B10).
 */
export function prReview(pr: PullRequest): PrReview | null {
  const work = pr.work;
  if (!work || pr.is_draft || pr.state === "merged" || pr.state === "closed" || pr.state === "draft") return null;
  const reviewers = work.reviewers;
  if (!reviewers) return null;
  const decision = pr.review ?? null;
  if (decision === null && reviewers.length === 0) return null;
  return {
    reviewers,
    more: work.reviewers_more ?? 0,
    approvals: reviewers.filter((reviewer) => reviewer.state === "approved").length,
    required: decision !== null ? work.required_approvals ?? null : null,
    decision,
    nobodyAsked: decision !== null && reviewers.length === 0,
  };
}

/** The marks a row draws and how many more stand behind `+N` (B2, B17). */
export function reviewerMarks(review: PrReview): { shown: PrReviewer[]; more: number } {
  const marked = review.reviewers.filter((reviewer) => MARKED.includes(reviewer.state));
  return { shown: marked.slice(0, REVIEWER_MARKS), more: Math.max(0, marked.length - REVIEWER_MARKS) + review.more };
}

/** `n/m` when the count is known (B3), with GitHub's word for its colour: green only when GitHub says approved (D-08). */
export function approvalCount(review: PrReview): { approvals: number; required: number; approved: boolean } | null {
  if (review.required === null) return null;
  return { approvals: review.approvals, required: review.required, approved: review.decision === "approved" };
}

/** A reviewer's mark letters: a login's first two, without a team's organisation (D-09). */
export function initials(login: string): string {
  const name = login.split("/").at(-1) ?? login;
  return Array.from(name.replace(/^[^\p{L}\p{N}]+/u, "")).slice(0, 2).join("").toUpperCase() || "?";
}

/**
 * Why review stops a merge, or null when it does not (B7): the approvals
 * still missing when the count is known, GitHub asking for more when the
 * count is met, and review required when the count is unknown.
 */
export type ReviewWhy = { key: "approvals"; count: number } | { key: "github_more" } | { key: "required" };

export function reviewWhy(pr: PullRequest): ReviewWhy | null {
  if (pr.review !== "review_required") return null;
  const review = prReview(pr);
  if (!review || review.required === null) return { key: "required" };
  const missing = review.required - review.approvals;
  return missing > 0 ? { key: "approvals", count: missing } : { key: "github_more" };
}
