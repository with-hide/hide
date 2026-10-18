// The rules of a pull request's decision card that are not drawing (PRD
// work-tab-pr-lens D-06, D-21, D-27; PRD sidebar-task-lens D-35, D-36; PRD
// pr-card-and-session-record-redesign D-05, D-06, D-15): the body's outline,
// which button is primary for a case, why Merge is not available, the
// decision box's facts, the timeline's milestones and their order among the
// session rows, and where a changed file opens. The core decided the case
// (`reason.kind`) and read the card's parts; nothing here re-derives a group.

import type { PrReason } from "./agentScope";
import type { PrCard, PrMergeMethod, PullRequest, PullRequestWork } from "./snapshot";

/** One truthful summary for the card and timeline; unknown totals stay unknown. */
export function checkSummary(pr: PullRequest) {
  const counts = pr.work?.check_counts;
  const values = { count: 0, total: counts?.total ?? 0 };
  if (!counts || counts.total === 0) return { key: pr.checks === "none" ? "recordCard.ciNone" as const : "recordCard.ciUnknown" as const, values };
  if (counts.failed > 0) return { key: "recordCard.ciFailed" as const, values: { ...values, count: counts.failed } };
  if (counts.running > 0) return { key: "recordCard.ciRunning" as const, values: { ...values, count: counts.running } };
  if (counts.waiting > 0) return { key: "recordCard.ciWaiting" as const, values: { ...values, count: counts.waiting } };
  if (counts.passed === counts.total) return { key: "recordCard.ciPassed" as const, values };
  return { key: counts.unknown === 0 ? "recordCard.ciFinished" as const : "recordCard.ciUnknown" as const, values: { ...values, count: counts.passed } };
}

/** What the primary button of the button row is (D-21). */
export type PrimaryAction = "merge" | "update" | "review" | "assign" | "answer" | "view_pane" | "assign_reviewer" | "github";

export function primaryAction(kind: PrReason["kind"]): PrimaryAction {
  switch (kind) {
    case "merge":
      return "merge";
    case "update":
      return "update";
    case "review":
    case "team_review":
      return "review";
    case "reply":
      return "answer";
    case "check":
      return "view_pane";
    case "failed":
    case "conflict":
    case "changes":
      return "assign";
    case "assign_reviewer":
      return "assign_reviewer";
    default:
      return "github";
  }
}

/** Why Merge cannot be pressed now, as the key of the reason word and its values; null when it can (B30). */
export type MergeWhy = { key: "ci" | "failed" | "changes" | "agent" | "conflict" | "reviewer" | "blocked" | "draft" | "permission" | "unknown" | "unread"; count: number; base: string };

export function mergeWhy(pr: PullRequest, card: PrCard | null, working: boolean): MergeWhy | null {
  const work = pr.work;
  if (!card?.merge) return { key: "unread", count: 0, base: "" };
  if (!card.merge.can_merge) return { key: "permission", count: 0, base: "" };
  if (pr.is_draft) return { key: "draft", count: 0, base: "" };
  if (working) return { key: "agent", count: 0, base: "" };
  switch (work?.merge_state) {
    case "clean":
    case "has_hooks":
    case "unstable":
      return null;
    case "dirty":
      return { key: "conflict", count: 0, base: pr.base_branch ?? "" };
    case "behind":
      return { key: "blocked", count: 0, base: "" };
    case "blocked":
      if ((work.pending_checks ?? 0) > 0) return { key: "ci", count: work.pending_checks, base: "" };
      if (pr.checks === "failed") return { key: "failed", count: Math.max(work.failed_checks?.length ?? 0, 1), base: "" };
      if (pr.review === "changes_requested") return { key: "changes", count: 0, base: "" };
      return { key: "reviewer", count: 0, base: "" };
    case "draft":
      return { key: "draft", count: 0, base: "" };
    default:
      return { key: "unknown", count: 0, base: "" };
  }
}

/** The method Merge uses unless the operator picked another: GitHub's own default order. */
export function defaultMethod(card: PrCard | null): PrMergeMethod | null {
  const methods = card?.merge?.methods ?? [];
  return methods[0] ?? null;
}

