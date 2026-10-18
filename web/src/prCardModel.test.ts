// The card's rules that are not drawing (PRD work-tab-pr-lens D-06, D-21,
// D-27; B27, B28, B30; PRD pr-card-and-session-record-redesign B4-B6): the
// outline of a body, which button leads for a case, why Merge cannot be
// pressed, where a changed file opens, and the timeline's milestones and
// their order among the sessions.

import { describe, expect, it } from "vitest";
import { checkSummary, confirmHolds, fileOpen, mergeWhy, milestones, prFacts, primaryAction, splitFiles, timeline } from "./prCardModel";
import type { PrCard, PullRequest } from "./snapshot";

const pr = (over: Omit<Partial<PullRequest>, "work"> & { work?: Record<string, unknown> }): PullRequest =>
  ({ number: 7, url: "https://github.com/acme/repo/pull/7", head_branch: "feat/x", base_branch: "main", is_draft: false, ...over, work: { merge_state: "clean", pending_checks: 0, queue: { state: "not_queued" }, ...over.work } }) as unknown as PullRequest;

const card = (merge: Partial<NonNullable<PrCard["merge"]>> | null): PrCard => ({ merge: merge === null ? null : { can_merge: true, methods: ["merge"], queue: false, ...merge } }) as unknown as PrCard;

describe("primaryAction", () => {
  it("leads with the button of the case", () => {
    expect(primaryAction("merge")).toBe("merge");
    expect(primaryAction("update")).toBe("update");
    expect(primaryAction("team_review")).toBe("review");
    expect(primaryAction("reply")).toBe("answer");
    expect(primaryAction("check")).toBe("view_pane");
    expect(primaryAction("conflict")).toBe("assign");
    expect(primaryAction("assign_reviewer")).toBe("assign_reviewer");
    expect(primaryAction("queued")).toBe("github");
  });
});

describe("mergeWhy", () => {
  it("is null only when the state is mergeable, permitted, not a draft and no agent is working", () => {
    expect(mergeWhy(pr({}), card({}), false)).toBeNull();
    expect(mergeWhy(pr({ work: { merge_state: "unstable" } }), card({}), false)).toBeNull();
  });

  it("names the first thing in the way", () => {
    expect(mergeWhy(pr({}), null, false)?.key).toBe("unread");
    expect(mergeWhy(pr({}), card({ can_merge: false }), false)?.key).toBe("permission");
    expect(mergeWhy(pr({ is_draft: true }), card({}), false)?.key).toBe("draft");
    expect(mergeWhy(pr({}), card({}), true)?.key).toBe("agent");
    expect(mergeWhy(pr({ work: { merge_state: "dirty" } }), card({}), false)).toMatchObject({ key: "conflict", base: "main" });
    expect(mergeWhy(pr({ work: { merge_state: "blocked", pending_checks: 2 } }), card({}), false)).toMatchObject({ key: "ci", count: 2 });
    expect(mergeWhy(pr({ checks: "failed", work: { merge_state: "blocked", failed_checks: [{ name: "verify", url: null }] } }), card({}), false)).toMatchObject({ key: "failed", count: 1 });
    expect(mergeWhy(pr({ review: "changes_requested", work: { merge_state: "blocked" } }), card({}), false)?.key).toBe("changes");
    expect(mergeWhy(pr({ work: { merge_state: "blocked" } }), card({}), false)?.key).toBe("reviewer");
    expect(mergeWhy(pr({ work: { merge_state: "unknown" } }), card({}), false)?.key).toBe("unknown");
  });
});

describe("confirmHolds", () => {
  const merge = { action: "merge", head: "a".repeat(40), method: "squash", queue: false } as const;
  const same = { head: "a".repeat(40), queue: false, method: "squash", mergeBlocked: false, behind: true } as const;

  it("holds while the card shows what the confirmation named", () => {
    expect(confirmHolds(merge, same)).toBe(true);
    expect(confirmHolds({ ...merge, action: "update", method: null }, { ...same, method: "merge", mergeBlocked: true })).toBe(true);
  });

  it("closes when the head, the queue, the method or Merge's permission moved", () => {
    expect(confirmHolds(merge, { ...same, head: "b".repeat(40) })).toBe(false);
    expect(confirmHolds(merge, { ...same, queue: true })).toBe(false);
    expect(confirmHolds(merge, { ...same, method: "rebase" })).toBe(false);
    expect(confirmHolds(merge, { ...same, mergeBlocked: true })).toBe(false);
    expect(confirmHolds({ ...merge, action: "update", method: null }, { ...same, head: null })).toBe(false);
    expect(confirmHolds({ ...merge, action: "update", method: null }, { ...same, behind: false })).toBe(false);
  });
});

describe("fileOpen", () => {
  const committed = [{ path: "/work/repo/src/app.ts" }];
  const input = { pr: { url: "https://github.com/acme/repo/pull/7", head_branch: "feat/x" }, committed, file: "src/app.ts" };

  it("opens the Hide diff only in a checkout of the branch whose committed changes hold the file", () => {
    expect(fileOpen({ ...input, front: { branch: "feat/x", path: "/work/repo" } })).toEqual({ kind: "diff", path: "/work/repo/src/app.ts" });
  });

  it("goes to GitHub's files page otherwise", () => {
    const github = { kind: "github", url: "https://github.com/acme/repo/pull/7/files" };
    expect(fileOpen({ ...input, front: { branch: "main", path: "/work/repo" } })).toEqual(github);
    expect(fileOpen({ ...input, front: null })).toEqual(github);
    expect(fileOpen({ ...input, front: { branch: "feat/x", path: "/work/repo" }, committed: null })).toEqual(github);
    expect(fileOpen({ ...input, front: { branch: "feat/x", path: "/work/repo" }, file: "src/other.ts" })).toEqual(github);
  });
});

