// The review a list row and a card draw (PRD pr-review-status B2-B4, B8-B11,
// B15, B17): nothing for a settled, draft, unread or unstarted review, the
// three marked states first, and `n/m` only from a count the core read.

import { describe, expect, it } from "vitest";
import { approvalCount, initials, prReview, reviewerMarks, reviewWhy } from "./prReview";
import type { PrReviewer, PullRequest } from "./snapshot";

const reviewer = (login: string, state: PrReviewer["state"]): PrReviewer => ({ login, team: false, state, at_unix_ms: 1 });

const pr = (over: Omit<Partial<PullRequest>, "work"> & { work?: Record<string, unknown> }): PullRequest =>
  ({ number: 7, state: "pending", is_draft: false, review: "review_required", ...over, work: { merge_state: "blocked", reviewers: [], reviewers_more: 0, required_approvals: 2, ...over.work } }) as unknown as PullRequest;

describe("prReview", () => {
  it("draws nothing for a settled, draft or unread pull request, or one review does not concern", () => {
    expect(prReview(pr({ state: "merged" }))).toBeNull();
    expect(prReview(pr({ state: "closed" }))).toBeNull();
    expect(prReview(pr({ is_draft: true, state: "draft" }))).toBeNull();
    expect(prReview(pr({ work: { reviewers: null } }))).toBeNull();
    expect(prReview(pr({ review: null, work: { reviewers: [], required_approvals: null } }))).toBeNull();
  });

  it("counts approvals only, and says nobody was asked when review is required and nobody is in it", () => {
    const review = prReview(pr({ work: { reviewers: [reviewer("kim", "approved"), reviewer("lee", "requested"), reviewer("choi", "commented"), reviewer("park", "dismissed")] } }))!;
    expect(review.approvals).toBe(1);
    expect(review.nobodyAsked).toBe(false);
    expect(prReview(pr({}))!.nobodyAsked).toBe(true);
  });

  it("shows started review on a repository that does not require it, without a count", () => {
    const review = prReview(pr({ review: null, work: { reviewers: [reviewer("kim", "commented")], required_approvals: null } }))!;
    expect(review.nobodyAsked).toBe(false);
    expect(approvalCount(review)).toBeNull();
  });
});

describe("approvalCount", () => {
  it("is drawn only from a count the core read, green only when GitHub approved", () => {
    expect(approvalCount(prReview(pr({ work: { reviewers: [reviewer("kim", "approved")] } }))!)).toEqual({ approvals: 1, required: 2, approved: false });
    expect(approvalCount(prReview(pr({ review: "approved", work: { reviewers: [reviewer("kim", "approved")], required_approvals: 1 } }))!)).toEqual({ approvals: 1, required: 1, approved: true });
    expect(approvalCount(prReview(pr({ work: { reviewers: [reviewer("kim", "approved")], required_approvals: null } }))!)).toBeNull();
  });
});

describe("reviewerMarks", () => {
  it("marks approvals, change requests and asks, three of them, and counts the rest behind +N", () => {
    const reviewers = [reviewer("a", "changes_requested"), reviewer("b", "approved"), reviewer("c", "approved"), reviewer("d", "requested"), reviewer("e", "commented")];
    const marks = reviewerMarks(prReview(pr({ work: { reviewers, reviewers_more: 2 } }))!);
    expect(marks.shown.map((mark) => mark.login)).toEqual(["a", "b", "c"]);
    expect(marks.more).toBe(3);
  });
});

describe("reviewWhy", () => {
  it("is null unless GitHub still asks for review", () => {
    expect(reviewWhy(pr({ review: "approved" }))).toBeNull();
    expect(reviewWhy(pr({ review: null }))).toBeNull();
  });
});

describe("initials", () => {
  it("takes a login's first two letters, without a team's organisation or a leading mark", () => {
    expect(initials("kim")).toBe("KI");
    expect(initials("acme/core")).toBe("CO");
    expect(initials("_x")).toBe("X");
    expect(initials("이호연")).toBe("이호");
  });
});