/**
 * Where a changed file opens (D-27): the Hide diff when the Workspace in
 * front is a checkout of the pull request's branch and its committed changes
 * hold the file, else GitHub's files page of the pull request.
 */
export type FileOpen = { kind: "diff"; path: string } | { kind: "github"; url: string };

export function fileOpen(input: { pr: Pick<PullRequest, "url" | "head_branch">; front: { branch: string | null; path: string } | null; committed: readonly { path: string }[] | null; file: string }): FileOpen {
  const { pr, front, committed, file } = input;
  if (front && front.branch !== null && front.branch === pr.head_branch && committed) {
    const absolute = `${front.path}/${file}`;
    if (committed.some((entry) => entry.path === absolute)) return { kind: "diff", path: absolute };
  }
  return { kind: "github", url: `${pr.url}/files` };
}

/** The files the card draws and the generated ones folded into one line (B28). */
export function splitFiles(files: PrCard["files"]): { shown: NonNullable<PrCard["files"]>; generated: number } {
  const all = files ?? [];
  return { shown: all.filter((file) => !file.generated), generated: all.filter((file) => file.generated).length };
}

/** What a confirmation named when it opened (D-22). */
export type Confirmed = { action: "merge" | "update"; head: string; method: PrMergeMethod | null; queue: boolean };

/**
 * Whether a confirmation still holds: the card shows the same head, the same
 * queue and, for a merge, the same method and Merge still allowed, or for an
 * update the branch still behind its base. Else
 * it closes and nothing is sent, so a click on Yes never writes what the
 * operator was not shown.
 */
export function confirmHolds(confirm: Confirmed, now: { head: string | null; queue: boolean; method: PrMergeMethod | null; mergeBlocked: boolean; behind: boolean }): boolean {
  if (confirm.head !== now.head || confirm.queue !== now.queue) return false;
  if (confirm.action === "update") return now.behind;
  return !now.mergeBlocked && (now.queue || confirm.method === now.method);
}

/** Where the merge stands with its base, as GitHub said it last. */
const MERGEABLE: readonly PullRequestWork["merge_state"][] = ["clean", "has_hooks", "unstable"];

/**
 * One fact of the decision box's fact line (PRD sidebar-task-lens B33): the
 * checks, the review and the conflict with the base, each said only when
 * GitHub said it. The card words each.
 */
export type PrFact =
  | { kind: "checks_passing" }
  | { kind: "checks_running"; count: number }
  | { kind: "checks_failed"; count: number }
  | { kind: "approved" }
  | { kind: "changes" }
  | { kind: "review_required" }
  | { kind: "conflict"; base: string }
  | { kind: "no_conflict"; base: string }
  | { kind: "behind"; base: string };

export function prFacts(pr: PullRequest): PrFact[] {
  const facts: PrFact[] = [];
  const work = pr.work;
  const base = pr.base_branch ?? "";
  if (pr.checks === "pending" || (work?.pending_checks ?? 0) > 0) facts.push({ kind: "checks_running", count: Math.max(work?.pending_checks ?? 0, 1) });
  else if (pr.checks === "failed") facts.push({ kind: "checks_failed", count: Math.max(work?.failed_checks.length ?? 0, 1) });
  else if (pr.checks === "passing") facts.push({ kind: "checks_passing" });
  if (pr.review === "approved") facts.push({ kind: "approved" });
  else if (pr.review === "changes_requested") facts.push({ kind: "changes" });
  else if (pr.review === "review_required") facts.push({ kind: "review_required" });
  if (pr.state === "merged" || pr.state === "closed" || !work) return facts;
  if (work.merge_state === "dirty") facts.push({ kind: "conflict", base });
  else if (work.merge_state === "behind") facts.push({ kind: "behind", base });
  else if (MERGEABLE.includes(work.merge_state)) facts.push({ kind: "no_conflict", base });
  return facts;
}

/**
 * One milestone row of the card's timeline (PRD pr-card-and-session-record-
 * redesign D-05, B5): only what happened, each with the time GitHub gave it.
 * A check or review row whose time the card has not read yet carries `null`
 * and stands right after the row it follows (B4).
 */