describe("splitFiles", () => {
  it("folds the generated files into a count", () => {
    const files = [{ path: "a.ts", generated: false }, { path: "Cargo.lock", generated: true }, { path: "dist/a.js", generated: true }] as NonNullable<PrCard["files"]>;
    expect(splitFiles(files)).toEqual({ shown: [files[0]], generated: 2 });
    expect(splitFiles(null)).toEqual({ shown: [], generated: 0 });
  });
});

describe("prFacts", () => {
  it("says the checks, the review and the conflict GitHub reported, nothing it did not", () => {
    expect(prFacts(pr({ state: "mergeable", checks: "passing", review: "approved" }))).toEqual([{ kind: "checks_passing" }, { kind: "approved" }, { kind: "no_conflict", base: "main" }]);
    expect(prFacts(pr({ state: "pending", checks: "pending", review: null, work: { merge_state: "blocked", pending_checks: 8 } }))).toEqual([{ kind: "checks_running", count: 8 }]);
    expect(prFacts(pr({ state: "failed", checks: "failed", review: "changes_requested", work: { merge_state: "dirty", failed_checks: [{ name: "a", url: null }, { name: "b", url: null }] } }))).toEqual([{ kind: "checks_failed", count: 2 }, { kind: "changes" }, { kind: "conflict", base: "main" }]);
    expect(prFacts(pr({ state: "merged", checks: "passing", review: null }))).toEqual([{ kind: "checks_passing" }]);
  });
});

describe("milestones", () => {
  it("lists only what happened, each with the time GitHub gave it, and leaves a running check to the now row", () => {
    const merged = pr({ state: "merged", checks: "passing", review: "approved", created_at_unix_ms: 1, merged_at_unix_ms: 9 });
    expect(milestones(merged, { checks_completed_at_unix_ms: 3, review_at_unix_ms: 5 })).toEqual([
      { kind: "opened", at: 1 }, { kind: "checks_passed", at: 3 }, { kind: "approved", at: 5 }, { kind: "merged", at: 9 },
    ]);
    expect(milestones(pr({ state: "pending", checks: "pending", review: null, work: { pending_checks: 2, review_requests: [{ name: "kim" }], review_requested_at_unix_ms: 4 } }), null)).toEqual([
      { kind: "opened", at: null }, { kind: "review_requested", at: 4, who: ["kim"] },
    ]);
    expect(milestones(pr({ state: "failed", checks: "failed", review: "changes_requested", created_at_unix_ms: 1 }), null)).toEqual([
      { kind: "opened", at: 1 }, { kind: "checks_failed", at: null }, { kind: "changes", at: null },
    ]);
  });
});

describe("timeline", () => {
  const rows = milestones(pr({ state: "mergeable", checks: "passing", review: "approved", created_at_unix_ms: 10 }), { checks_completed_at_unix_ms: 30, review_at_unix_ms: 50 });
  const sessions = [{ session: "late", at: 40 }, { session: "early", at: 5 }, { session: "unknown", at: null }];

  it("orders sessions by start and milestones by time, oldest first, with a timeless session first and now last on an open pull request", () => {
    const open = pr({ state: "pending", checks: "pending", work: { pending_checks: 3 } });
    expect(timeline(sessions, rows, open).map((entry) => (entry.kind === "session" ? entry.session : entry.kind === "milestone" ? entry.milestone.kind : `now:${entry.running}`))).toEqual([
      "unknown", "early", "opened", "checks_passed", "late", "approved", "now:3",
    ]);
  });

  it("stands a timeless milestone right after the one before it, and ends a merged pull request without now", () => {
    const merged = pr({ state: "merged", checks: "passing", review: "approved", created_at_unix_ms: 10, merged_at_unix_ms: 60 });
    const timeless = milestones(merged, null);
    expect(timeline([{ session: "s", at: 20 }], timeless, merged).map((entry) => (entry.kind === "session" ? entry.session : entry.kind === "milestone" ? entry.milestone.kind : "now"))).toEqual([
      "opened", "checks_passed", "approved", "s", "merged",
    ]);
  });
});

it("reports actual CI totals without counting skipped or unknown checks as passes", () => {
  const counts = { total: 18, passed: 14, running: 3, waiting: 1, failed: 0, skipped: 0, unknown: 0 };
  const summary = (over = {}) => checkSummary(pr({ checks: "pending", work: { check_counts: { ...counts, ...over } } }));
  expect(summary()).toEqual({ key: "recordCard.ciRunning", values: { count: 3, total: 18 } });
  expect(summary({ running: 0, waiting: 4 })).toEqual({ key: "recordCard.ciWaiting", values: { count: 4, total: 18 } });
  expect(summary({ failed: 2, passed: 12 })).toEqual({ key: "recordCard.ciFailed", values: { count: 2, total: 18 } });
  expect(summary({ passed: 18, running: 0, waiting: 0 }).key).toBe("recordCard.ciPassed");
  expect(summary({ passed: 16, running: 0, waiting: 0, skipped: 2 })).toEqual({ key: "recordCard.ciFinished", values: { count: 16, total: 18 } });
  expect(summary({ passed: 16, running: 0, waiting: 0, unknown: 2 }).key).toBe("recordCard.ciUnknown");
  expect(checkSummary(pr({ checks: "passing" })).key).toBe("recordCard.ciUnknown");
  expect(checkSummary(pr({ checks: "none", work: { check_counts: { ...counts, total: 0 } } })).key).toBe("recordCard.ciNone");
});
