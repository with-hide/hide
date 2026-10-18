import { badgeParts } from "./agentRow";
import { scopeOccurrences, type AgentRef } from "./agentScope";
import type { AgentScope } from "./agentScope";
// The Projects list as the sidebar draws it: the raised Needs You and Done
// agents, then pinned rows under their own header, then the activity rows
// with a per-device fold of projects with no agents and one of inactive
// projects, and per project a fold of inactive checkouts. The split only
// reads flags and groups the core set; no age, merge or attention rule is
// repeated.

import type { TFunction } from "i18next";
import { agentGroupTitle, type AgentGroup, type ListedAgent } from "./navigation";
import { badgeWord, checksWord, PR_LOOK, reviewWord, settled, staleness } from "./prMark";
import type { Checkout, InactiveProjectGroup, MarkCounts, PrState, PullRequest, RecentCheckout, SnapshotRest, Workspace } from "./snapshot";

/** Return to this device's last usable checkout, then its primary, then row order. */
export function projectCheckout(workspace: Workspace, recent: readonly RecentCheckout[] = []): Checkout | null {
  const usable = workspace.checkouts.filter((checkout) => checkout.exists);
  for (const visit of recent) {
    if (visit.device_id !== workspace.device_id) continue;
    const checkout = usable.find((checkout) => checkout.id === visit.checkout_id);
    if (checkout) return checkout;
  }
  return usable.find((checkout) => checkout.is_primary) ?? usable[0] ?? null;
}

export type ProjectRow =
  | { kind: "header"; section: "pinned" | "recent"; count: number }
  | { kind: "raised"; group: AgentGroup; agents: ListedAgent[]; more: ListedAgent[]; expanded: boolean }
  | { kind: "workspace"; workspace: Workspace; level: "root" | "child" }
  | { kind: "no_agents"; group: InactiveProjectGroup; count: number }
  | { kind: "inactive_projects"; group: InactiveProjectGroup; count: number };

/** The device folds of the Projects list, as the core grouped them (`navigator`). */
export type ProjectFolds = Pick<NonNullable<SnapshotRest["navigator"]>, "inactive_projects" | "no_agent_projects">;

/** Whether a project's checkouts are drawn: a sleeping project follows its own fold (D-19), any other the operator's. */
export function projectExpanded(workspace: Workspace): boolean {
  const asleep = workspace.agent_presence?.asleep;
  return asleep ? asleep.expanded : workspace.expanded !== false;
}

/**
 * `listed` is every agent the Agents list shows. A raised section holds, in
 * the core's order, the Needs You or Done agents whose pane a drawn project's
 * or the device's Home's checkout owns, so every raised agent outside the Home is also in the tree below; an empty
 * section is left out, and a raised row never unfolds, so it is handed on
 * folded (docs/status-model.md, The descendant badge). Past its cap a
 * section draws the most recent ones in `agents` and folds the rest into
 * `more`, drawn only while `openRaised` names the group.
 */