export type Milestone =
  | { kind: "opened"; at: number | null }
  | { kind: "checks_passed"; at: number | null }
  | { kind: "checks_failed"; at: number | null }
  | { kind: "review_requested"; at: number | null; who: string[] }
  | { kind: "approved"; at: number | null }
  | { kind: "changes"; at: number | null }
  | { kind: "merged"; at: number | null };

/** The times the card's own read adds to the list's values (D-15). */
export type MilestoneTimes = Pick<PrCard, "checks_completed_at_unix_ms" | "review_at_unix_ms">;

export function milestones(pr: PullRequest, times: MilestoneTimes | null): Milestone[] {
  const rows: Milestone[] = [{ kind: "opened", at: pr.created_at_unix_ms ?? null }];
  const work = pr.work;
  const running = pr.checks === "pending" || (work?.pending_checks ?? 0) > 0;
  // A completed verdict only: a run still going is the 지금 row's (B6).
  if (!running && pr.checks === "failed") rows.push({ kind: "checks_failed", at: times?.checks_completed_at_unix_ms ?? null });
  else if (!running && pr.checks === "passing") rows.push({ kind: "checks_passed", at: times?.checks_completed_at_unix_ms ?? null });
  if (pr.review === "approved") rows.push({ kind: "approved", at: times?.review_at_unix_ms ?? null });
  else if (pr.review === "changes_requested") rows.push({ kind: "changes", at: times?.review_at_unix_ms ?? null });
  else if (work && work.review_requests.length > 0 && pr.state !== "merged" && pr.state !== "closed") rows.push({ kind: "review_requested", at: work.review_requested_at_unix_ms, who: work.review_requests.map((request) => request.name) });
  if (pr.state === "merged") rows.push({ kind: "merged", at: pr.merged_at_unix_ms ?? null });
  return rows;
}

/** The order a milestone takes among the others at one time, and where a timeless one stands: after the row of the kind before it. */
const MILESTONE_RANK: Record<Milestone["kind"], number> = { opened: 0, checks_passed: 1, checks_failed: 1, review_requested: 2, approved: 3, changes: 3, merged: 4 };

/** What the timeline draws: a session row at its start, a milestone at its time, and 지금 last on an open pull request (B4, B6). */
export type TimelineEntry<S> =
  | { kind: "session"; session: S; at: number | null }
  | { kind: "milestone"; milestone: Milestone }
  | { kind: "now"; running: number };

/**
 * The timeline's rows oldest first (D-06, B4): sessions by their start,
 * milestones by their time, a milestone with no time right after the one
 * before it, a session with no start first, and 지금 at the end of an open
 * pull request, with how many checks still run.
 */
export function timeline<S>(sessions: readonly { session: S; at: number | null }[], rows: readonly Milestone[], pr: Pick<PullRequest, "state" | "checks" | "work">): TimelineEntry<S>[] {
  const keyed: { entry: TimelineEntry<S>; at: number; rank: number }[] = sessions.map(({ session, at }) => ({ entry: { kind: "session", session, at }, at: at ?? Number.NEGATIVE_INFINITY, rank: -1 }));
  let last = Number.NEGATIVE_INFINITY;
  for (const milestone of rows) {
    // A timeless milestone takes the time of the one before it and sorts after it by rank.
    if (milestone.at !== null) last = Math.max(last, milestone.at);
    keyed.push({ entry: { kind: "milestone", milestone }, at: milestone.at ?? last, rank: MILESTONE_RANK[milestone.kind] });
  }
  keyed.sort((a, b) => a.at - b.at || a.rank - b.rank);
  const entries = keyed.map(({ entry }) => entry);
  if (pr.state !== "merged" && pr.state !== "closed") {
    const running = pr.checks === "pending" || (pr.work?.pending_checks ?? 0) > 0 ? Math.max(pr.work?.pending_checks ?? 0, 1) : 0;
    entries.push({ kind: "now", running });
  }
  return entries;
}
