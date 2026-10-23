// The PRs tool's list (PRD work-tab-pr-lens, sidebar-task-lens D-17): which pull requests
// a scope lists, in the groups and with the reasons the core decided
// (`herdr-core/src/agent_state/work/board.rs`). This module only joins the
// core's rows with the pull requests and agents they name, and counts what is
// asked of the operator; it applies no grouping rule of its own.

import { scopeOccurrences, type PrBoard, type PrGroupId, type PrReason } from "./agentScope";
import { readsHere } from "./devices";
import type { TFunction } from "i18next";
import type { AgentRow, Device, ExtraQuery, GithubStatus, PullRequest, Workspace } from "./snapshot";

/** One listed pull request: the core's row with everything it names. */
export type PrEntry = {
  /** `<project id>#<number>`: a number alone repeats across the projects Home gathers. */
  key: string;
  project: Workspace;
  /** The device's name when the project is on another machine. */
  device: string | null;
  pr: PullRequest;
  row: PrBoard["rows"][number];
  reason: PrReason;
  agents: AgentRow[];
};

export type PrGroupView = {
  group: PrGroupId;
  /** The rows the group draws as rows. */
  entries: PrEntry[];
  /** The review asks of a team, folded into one line (D-15). */
  team: PrEntry[];
  /** The pull requests nothing has moved on for 7 days, folded into one line (D-11). */
  old: PrEntry[];
};

/** How GitHub reads stand for the projects a list gathers (B39, B42, B43). */
export type PrReading =
  /** `gh` could not be used at all: not installed or not signed in. */
  | { state: "blocked"; category: "not_installed" | "not_logged_in" }
  /** No project of the scope has been answered yet. */
  | { state: "reading" }
  /** The last read failed; the list keeps the last values, `minutes` old, and `unread` names the extra questions that were also unanswered. */
  | { state: "stale"; minutes: number | null; unread: ExtraQuery[] }
  /** The list was read and current, but the extra questions in `unread` were not answered, so what depends on them is unknown. */
  | { state: "partial"; unread: ExtraQuery[] }
  | { state: "ready" };

export type PrListModel = {
  groups: PrGroupView[];
  /** Needs You rows that are not folded: the PRs side of the Work count. */
  needsYou: PrEntry[];
  reading: PrReading;
  /** Projects of the scope that have a GitHub remote; none hides the PRs basis (B41). */
  projects: number;
  /** Every listed pull request, folded ones included. */
  total: number;
};

export type PrScopeProject = { workspace: Workspace; agents: AgentRow[]; device: string | null };

/** A project is Git with a GitHub remote, so it has pull requests to list. */
export function hasPullRequests(workspace: Workspace): boolean {
  return workspace.is_git === true && !workspace.checkouts.some((checkout) => checkout.github?.failure_category === "no_github_remote");
}

function projectStatuses(workspace: Workspace): GithubStatus[] {
  return workspace.checkouts.flatMap((checkout) => (checkout.github ? [checkout.github] : []));
}

const EXTRA_QUERIES: readonly ExtraQuery[] = ["account", "review_requests", "merge_queue"];

/**
 * The warning in the PRs tool while GitHub reading is not whole, or null when
 * it is: a failed read says how old the list is, and either state names the
 * extra questions that were not answered, in plain words.
 */
export function readingWarning(reading: PrReading, t: TFunction<"translation">): string | null {
  if (reading.state !== "stale" && reading.state !== "partial") return null;
  const names = reading.unread.map((query) => t(`work.lens.query.${query}`)).join(", ");
  if (reading.state === "partial") return t("work.lens.partial", { names });
  const age = reading.minutes === null ? t("work.lens.staleNow") : t("work.lens.stale", { minutes: reading.minutes });
  return names ? `${age} · ${t("work.lens.unread", { names })}` : age;
}

/**
 * Where GitHub reading stands over `projects`: a refusal of `gh` itself
 * speaks first, then a read that has not answered, then a failed read that
 * keeps its last values with their age.
 */