export function projectRows(
  workspaces: Workspace[],
  folds: ProjectFolds,
  listed: ListedAgent[],
  scope: AgentScope | null,
  openRaised: readonly string[] = [],
): ProjectRow[] {
  const rows: ProjectRow[] = [];
  const drawn = (refs: AgentRef[]) => scopeOccurrences(refs, listed.map(r => r.agent)).map(agent => {
    const row = listed.find(r => r.agent === agent)!;
    return { ...row, agent: { ...agent, lineage_collapsed: true } };
  });
  for (const raised of scope?.raised ?? []) {
    rows.push({ kind: "raised", group: raised.group, agents: drawn(raised.shown), more: drawn(raised.more), expanded: openRaised.includes(raised.group) });
  }
  const pinned = workspaces.filter((row) => row.pinned);
  const recent = workspaces.filter((row) => !row.pinned);
  if (pinned.length > 0) {
    rows.push({ kind: "header", section: "pinned", count: pinned.length });
    for (const workspace of pinned) rows.push({ kind: "workspace", workspace, level: "root" });
  }
  rows.push({ kind: "header", section: "recent", count: recent.length });
  const byId = new Map(recent.map((row) => [row.id, row]));
  const deviceIds = [...new Set(recent.map((row) => row.device_id))];
  for (const deviceId of deviceIds) {
    const quiet = (folds.no_agent_projects ?? []).find((row) => row.device_id === deviceId);
    const inactive = (folds.inactive_projects ?? []).find((row) => row.device_id === deviceId);
    const folded = new Set([...(quiet?.project_ids ?? []), ...(inactive?.project_ids ?? [])]);
    for (const workspace of recent) {
      if (workspace.device_id === deviceId && !folded.has(workspace.id)) {
        rows.push({ kind: "workspace", workspace, level: "root" });
      }
    }
    // No agents, then Inactive projects (D-01): each a fold row with its projects under it while open.
    for (const [kind, group] of [["no_agents", quiet], ["inactive_projects", inactive]] as const) {
      if (!group) continue;
      const members = group.project_ids.map((id) => byId.get(id)).filter((row): row is Workspace => !!row);
      if (members.length === 0) continue;
      rows.push({ kind, group, count: members.length });
      if (group.expanded) for (const workspace of members) rows.push({ kind: "workspace", workspace, level: "child" });
    }
  }
  return rows;
}

export function activeCheckouts(workspace: Workspace): Checkout[] {
  const inactive = new Set([...workspace.inactive_checkouts.checkout_ids, ...(workspace.session_folds?.empty ?? []), ...(workspace.session_folds?.child_only ?? []), ...(workspace.session_folds?.cleanup ?? [])]);
  return workspace.checkouts.filter((row) => !inactive.has(row.id));
}

export function inactiveCheckouts(workspace: Workspace): Checkout[] {
  const byId = new Map(workspace.checkouts.map((row) => [row.id, row]));
  return workspace.inactive_checkouts.checkout_ids.map((id) => byId.get(id)).filter((row): row is Checkout => !!row);
}

type Age = { unit: "now" } | { unit: "minutes" | "hours" | "days"; count: number };

/** The first minute is "now", then minutes, hours and days; a timestamp from the future is "now". */
export function activityAge(unixMs: number | null | undefined, nowMs: number): Age | null {
  if (unixMs == null) return null;
  const seconds = (nowMs - unixMs) / 1000;
  if (seconds < 60) return { unit: "now" };
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return { unit: "minutes", count: minutes };
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return { unit: "hours", count: hours };
  return { unit: "days", count: Math.floor(hours / 24) };
}

export function ageText(age: Age, t: TFunction<"translation">): string {
  switch (age.unit) {
    case "now":
      return t("common.now");
    case "minutes":
      return t("common.ageMinutes", { count: age.count });
    case "hours":
      return t("common.ageHours", { count: age.count });
    case "days":
      return t("common.ageDays", { count: age.count });
  }
}

/** "now" for the first minute, then minutes, hours and days in the interface language; a timestamp from the future is "now". */
export function relativeActivity(unixMs: number | null | undefined, nowMs: number, t: TFunction<"translation">): string | null {
  const age = activityAge(unixMs, nowMs);
  return age === null ? null : ageText(age, t);
}

/**
 * A plain folder's only checkout: a project that is not a Git repository and
 * holds one checkout is drawn as that checkout, on one row.
 */
export function folderCheckout(workspace: Workspace): Checkout | null {
  return !workspace.is_git && workspace.checkouts.length === 1 ? (workspace.checkouts[0] ?? null) : null;
}

/**
 * The pull request a checkout's row speaks for: a stale refresh keeps the
 * last known one (its glyph muted), an unavailable answer that is not stale
 * shows none, so the row falls back to its branch.
 */
export function shownPullRequest(checkout: Checkout): PullRequest | null {
  const pr = checkout.pull_request;
  const github = checkout.github;
  if (pr === null) return null;
  return !github || github.available || github.stale || github.unavailable_reason === null ? pr : null;
}

