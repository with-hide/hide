import { GitMergeIcon, GitPullRequestClosedIcon, GitPullRequestDraftIcon, GitPullRequestIcon, type LucideIcon } from "lucide-react";
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { formatDateTime } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import { catalogWorkspaces, type AgentPullRequest, type AgentRow, type GithubStatus, type PrState, type PullRequest, type SnapshotRest } from "./snapshot";

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

/** The row's own PRs, worst first, as the core ordered them. */
export function ownPulls(agent: AgentRow): { pull: AgentPullRequest; state: PrState }[] {
  const pulls = agent.request?.pull_requests ?? [];
  return (agent.state.pr?.pulls ?? []).map(({ index, state }) => {
    const pull = pulls[index];
    if (!pull) throw new Error("Own PR summary points past the row's pull requests");
    return { pull, state };
  });
}

/** A distinct PR in a Sessions root's tree, with the sessions that hold it. */
export type SessionPull = { pull: AgentPullRequest; state: PrState; owners: AgentRow[] };

const PR_RANK = Object.fromEntries(Object.keys(PR_LOOK).map((state, rank) => [state, rank])) as Record<PrState, number>;

/**
 * The root's and every descendant's core-projected PRs, independent of tree
 * folds and sibling limits. URL is identity: equal numbers in different
 * repositories remain separate, and repeated sightings keep all owners.
 */
export function sessionPulls(root: AgentRow, byPane: ReadonlyMap<string, AgentRow>): SessionPull[] {
  const pulls = new Map<string, SessionPull>();
  const seen = new Set<string>();
  const pending = [root];
  while (pending.length > 0) {
    const agent = pending.pop()!;
    if (seen.has(agent.pane_id)) continue;
    seen.add(agent.pane_id);
    for (const { pull, state } of ownPulls(agent)) {
      const existing = pulls.get(pull.url);
      if (!existing) pulls.set(pull.url, { pull, state, owners: [agent] });
      else {
        existing.owners.push(agent);
        if (PR_RANK[state] < PR_RANK[existing.state]) {
          existing.pull = pull;
          existing.state = state;
        }
      }
    }
    for (const pane of [...(agent.lineage_child_pane_ids ?? [])].reverse()) {
      const child = byPane.get(pane);
      if (child && !seen.has(pane)) pending.push(child);
    }
  }
  return [...pulls.values()].sort((a, b) => PR_RANK[a.state] - PR_RANK[b.state]);
}

/**
 * What a row's PR mark gathers: a parent's family, itself and every
 * descendant, or undefined for a row with no child in the list, which shows
 * its own PRs. The 태스크 row, the 프로젝트 root row and the pane header agree.
 */
export function familyPulls(agent: AgentRow, byPane: ReadonlyMap<string, AgentRow>): SessionPull[] | undefined {
  return (agent.lineage_child_pane_ids ?? []).some((pane) => byPane.has(pane)) ? sessionPulls(agent, byPane) : undefined;
}

/** Any stale contributing GitHub read dims the tree's mark, using its oldest failed read. */
export function sessionStaleness(rest: SnapshotRest | null, pulls: SessionPull[]): PrStaleness | undefined {
  const urls = new Set(pulls.map(({ pull }) => pull.url));
  if (urls.size === 0) return undefined;
  const reads = catalogWorkspaces(rest)
    .filter((project) => project.pull_requests?.some((pull) => urls.has(pull.url)))
    .map((project) => staleness(project.checkouts.find((checkout) => checkout.github)?.github))
    .filter((read): read is PrStaleness => read !== undefined);
  if (reads.length === 0) return undefined;
  const stale = reads.filter((read) => read.stale);
  const contributing = stale.length > 0 ? stale : reads;
  return { stale: stale.length > 0, lastRead: contributing.some((read) => read.lastRead === null) ? null : Math.min(...contributing.map((read) => read.lastRead!)) };
}

/** What a mark draws: one PR's state and number, or for several, how many of each state, worst first. */
export type PrMarkParts = { state: PrState; number: number } | { counts: { state: PrState; count: number }[] };

/** The mark of a row's own PRs or a root's family; null for none. */
export function prMarkParts(pulls: readonly { pull: AgentPullRequest; state: PrState }[]): PrMarkParts | null {
  const [first] = pulls;
  if (!first) return null;
  if (pulls.length === 1) return { state: first.state, number: first.pull.number };
  const counts = new Map<PrState, number>();
  for (const { state } of pulls) counts.set(state, (counts.get(state) ?? 0) + 1);
  return { counts: [...counts].sort(([a], [b]) => PR_RANK[a] - PR_RANK[b]).map(([state, count]) => ({ state, count })) };
}

/** The GitHub read of the project that lists the row's own PRs, each row its own. */
export function agentStaleness(rest: SnapshotRest | null, agent: AgentRow): PrStaleness | undefined {
  const urls = new Set(ownPulls(agent).map(({ pull }) => pull.url));
  if (urls.size === 0) return undefined;
  const project = catalogWorkspaces(rest).find((row) => row.pull_requests?.some((pull) => urls.has(pull.url)));
  return staleness(project?.checkouts.find((checkout) => checkout.github)?.github);
}