export function prReading(projects: readonly Workspace[], now: number): PrReading {
  const statuses = projects.flatMap(projectStatuses);
  const answered = statuses.filter((status) => status.last_success_at_unix_ms != null);
  const refusal = statuses.find((status) => status.failure_category === "not_installed" || status.failure_category === "not_logged_in");
  if (refusal && answered.length === 0) return { state: "blocked", category: refusal.failure_category as "not_installed" | "not_logged_in" };
  if (answered.length === 0) return { state: "reading" };
  const failed = answered.some((status) => status.stale === true || (status.failure_category != null && !status.available));
  const unread = EXTRA_QUERIES.filter((query) => answered.some((status) => status.unread_queries?.includes(query)));
  if (failed) {
    const oldest = Math.min(...answered.map((status) => status.last_success_at_unix_ms!));
    return { state: "stale", minutes: Math.max(0, Math.floor((now - oldest) / 60_000)), unread };
  }
  return unread.length > 0 ? { state: "partial", unread } : { state: "ready" };
}

/**
 * The list of `projects`, in the core's groups. `only` is the checkout the
 * scope chip narrows to (D-19): the pull requests that checkout's sessions
 * made or worked on, as the core named them (`checkout_ids`).
 */
export function prListModel(projects: readonly PrScopeProject[], only: string | null, now: number): PrListModel {
  const scoped = projects.filter((project) => hasPullRequests(project.workspace));
  const byGroup = new Map<PrGroupId, PrGroupView>();
  const needsYou: PrEntry[] = [];
  let total = 0;
  for (const { workspace, agents, device } of scoped) {
    const board = workspace.agent_scope.prs;
    const pulls = new Map((workspace.pull_requests ?? []).map((pr) => [pr.number, pr]));
    for (const group of board.groups) {
      const view = byGroup.get(group.group) ?? { group: group.group, entries: [], team: [], old: [] };
      byGroup.set(group.group, view);
      for (const number of group.numbers) {
        const row = board.rows.find((candidate) => candidate.number === number);
        const pr = pulls.get(number);
        if (!row || !pr) throw new Error(`Core PR row without its pull request: ${workspace.id}#${number}`);
        if (only !== null && !row.checkout_ids.includes(only)) continue;
        const entry: PrEntry = { key: `${workspace.id}#${number}`, project: workspace, device, pr, row, reason: row.reason, agents: scopeOccurrences(row.agents, agents) };
        total += 1;
        if (row.fold === "team") view.team.push(entry);
        else if (row.fold === "old") view.old.push(entry);
        else view.entries.push(entry);
        if (group.group === "needs_you" && row.fold === null) needsYou.push(entry);
      }
    }
  }
  const order: PrGroupId[] = ["needs_you", "working", "blocked", "waiting", "drafts", "bots", "merged"];
  const groups = order.flatMap((group) => {
    const view = byGroup.get(group);
    return view && view.entries.length + view.team.length + view.old.length > 0 ? [view] : [];
  });
  return { groups, needsYou, reading: prReading(scoped.map((project) => project.workspace), now), projects: scoped.length, total };
}

/** Whether the card can be opened for this project by this core; another machine's project goes to GitHub. */
export function opensCard(workspace: Workspace, devices: readonly Device[] | null | undefined): boolean {
  return readsHere(workspace, devices);
}

/** The groups that start folded (B7): everything after Needs You and Agent is working and Blocked. */
export const FOLDED_GROUPS: ReadonlySet<PrGroupId> = new Set(["waiting", "drafts", "bots", "merged"]);

/** The entry of one pull request wherever the core placed it, for a card opened on a PR the list may not show. */
export function findPrEntry(scope: PrScopeProject, number: number): PrEntry | null {
  const [model] = [prListModel([scope], null, Date.now())];
  for (const group of model.groups) {
    for (const entry of [...group.entries, ...group.team, ...group.old]) if (entry.pr.number === number) return entry;
  }
  return null;
}