/** The checkout row's leading glyph: its pull request's mark, else what kind of checkout it is. */
export type CheckoutKind = "pull_request" | "folder" | "primary" | "detached" | "branch";

/** One checkout row of the Projects tab, mirrored from `SidebarCheckoutPresentation`. */
export type CheckoutPresentation = {
  kind: CheckoutKind;
  /** The glyph's color class when it is not a pull request's mark: danger for a missing folder, else subtle. */
  kindTone: string;
  /** GitHub could not be read again: the pull request's mark dims. */
  stale: boolean;
  /** The last commit's age; absent for a missing folder and until Git has been read. */
  age: string | null;
  /** Line two waits for Git to be read, so a row does not grow and shrink as the facts arrive. */
  secondLineReady: boolean;
  /** A merged or closed pull request's row is drawn dimmed. */
  settled: boolean;
  /** The row's screen-reader description: the pull request, the agents by state, and the branch and path. */
  detail: string;
  /** The pull request the row shows, whose glyph is a button and whose card has a header. */
  pullRequest: PullRequest | null;
};

export function checkoutPresentation(workspace: Workspace, checkout: Checkout, nowMs: number, t: TFunction<"translation">): CheckoutPresentation {
  const github = checkout.github;
  const pr = shownPullRequest(checkout);
  const primary = checkout.is_primary === true;
  const detached = checkout.worktree ? checkout.worktree.branch === null : false;
  // A folder that is gone draws its kind in danger rather than a pull request's mark.
  const kind: CheckoutKind = primary ? "primary" : pr && checkout.exists ? "pull_request" : !workspace.is_git ? "folder" : detached ? "detached" : "branch";
  const kindTone = !checkout.exists ? "text-destructive" : "text-subtle-foreground";
  const gitLoading = !!workspace.is_git && !checkout.worktree;
  const commitSeconds = checkout.worktree?.last_commit_unix_seconds;
  const age = checkout.exists && !gitLoading && commitSeconds != null ? relativeActivity(commitSeconds * 1000, nowMs, t) : null;
  const summary = checkout.agent_summary;
  const agentCount = checkout.agent_scope.badge_total;

  const lines: string[] = [];
  if (pr) {
    let first = `#${pr.number} · ${t(PR_LOOK[pr.state].word)}`;
    if (pr.title) first += ` · ${pr.title}`;
    const lastKnown = github?.stale ? activityAge(github.last_success_at_unix_ms, nowMs) : null;
    if (lastKnown) first += ` · ${t("card.lastKnown", { age: ageText(lastKnown, t) })}`;
    lines.push(first);
  }
  if (github?.unavailable_reason) lines.push(github.unavailable_reason);
  if (summary && agentCount > 0) {
    const counts = (
      [
        ["needs_you", summary.needs_you],
        ["done", summary.done],
        ["working", summary.working],
        ["seen", summary.seen],
      ] as const
    )
      .filter(([, count]) => count > 0)
      .map(([group, count]) => t("card.stateCount", { name: agentGroupTitle(group, t), count }))
      .join(" · ");
    lines.push(summary.unknown > 0 ? t("card.countsWithUnknown", { counts, unknown: summary.unknown }) : counts);
  }
  if (detached) lines.push(checkout.worktree?.head_sha ? t("card.detachedAt", { sha: checkout.worktree.head_sha }) : t("card.detached"));
  else if (checkout.branch) lines.push(checkout.branch);
  lines.push(checkout.path);

  return {
    kind,
    kindTone,
    stale: staleness(github)?.stale === true,
    age,
    secondLineReady: !gitLoading,
    settled: pr ? settled(pr.state) : false,
    detail: lines.join("\n"),
    pullRequest: pr,
  };
}

/** One line of the checkout card: a label and its value in a tone; the Agents row carries marks instead of a word. */
export type CheckoutCardRow =
  | { key: "review" | "checks"; label: string; value: string; tone: string }
  | { key: "branch" | "commit" | "base" | "changes"; label: string; value: string }
  | { key: "path"; label: string; value: string }
  | { key: "agents"; label: string; marks: MarkCounts };

