// The PRs tool's list (PRD work-tab-pr-lens, sidebar-task-lens D-17): which pull requests
// a scope lists, in the groups and with the reasons the core decided
// (`hide-core/src/agent_state/work/board.rs`). This module only joins the
// core's rows with the pull requests and agents they name, and counts what is
// asked of the operator; it applies no grouping rule of its own.

import { scopeOccurrences, type PrBoard, type PrGroupId, type PrReason } from "./agentScope";
import { readsHere } from "./devices";
import { namedHost } from "./codeHost";
import { GH_REFUSAL_KEY, isGhRefusal, type GhRefusal } from "./githubFailure";
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

/** How code host reads stand for the projects a list gathers (B39, B42, B43). */
export type PrReading = (
  /** The host's program could not be used at all: not installed, too old or not signed in. */
  | { state: "blocked"; category: GhRefusal }
  /** No project of the scope has been answered yet. */
  | { state: "reading" }
  /** The last read failed; the list keeps the last values, `minutes` old, and `unread` names the extra questions that were also unanswered. */
  | { state: "stale"; minutes: number | null; unread: ExtraQuery[] }
  /** The list was read and current, but the extra questions in `unread` were not answered, so what depends on them is unknown. */
  | { state: "partial"; unread: ExtraQuery[] }
  | { state: "ready" }
) & {
  /** Each host that is refused, failed or read in part, with its own reading and how it is named, so one host's success never hides another's failure. Absent when every host is ready or still reading its first answer. */
  hosts?: HostReading[];
};

/** One host's own reading, named as its declaration names it (`HostDeclaration`). */
export type HostReading = { key: string; name: string; program: string; reading: PrReading };

/** A status without a declaration is from a core before hosts, which read GitHub alone. */
const UNDECLARED: Omit<HostReading, "reading"> = namedHost(null);

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

const EXTRA_QUERIES: readonly ExtraQuery[] = ["account", "review_requests", "merge_queue", "reviews"];

/**
 * The warning in the PRs tool while reading is not whole, or null when it
 * is: a failed read says how old the list is, and either state names the
 * extra questions that were not answered, in plain words, each host in its
 * own words; beside a host that answered, a refused host says so too. A
 * reading refused whole draws its own lines ([`blockedLines`]).
 */
export function readingWarning(reading: PrReading, t: TFunction<"translation">): string | null {
  if (reading.state === "blocked") return null;
  // A reading made without hosts is GitHub's, as before hosts.
  const lines = hostLines(reading.hosts ?? [{ ...UNDECLARED, reading }], t);
  return lines.length > 0 ? lines.join(" · ") : null;
}

/** The lines a reading refused before anything was read says: one per refused host. */
export function blockedLines(reading: PrReading & { state: "blocked" }, t: TFunction<"translation">): string[] {
  const refused = (reading.hosts ?? []).filter((host) => host.reading.state === "blocked");
  if (refused.length === 0) return [t(GH_REFUSAL_KEY[reading.category], { program: UNDECLARED.program })];
  return hostLines(refused, t);
}

/** Each listed host's line: a refusal names its program, a failed or partial read its host. */
export function hostLines(hosts: readonly HostReading[], t: TFunction<"translation">): string[] {
  return hosts.flatMap(({ name, program, reading }) => {
    if (reading.state === "blocked") return [t(GH_REFUSAL_KEY[reading.category], { program })];
    if (reading.state !== "stale" && reading.state !== "partial") return [];
    const names = reading.unread.map((query) => t(`work.lens.query.${query}`)).join(", ");
    if (reading.state === "partial") return [t("work.lens.partial", { host: name, names })];
    const age = reading.minutes === null ? t("work.lens.staleNow", { host: name }) : t("work.lens.stale", { host: name, minutes: reading.minutes });
    return [names ? `${age} · ${t("work.lens.unread", { names })}` : age];
  });
}

/**
 * Where GitHub reading stands over `projects`: a refusal of `gh` itself
 * speaks first, then a read that has not answered, then a failed read that
 * keeps its last values with their age.
 */
export function prReading(projects: readonly Workspace[], now: number): PrReading {
  return readingOfStatuses(projects.flatMap(projectStatuses), now);
}

/**
 * The same reading over the statuses a caller already holds, as the phone
 * gets them per project. Statuses are read per host, named by their
 * declaration (a status without one is GitHub's). The state is the reading
 * of the hosts that answered, as one host's was before hosts; `hosts` lists
 * every host refused, failed or read in part with its own reading, so one
 * host's success never hides another's refusal or first failure, in
 * whatever order the hosts come. A host still reading its first answer is
 * not listed: nothing can be done about it, and the list draws a reading
 * placeholder only while it is empty.
 */
export function readingOfStatuses(statuses: readonly GithubStatus[], now: number): PrReading {
  const byHost = new Map<string, { named: Omit<HostReading, "reading">; statuses: GithubStatus[] }>();
  for (const status of statuses) {
    const named = namedHost(status.host);
    const entry = byHost.get(named.key) ?? { named, statuses: [] };
    if (status.host) entry.named = named;
    entry.statuses.push(status);
    byHost.set(named.key, entry);
  }
  // Alone, a host keeps the reading one host had before hosts: a first
  // failure is still reading. Beside another, it is a failure of its own.
  const alone = byHost.size <= 1;
  const hosts: HostReading[] = [...byHost.values()]
    .sort((left, right) => left.named.key.localeCompare(right.named.key))
    .map(({ named, statuses: own }) => ({ ...named, reading: hostReadingOf(own, now, !alone) }));
  const listed = hosts.filter((host) => host.reading.state === "blocked" || host.reading.state === "stale" || host.reading.state === "partial");
  const withHosts = (reading: PrReading): PrReading => (listed.length > 0 ? { ...reading, hosts: listed } : reading);
  if (hosts.length === 0) return { state: "reading" };
  if (hosts.every((host) => host.reading.state === "blocked")) return withHosts(hosts[0]!.reading);
  const answered = new Set(hosts.filter((host) => host.reading.state !== "blocked" && host.reading.state !== "reading").map((host) => host.key));
  const own = statuses.filter((status) => answered.has(status.host?.key ?? UNDECLARED.key));
  return withHosts(answered.size > 0 ? hostReadingOf(own, now, false) : { state: "reading" });
}

/**
 * One host's reading. `firstFailure`: a failed read before any success
 * reads as failed with no age rather than as still reading, which is how a
 * host beside another that answered is told apart. A failed read is told by
 * its state, not by its optional category: an answer cut at the node's
 * byte cap carries none.
 */
function hostReadingOf(statuses: readonly GithubStatus[], now: number, firstFailure: boolean): PrReading {
  const answered = statuses.filter((status) => status.last_success_at_unix_ms != null);
  const refusal = statuses.map((status) => status.failure_category).find(isGhRefusal);
  if (refusal && answered.length === 0) return { state: "blocked", category: refusal };
  if (answered.length === 0) {
    const failed = statuses.some((status) => status.failure_category !== "no_github_remote" && (status.stale || status.failure_category != null));
    return firstFailure && failed ? { state: "stale", minutes: null, unread: [] } : { state: "reading" };
  }
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
