import { GitMergeIcon, GitPullRequestClosedIcon, GitPullRequestDraftIcon, GitPullRequestIcon, type LucideIcon } from "lucide-react";
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { formatDateTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import type { AgentRow, GithubStatus, PrState, PullRequest, SessionPull } from "./snapshot";

// The one place a pull request's state becomes an icon, a colour and a word
// (docs/status-model.md, Pull request visual states). The core decides the
// state (`PrState::of`); every PR mark, card, row and search result draws it
// from here, so the same pull request looks the same everywhere.

/** Each state's icon, its colour class and its word, worst first. */
export const PR_LOOK: Record<PrState, { icon: LucideIcon; tone: string; word: MessageKey }> = {
  failed: { icon: GitPullRequestIcon, tone: "text-pr-failed", word: "agentSessions.pr.failed" },
  pending: { icon: GitPullRequestIcon, tone: "text-pr-pending", word: "agentSessions.pr.pending" },
  mergeable: { icon: GitPullRequestIcon, tone: "text-pr-mergeable", word: "agentSessions.pr.mergeable" },
  draft: { icon: GitPullRequestDraftIcon, tone: "text-pr-draft", word: "agentSessions.pr.draft" },
  merged: { icon: GitMergeIcon, tone: "text-pr-merged", word: "agentSessions.pr.merged" },
  closed: { icon: GitPullRequestClosedIcon, tone: "text-pr-closed", word: "agentSessions.pr.closed" },
};

/** A word and its colour: a card's Checks and Review rows. */
type PrWord = { key: MessageKey; tone: string };

const CHECKS_WORD: Partial<Record<NonNullable<PullRequest["checks"]>, PrWord>> = {
  passing: { key: "card.checkPassing", tone: "text-pr-mergeable" },
  failed: { key: "card.checkFailed", tone: "text-pr-failed" },
  pending: { key: "card.checkPending", tone: "text-pr-pending" },
};

/** The CI rollup once GitHub has read it; no checks or unknown checks say nothing. */
export function checksWord(checks: PullRequest["checks"]): PrWord | null {
  return (checks && CHECKS_WORD[checks]) ?? null;
}

const REVIEW_WORD: Record<NonNullable<PullRequest["review"]>, PrWord> = {
  approved: { key: "board.review.approved", tone: "text-pr-mergeable" },
  // Changes asked for are something to fix, red as a failed check is.
  changes_requested: { key: "board.review.changes", tone: "text-pr-failed" },
  review_required: { key: "board.review.required", tone: "text-muted-foreground" },
};

export function reviewWord(review: PullRequest["review"] | undefined): PrWord | null {
  return review ? REVIEW_WORD[review] : null;
}

/**
 * The PR card badge's word (PRD checkout-pr-glyph-card D-08): the lifecycle
 * for a merged, closed, open or draft pull request, and the review decision
 * for one under review, with `draft` beside a draft under review. The colour
 * is the icon's alone: the word says what, not how it stands.
 */
export function badgeWord(pr: PullRequest, t: TFunction<"translation">): { label: string; draft: boolean } {
  switch (pr.badge) {
    case "merged":
    case "closed":
      return { label: t(PR_LOOK[pr.badge].word), draft: false };
    case "open":
      return { label: pr.is_draft ? t(PR_LOOK.draft.word) : t("requests.badge.open"), draft: false };
    case "review":
      return { label: t(REVIEW_WORD[pr.review ?? "review_required"].key), draft: pr.is_draft };
  }
}

/** Merged or closed: the pull request's life is over. */
export function settled(state: PrState): boolean {
  return state === "merged" || state === "closed";
}

/** When GitHub could not be read again, the last read time a PR mark dims for. */
export type PrStaleness = { stale: boolean; lastRead: number | null };

/** A project's GitHub read as a mark's staleness: undefined before any answer. */
export function staleness(github: GithubStatus | null | undefined): PrStaleness | undefined {
  return github ? { stale: github.stale === true, lastRead: github.last_success_at_unix_ms ?? null } : undefined;
}

/** The tooltip line of a stale mark, or null while GitHub reads. */
export function staleLabel(value: PrStaleness | undefined, t: TFunction<"translation">, language: InterfaceLanguage): string | null {
  if (!value?.stale) return null;
  const time = value.lastRead == null ? "-" : formatDateTime(language, value.lastRead, { dateStyle: "short", timeStyle: "short" });
  return t("agentSessions.pr.stale", { time });
}

/**
 * A row's pull requests as the core lists them (`agent_state::work::family`):
 * its session's and every descendant's, live or ended, worst first; `own`
 * keeps those its own session holds or made, which a 프로젝트 tree child row
 * draws under a root that already draws the family.
 */
export function rowPulls(agent: AgentRow, scope: "family" | "own" = "family"): SessionPull[] {
  const pulls = agent.pulls?.pulls ?? [];
  return scope === "own" ? pulls.filter((pull) => pull.own) : pulls;
}

/** A list's GitHub read: stale while any of its pull requests is, with the oldest of those last reads; undefined for no pull request. */
export function pullsStaleness(pulls: readonly SessionPull[]): PrStaleness | undefined {
  if (pulls.length === 0) return undefined;
  const stale = pulls.filter((pull) => pull.stale);
  if (stale.length === 0) return { stale: false, lastRead: null };
  return { stale: true, lastRead: stale.some((pull) => pull.last_read_at_unix_ms === null) ? null : Math.min(...stale.map((pull) => pull.last_read_at_unix_ms!)) };
}

/** A closed pull request is drawn dimmed in a session's list: it stays, but asks nothing (docs/status-model.md, One list per session). */
export const CLOSED_DIM = "opacity-(--opacity-dimmed)";

const PR_RANK = Object.fromEntries(Object.keys(PR_LOOK).map((state, rank) => [state, rank])) as Record<PrState, number>;

/** What a mark draws: one PR's state and number, or for several, how many of each state, worst first. */
export type PrMarkParts = { state: PrState; number: number } | { counts: { state: PrState; count: number }[] };

/** The mark of a list of PRs; null for none. */
export function prMarkParts(pulls: readonly Pick<SessionPull, "number" | "state">[]): PrMarkParts | null {
  const [first] = pulls;
  if (!first) return null;
  if (pulls.length === 1) return { state: first.state, number: first.number };
  const counts = new Map<PrState, number>();
  for (const { state } of pulls) counts.set(state, (counts.get(state) ?? 0) + 1);
  return { counts: [...counts].sort(([a], [b]) => PR_RANK[a] - PR_RANK[b]).map(([state, count]) => ({ state, count })) };
}