/** The card a checkout row opens on hover and focus (PRD checkout-pr-glyph-card D-03, D-08, D-09, D-12). */
export type CheckoutCard = {
  /** A pull request's badge line, or the danger header of a folder that is gone; null on a plain checkout. */
  header:
    | { kind: "pull_request"; badge: ReturnType<typeof badgeWord>; state: PrState; number: number; url: string; title: string }
    | { kind: "missing"; label: string }
    | null;
  /** Each row only where its source has a value; a row with nothing to say takes no place. */
  rows: CheckoutCardRow[];
};

/**
 * Every value the card draws comes from the snapshot (design principle 10):
 * Review and Checks from the pull request, Branch from the worktree (a
 * detached HEAD names its short sha), Agents from the summary while any agent
 * runs, Commit from the last commit's age once Git has been read, Path always.
 * A folder that is gone has only its Path under a danger header.
 */
export function checkoutCard(workspace: Workspace, checkout: Checkout, nowMs: number, t: TFunction<"translation">): CheckoutCard {
  const rows: CheckoutCardRow[] = [];
  if (!checkout.exists) {
    rows.push({ key: "path", label: t("card.path"), value: checkout.path });
    return { header: { kind: "missing", label: t("graph.folderMissing") }, rows };
  }
  const pr = shownPullRequest(checkout);
  if (pr) rows.push(...pullRequestRows(pr, t));
  const worktree = checkout.worktree;
  if (workspace.is_git && worktree) {
    if (worktree.branch) rows.push({ key: "branch", label: t("card.branch"), value: worktree.branch });
    else rows.push({ key: "branch", label: t("card.branch"), value: worktree.head_sha ? t("card.detachedAt", { sha: worktree.head_sha.slice(0, 7) }) : t("card.detached") });
  }
  const summary = checkout.agent_summary;
  // Quiet agents count on no badge, so a checkout with only those has no row to show (status-marks B19).
  if (summary && checkout.agent_scope.badge_total > 0 && badgeParts(summary.marks).length > 0) rows.push({ key: "agents", label: t("overview.agents"), marks: summary.marks });
  const commitSeconds = worktree?.last_commit_unix_seconds;
  const age = activityAge(commitSeconds != null ? commitSeconds * 1000 : null, nowMs);
  if (age) rows.push({ key: "commit", label: t("card.commit"), value: age.unit === "now" ? t("common.now") : t("card.ago", { age: ageText(age, t) }) });
  rows.push({ key: "path", label: t("card.path"), value: checkout.path });
  return {
    header: pr ? pullRequestHeader(pr, t) : null,
    rows,
  };
}

/**
 * The PR card of a pull request on its own, for a PRs tool row whose branch
 * has no checkout here (PRD overview-lenses-prs B7): the same header and
 * Review and Checks rows the checkout card draws, and the branch.
 */
export function pullRequestCard(pr: PullRequest, t: TFunction<"translation">): CheckoutCard {
  const rows = pullRequestRows(pr, t);
  if (pr.head_branch) rows.push({ key: "branch", label: t("card.branch"), value: pr.head_branch });
  return { header: pullRequestHeader(pr, t), rows };
}

function pullRequestHeader(pr: PullRequest, t: TFunction<"translation">): NonNullable<CheckoutCard["header"]> {
  return { kind: "pull_request", badge: badgeWord(pr, t), state: pr.state, number: pr.number, url: pr.url, title: pr.title };
}

/** A pull request's Review and Checks rows, each once GitHub has answered it. */
function pullRequestRows(pr: PullRequest, t: TFunction<"translation">): CheckoutCardRow[] {
  const rows: CheckoutCardRow[] = [];
  const review = reviewWord(pr.review);
  if (review) rows.push({ key: "review", label: t("card.review"), value: t(review.key), tone: review.tone });
  const checks = checksWord(pr.checks);
  if (checks) rows.push({ key: "checks", label: t("card.checks"), value: t(checks.key), tone: checks.tone });
  return rows;
}

