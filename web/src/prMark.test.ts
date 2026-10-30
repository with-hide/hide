import fs from "node:fs";
import path from "node:path";
import { GitMergeIcon, GitPullRequestClosedIcon, GitPullRequestDraftIcon, GitPullRequestIcon } from "lucide-react";
import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { badgeWord, checksWord, PR_LOOK, prMarkParts, pullsStaleness, reviewWord, rowPulls, staleLabel, staleness } from "./prMark";
import type { AgentRow, PullRequest, SessionPull } from "./snapshot";
import { sessionPull } from "../test/sessionPull";

// The one PR mark (docs/status-model.md, Pull request visual states): the
// palette the operator approved on 2026-10-10, one token per state.

const t = initializeInterfaceI18n("en").getFixedT(null, "translation");
const ko = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("the PR look", () => {
  it("has a look for every state the core sends, in the core's order", () => {
    const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
    expect(Object.keys(PR_LOOK)).toEqual(contract.pr_state);
  });

  it("draws failed red, pending amber, mergeable green, merged purple, draft grey and closed dim, each in its own token", () => {
    expect(Object.fromEntries(Object.entries(PR_LOOK).map(([state, look]) => [state, look.tone]))).toEqual({
      failed: "text-pr-failed",
      pending: "text-pr-pending",
      mergeable: "text-pr-mergeable",
      draft: "text-pr-draft",
      merged: "text-pr-merged",
      closed: "text-pr-closed",
    });
  });

  it("draws a merge for a merged pull request, a draft and a closed one in their own shapes, and the pull request icon for the rest", () => {
    expect([PR_LOOK.merged.icon, PR_LOOK.draft.icon, PR_LOOK.closed.icon]).toEqual([GitMergeIcon, GitPullRequestDraftIcon, GitPullRequestClosedIcon]);
    expect([PR_LOOK.failed.icon, PR_LOOK.pending.icon, PR_LOOK.mergeable.icon]).toEqual([GitPullRequestIcon, GitPullRequestIcon, GitPullRequestIcon]);
  });
});

describe("the card words", () => {
  it("reads a change request red as a failed check, an approval green, and a review still required quiet", () => {
    expect(reviewWord("changes_requested")).toEqual({ key: "board.review.changes", tone: "text-pr-failed" });
    expect(reviewWord("approved")).toEqual({ key: "board.review.approved", tone: "text-pr-mergeable" });
    expect(reviewWord("review_required")?.tone).toBe("text-muted-foreground");
    expect(reviewWord(null)).toBeNull();
  });

  it("names checks in the colour of the state they lead to, and says nothing until GitHub read them", () => {
    expect([checksWord("passing")?.tone, checksWord("failed")?.tone, checksWord("pending")?.tone]).toEqual(["text-pr-mergeable", "text-pr-failed", "text-pr-pending"]);
    expect([checksWord("none"), checksWord("unknown"), checksWord(undefined)]).toEqual([null, null, null]);
  });

  it("names the lifecycle on the badge, or the review decision under review with Draft beside a draft, in the interface language", () => {
    const base: PullRequest = { number: 1, title: "", url: "", badge: "open", review: null, is_draft: false, state: "pending" };
    expect(badgeWord({ ...base, badge: "merged" }, t)).toEqual({ label: "Merged", draft: false });
    expect(badgeWord({ ...base, badge: "closed" }, t)).toEqual({ label: "Closed", draft: false });
    expect(badgeWord(base, t)).toEqual({ label: "Open", draft: false });
    expect(badgeWord({ ...base, is_draft: true }, t)).toEqual({ label: "Draft", draft: false });
    expect(badgeWord({ ...base, badge: "review", review: "changes_requested" }, t)).toEqual({ label: "Changes requested", draft: false });
    expect(badgeWord({ ...base, badge: "review", review: "approved", is_draft: true }, t)).toEqual({ label: "Approved", draft: true });
    expect(badgeWord({ ...base, badge: "merged" }, ko).label).toBe("머지됨");
  });
});

describe("staleness", () => {
  it("dims only once GitHub could not be read again, and says when it last was", () => {
    expect(staleness(null)).toBeUndefined();
    const read = { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: 1_800_000_000_000, unavailable_reason: null };
    expect(staleness(read)).toEqual({ stale: false, lastRead: 1_800_000_000_000 });
    expect(staleLabel(staleness(read), t, "en")).toBeNull();
    expect(staleLabel(staleness({ ...read, stale: true }), t, "en")).toMatch(/^GitHub last read .+ · cannot read now$/);
    expect(staleLabel({ stale: true, lastRead: null }, t, "en")).toBe("GitHub last read - · cannot read now");
  });
});

describe("a PR mark's parts", () => {
  it("draws one PR as its state and number", () => {
    expect(prMarkParts([sessionPull(924, "mergeable")])).toEqual({ state: "mergeable", number: 924 });
  });

  it("counts several PRs per state, worst first, whatever order they arrive in", () => {
    const pulls = [sessionPull(1, "merged"), sessionPull(2, "mergeable"), sessionPull(3, "merged"), sessionPull(4, "failed")];
    expect(prMarkParts(pulls)).toEqual({ counts: [{ state: "failed", count: 1 }, { state: "mergeable", count: 1 }, { state: "merged", count: 2 }] });
  });

  it("draws nothing without a PR", () => {
    expect(prMarkParts([])).toBeNull();
  });
});

describe("a row's list", () => {
  const row = (pulls: SessionPull[] | null) => ({ pulls: pulls ? { pulls } : null }) as unknown as AgentRow;

  it("reads the core's list whole, or only the session's own for a row that draws its descendants beside it", () => {
    const own = sessionPull(1, "pending");
    const child = sessionPull(2, "closed", { own: false, holders: ["child"] });
    expect(rowPulls(row([own, child]))).toEqual([own, child]);
    expect(rowPulls(row([own, child]), "own")).toEqual([own]);
    expect(rowPulls(row(null))).toEqual([]);
  });

  it("is stale while any of its pull requests is, with the oldest last read", () => {
    expect(pullsStaleness([])).toBeUndefined();
    expect(pullsStaleness([sessionPull(1, "pending", { last_read_at_unix_ms: 300 })])).toEqual({ stale: false, lastRead: null });
    const stale = [sessionPull(1, "pending", { stale: true, last_read_at_unix_ms: 200 }), sessionPull(2, "merged", { stale: true, last_read_at_unix_ms: 100 }), sessionPull(3, "pending", { last_read_at_unix_ms: 300 })];
    expect(pullsStaleness(stale)).toEqual({ stale: true, lastRead: 100 });
    expect(pullsStaleness([sessionPull(1, "pending", { stale: true })])).toEqual({ stale: true, lastRead: null });
  });
});