/** `↑N ↓N`, each part only above zero; empty when the branch is even with both. */
export function distanceText(ahead: number, behind: number): string {
  return [ahead > 0 ? `↑${ahead}` : null, behind > 0 ? `↓${behind}` : null].filter(Boolean).join(" ");
}

/** `1 file`, `N files`. */
export function filesText(count: number, t: TFunction<"translation">): string {
  return t("card.files", { count });
}

/**
 * The card an Agents graph box head opens (PRD agents-graph-view
 * B15): the checkout card with where the branch stands against its base
 * (`↑` commits of its own, `↓` behind its upstream) and how many files it
 * changed, each only once Git has said so. Lines added and removed are not
 * in the snapshot, so the card does not draw them (design 10).
 */
export function laneCheckoutCard(workspace: Workspace, checkout: Checkout, nowMs: number, t: TFunction<"translation">): CheckoutCard {
  const card = checkoutCard(workspace, checkout, nowMs, t);
  const worktree = checkout.worktree;
  if (!checkout.exists || !workspace.is_git || !worktree) return card;
  const extra: CheckoutCardRow[] = [];
  const ahead = checkout.ahead ?? 0;
  const behind = worktree.behind_upstream ?? 0;
  if (checkout.is_worktree && checkout.is_primary !== true && workspace.default_branch) {
    extra.push({ key: "base", label: t("card.base"), value: [workspace.default_branch, distanceText(ahead, behind)].filter(Boolean).join(" ") });
  }
  extra.push({ key: "changes", label: t("card.changes"), value: worktree.changed_file_count === 0 ? t("card.clean") : filesText(worktree.changed_file_count, t) });
  const at = card.rows.findIndex((row) => row.key === "branch");
  const rows = [...card.rows];
  rows.splice(at < 0 ? 0 : at + 1, 0, ...extra);
  return { ...card, rows };
}

/** A card with one value and no header is the plain text tooltip instead (D-09): that one value, never an empty card. */
export function cardSingleValue(card: CheckoutCard): string | null {
  if (card.header !== null || card.rows.length !== 1) return null;
  const row = card.rows[0]!;
  return row.key === "agents" ? null : row.value;
}

/** A sidebar activation opens agents unless this Workspace is already open and unfolded. */
export function checkoutRowExpansion(foldable: boolean, workspaceSelected: boolean, expanded: boolean): boolean | undefined {
  return foldable ? !(workspaceSelected && expanded) : undefined;
}

/** A project row opens its usable checkout, then folds if that target is already open and unfolded. */
export function projectRowExpansion(workspace: Workspace, target: Checkout | null, focusedCheckoutId: string | null, foldable: boolean): boolean | undefined {
  return checkoutRowExpansion(foldable, target !== null && target.id === focusedCheckoutId, projectExpanded(workspace));
}

/**
 * Whether a checkout row draws line two: only when it has something to say,
 * a purpose or the parent its agents were raised from, once Git has been read
 * (PRD sidebar-typography D-04). Agents alone never earn a line; the row's age
 * then ends line one.
 */
export function checkoutHasSecondLine(view: Pick<CheckoutPresentation, "secondLineReady">, purpose: string | null, raisedFrom: string | null): boolean {
  return view.secondLineReady && (purpose !== null || raisedFrom !== null);
}

/**
 * A checkout name read as its path prefix, up to and including the first
 * slash (`prd/`, `gen-prd/`), which the row mutes, and the rest (D-06). A
 * name without a slash, or with nothing on either side of it, has no prefix.
 */
export function checkoutNameParts(name: string): { prefix: string; rest: string } {
  const slash = name.indexOf("/");
  if (slash <= 0 || slash === name.length - 1) return { prefix: "", rest: name };
  return { prefix: name.slice(0, slash + 1), rest: name.slice(slash + 1) };
}
