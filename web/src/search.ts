import type { AgentScope } from "./agentScope";
// ⌘K's data (PRD cmdk-navigation): the snapshot rows the palette finds by
// name or #number and the entries it draws for what is in front. The entries,
// the fuzzy score, the number-first ranking and the grouping are pure
// functions, so the palette's behavior is testable without a browser; ⌘P's
// ranking happens in hided, beside the walk.

import { addressUrl } from "./browserViews";
import { checkoutPlaces } from "./navigation";
import { deviceConnected, frontDeviceId, localDeviceId, thisMachineKey } from "./devices";
import type { TFunction } from "i18next";
import { statusText } from "./agentStatus";
import type { MessageKey } from "./i18n/catalogs";
import { translate } from "./i18n/client";
import { herdrPaneId, projectsOf } from "./remote";
import { PR_LOOK } from "./prMark";
import { machineName, windowDeviceId, windowFirst } from "./screenMachine";
import type { AgentRow, Checkout, Device, GithubSearchResult, PrState, PullRequest, SessionWorkSearch, SnapshotRest, Task, WorkSession, Workspace } from "./snapshot";
import { workKey } from "./sessionWork";
import { askSubjectText } from "./askSubject";

/** The header an entry is drawn under: one per kind of thing; `label` is the catalog key the palette translates where it draws the header. */
export type SearchGroup = { id: string; label: MessageKey };

const ISSUES_GROUP: SearchGroup = { id: "issues", label: "overview.issues" };
const PULL_REQUESTS_GROUP: SearchGroup = { id: "pull-requests", label: "search.group.pullRequests" };
const AGENTS_GROUP: SearchGroup = { id: "agents", label: "overview.agents" };
const PROJECTS_GROUP: SearchGroup = { id: "projects", label: "overview.projects" };
const CHECKOUTS_GROUP: SearchGroup = { id: "checkouts", label: "search.group.checkouts" };
const DEVICES_GROUP: SearchGroup = { id: "devices", label: "search.group.devices" };
/** The one command ⌘K keeps. */
const COMMANDS_GROUP: SearchGroup = { id: "commands", label: "search.group.commands" };
export const GITHUB_GROUP: SearchGroup = { id: "github", label: "search.group.github" };
export const SESSIONS_GROUP: SearchGroup = { id: "sessions", label: "sessionWork.sessions" };
/** The Related list's own header; an entry in it keeps its kind's group for search. */
export const RELATED_GROUP: SearchGroup = { id: "related", label: "search.group.related" };
/** The empty query's second header: the checkouts last brought to the front (PRD cmdk-recent). */
export const RECENT_GROUP: SearchGroup = { id: "recent", label: "search.group.recent" };
/** How many Recent rows ⌘K shows; the core keeps ten so the exclusions still leave five. */
export const RECENT_SHOWN = 5;

type EntryKind = "agent" | "session" | "project" | "checkout" | "device" | "issue" | "pr" | "command";

/** A colour family for a state; the palette maps each to a token class. */
export type Tone = "working" | "attention" | "done" | "open" | "pending" | "failed" | "muted";

export type EntryStatus = { tone: Tone; label: string };

export type SearchEntry = {
  session?: WorkSession;
  id: string;
  title: string;
  subtitle: string;
  /** An agent's checkout, as `project › checkout`, which its detail names. */
  place?: string;
  kind: EntryKind;
  group: SearchGroup;
  /** The device the entry is on, which activating it brings forward; absent on a command. */
  deviceId?: string;
  /** The device's chip while the entry is not on the device in front (PRD home-device-rail B40). */
  chip?: { label: string; local: boolean };
  /** An agent's kind, which picks its mark. */
  agentKind?: string;
  /** The ids the entry activates: a pane, or a workspace/checkout pair. */
  paneId?: string;
  workspaceId?: string;
  checkoutId?: string;
  /** An issue's or pull request's number, which `#N` ranks first. */
  number?: number;
  taskKey?: string;
  /** Where a pull request or issue lives on GitHub; a result the core searched is opened there. */
  url?: string | null;
  /** `owner/repo` of a GitHub search result. */
  repository?: string;
  /** The rows the snapshot carried, which the detail reads facts from. */
  agent?: AgentRow;
  checkout?: Checkout;
  workspace?: Workspace;
  pr?: PullRequest;
  task?: Task;
  /** The state drawn at the row's end, when the snapshot names one. */
  status?: EntryStatus;
  /** A pull request's one state, which its mark draws (`PrState::of`). */
  prState?: PrState;
  /** The row of a relation list: how deep it nests and what it is to the thing in front. */
  depth?: number;
  tag?: "here" | "parent";
  /** The commands ⌘K keeps. */
  command?: "start_agent" | "open_url";
  /** A GitHub search result: its row is opened on GitHub, never in Hide. */
  external?: boolean;
  /** A Recent row of a device that is not connected: drawn dimmed, found by arrows, and ↵ does nothing (PRD cmdk-recent D-11). */
  dimmed?: boolean;
};

/** Local work-record results have their own group and never replace Agents. */
export function sessionEntries(answer: SessionWorkSearch | null, query: string): SearchEntry[] {
  if (!answer || answer.options?.request_id || answer.query !== query || answer.loading || answer.failure) return [];
  return answer.results.map((hit) => ({ id:`session:${workKey(hit.session)}`,title:hit.title,subtitle:hit.matched,place:hit.cwd ?? undefined,kind:"session",group:SESSIONS_GROUP,agentKind:hit.session.agent,deviceId:hit.session.device_id,session:hit.session }));
}

/** The fuzzy score of `query` against `candidate`, the same scorer hided's
 * index runs (`hided/src/index.rs`): characters in order, early and
 * adjacent matches higher, shorter candidates first on a tie. */
export function fuzzyScore(candidate: string, query: string): number | null {
  if (query.length === 0) return 0;
  const haystack = candidate;
  let cursor = 0;
  let score = 0;
  let previous = -1;
  for (const wanted of query) {
    const found = haystack.indexOf(wanted, cursor);
    if (found === -1) return null;
    score += 100 - Math.min(found, 90);
    if (previous !== -1 && previous + 1 === found) score += 35;
    if (found === 0 || "/_- .".includes(haystack[found - 1] ?? "")) score += 25;
    previous = found;
    cursor = found + 1;
  }
  score -= haystack.length;
  return score;
}

/** `Start an agent…` opens the start panel on every screen, and is how a browser tab reaches it, where ⌘N is the browser's (PRD home-device-rail D-21). */
function startAgentEntry(t: TFunction<"translation">): SearchEntry {
  return {
    id: "command:start-agent",
    title: t("search.startAgent"),
    subtitle: t("search.startAgentSubtitle"),
    kind: "command",
    group: COMMANDS_GROUP,
    command: "start_agent",
  };
}

/**
 * `Open URL in Browser`, offered while the query reads as a web address: one
 * written with http(s), or a loopback host a dev server runs on
 * (`localhost:5173`). The address is the one the View's address field would
 * load, so any other text stays a name to search. `unavailable` is why it
 * cannot run now; the row then stays listed, dimmed, with that reason.
 */
export function openUrlEntry(query: string, unavailable: string | null, t: TFunction<"translation">): SearchEntry | null {
  const text = query.trim();
  const url = addressUrl(text);
  if (!url || !(/^https?:\/\//i.test(text) || url.startsWith("http://"))) return null;
  return {
    id: "command:open-url",
    title: t("search.openUrl"),
    subtitle: unavailable ?? url,
    kind: "command",
    group: COMMANDS_GROUP,
    command: "open_url",
    url,
    dimmed: unavailable !== null,
  };
}


/** What ⌘K reads from one device: its name as this window calls it, its agents and its projects (not its Home, which the device entry stands for). `own` is this window's machine. */
export type SearchDevice = { agentScope: AgentScope | undefined; device: Device; name: string; own: boolean; agents: AgentRow[]; workspaces: Workspace[]; allWorkspaces: Workspace[]; local: boolean };

/** This machine and each connected device, in the rail's order; a device that is not connected has no current agents or projects to find. */
export function searchDevices(rest: SnapshotRest): SearchDevice[] {
  const rows: SearchDevice[] = [];
  // A snapshot that names no device is the core's own node alone, by the id it has before the snapshot names it.
  const listed = rest.navigator?.devices;
  const devices = listed?.length ? windowFirst(listed) : [{ id: localDeviceId(rest), label: null, kind: "local" } as Device];
  const own = windowDeviceId(listed);
  for (const device of devices) {
    const named = { name: listed?.length ? machineName(listed, device.id, translate) : translate(thisMachineKey(null)), own: device.id === own };
    if (device.id === localDeviceId(rest)) {
      const all = rest.navigator?.workspaces ?? [];
      rows.push({ agentScope: rest.navigator?.devices?.find((d) => d.id === device.id)?.agent_scope ?? rest.navigator?.agent_scope, device, ...named, agents: rest.navigator?.agents ?? [], workspaces: projectsOf(all), allWorkspaces: all, local: true });
      continue;
    }
    const status = rest.status?.remote?.find((row) => row.target_id === device.id);
    const session = status?.state === "connected" ? status.session : null;
    rows.push({ agentScope: device.agent_scope, device, ...named, agents: session?.agents ?? [], workspaces: projectsOf(session?.workspaces ?? []), allWorkspaces: session?.workspaces ?? [], local: false });
  }
  return rows;
}

function deviceChip(scope: SearchDevice, front: string): SearchEntry["chip"] {
  return scope.device.id === front ? undefined : { label: scope.name, local: scope.own };
}

/** An agent's state as the sidebar colours it (`chipTone`'s rules, as tones). */
function agentStatus(agent: Pick<AgentRow, "state" | "status_code">, t: TFunction<"translation">): EntryStatus {
  return { tone: agent.state.search_tone, label: statusText(t, agent.status_code) };
}

/**
 * An agent row: titled by the name every surface calls it, with its Herdr
 * name (`agent start --name`, `agent rename`), its place and its state
 * sentence under it, so a query finds it by any of them. A Herdr name the
 * title already reads as is not drawn twice.
 */
export function agentEntry(scope: SearchDevice, agent: AgentRow, place: string | null, front: string, t: TFunction<"translation">): SearchEntry {
  const sentence = agent.detail || (agent.ask_subject ? askSubjectText(t, agent.ask_subject) : statusText(t, agent.status_code));
  const name = agent.herdr_name && agent.herdr_name.toLowerCase() !== agent.identity_label.toLowerCase() ? agent.herdr_name : null;
  return {
    id: `agent:${agent.pane_id}`,
    title: agent.identity_label,
    subtitle: [name, place, sentence].filter(Boolean).join(" · "),
    place: place ?? undefined,
    kind: "agent",
    group: AGENTS_GROUP,
    agentKind: agent.agent_kind,
    paneId: agent.pane_id,
    deviceId: scope.device.id,
    chip: deviceChip(scope, front),
    agent,
    status: agentStatus(agent, t),
  };
}

function projectEntry(scope: SearchDevice, workspace: Workspace, front: string): SearchEntry {
  return {
    id: `project:${workspace.id}`,
    title: workspace.label,
    subtitle: workspace.path,
    kind: "project",
    group: PROJECTS_GROUP,
    workspaceId: workspace.id,
    deviceId: scope.device.id,
    chip: deviceChip(scope, front),
    workspace,
  };
}

/** A checkout row; `compact` names it by its branch alone, as a relation group's head does under its project. */
export function checkoutEntry(scope: SearchDevice, workspace: Workspace, checkout: Checkout, front: string, compact = false): SearchEntry {
  return {
    id: `checkout:${checkout.id}`,
    title: compact ? (checkout.branch ?? checkout.label) : `${workspace.label} / ${checkout.label}`,
    subtitle: compact ? workspace.label : checkout.path,
    kind: "checkout",
    group: CHECKOUTS_GROUP,
    workspaceId: workspace.id,
    checkoutId: checkout.id,
    deviceId: scope.device.id,
    chip: deviceChip(scope, front),
    workspace,
    checkout,
  };
}

/**
 * ⌘K's Recent rows (PRD cmdk-recent B1-B5, B9, B10): the core's recent
 * checkouts, newest first, without the ones in `shown` (the checkout in front
 * and every checkout Related lists), at most `RECENT_SHOWN`. A checkout of a
 * connected device is its live row, so a rename or a new branch reads as it is
 * now; one a connected catalog no longer lists is gone and is not drawn; one
 * of a device that is not connected is a dimmed row drawn from the names the
 * record kept, because that device has no catalog to read.
 */
export function recentEntries(rest: SnapshotRest | null, shown: ReadonlySet<string>): SearchEntry[] {
  const records = rest?.ui_state?.recent_checkouts;
  if (!rest || !records?.length) return [];
  const front = frontDeviceId(rest);
  const scopes = searchDevices(rest);
  const entries: SearchEntry[] = [];
  for (const record of records) {
    if (entries.length === RECENT_SHOWN) break;
    if (shown.has(record.checkout_id)) continue;
    const scope = scopes.find((candidate) => candidate.device.id === record.device_id);
    if (!scope) continue;
    if (!deviceConnected(rest, scope.device.id)) {
      entries.push({
        id: `checkout:${record.checkout_id}`,
        title: record.branch,
        subtitle: record.project_name,
        kind: "checkout",
        group: RECENT_GROUP,
        deviceId: record.device_id,
        checkoutId: record.checkout_id,
        chip: deviceChip(scope, front),
        dimmed: true,
      });
      continue;
    }
    for (const workspace of scope.workspaces) {
      const checkout = workspace.checkouts.find((candidate) => candidate.id === record.checkout_id);
      if (checkout) {
        entries.push({ ...checkoutEntry(scope, workspace, checkout, front, true), group: RECENT_GROUP });
        break;
      }
    }
  }
  return entries;
}

export function pullRequestEntry(scope: SearchDevice, workspace: Workspace, pr: PullRequest, front: string, t: TFunction<"translation">): SearchEntry {
  const state = t(PR_LOOK[pr.state].word);
  return {
    id: `pr:${workspace.id}:${pr.number}`,
    title: `#${pr.number} ${pr.title}`,
    subtitle: pr.head_branch ? t("search.prBranchSubtitle", { state, branch: pr.head_branch }) : t("search.prSubtitle", { state }),
    kind: "pr",
    group: PULL_REQUESTS_GROUP,
    workspaceId: workspace.id,
    deviceId: scope.device.id,
    chip: deviceChip(scope, front),
    number: pr.number,
    url: pr.url,
    workspace,
    pr,
    prState: pr.state,
    // The mark carries the colour; the word only names the state.
    status: { tone: "muted", label: state },
  };
}

export function issueEntry(scope: SearchDevice, workspace: Workspace, task: Task, front: string, t: TFunction<"translation">): SearchEntry {
  return {
    id: `issue:${task.key}`,
    title: `${task.id ? `${task.id} ` : ""}${task.title}`,
    subtitle: t("search.issueProject", { project: workspace.label }),
    kind: "issue",
    group: ISSUES_GROUP,
    workspaceId: workspace.id,
    deviceId: scope.device.id,
    chip: deviceChip(scope, front),
    number: taskNumber(task),
    taskKey: task.key,
    url: task.url,
    workspace,
    task,
    status: task.open ? { tone: "open", label: t("issue.state.open") } : { tone: "muted", label: t("issue.state.closed") },
  };
}

/** The number a task shows: `#N` and `owner/repo#N` end in it, a local issue's `L-N` too. */
function taskNumber(task: Task): number | undefined {
  const match = /(\d+)$/.exec(task.id ?? "");
  return match ? Number(match[1]) : undefined;
}

/** A GitHub search result as a row: opened on GitHub, whichever project it came from. */
function githubEntry(result: GithubSearchResult, t: TFunction<"translation">): SearchEntry {
  const pr = result.kind === "pr";
  const prState = pr ? result.pr_state ?? undefined : undefined;
  if (pr && !prState) throw new Error(`GitHub search result #${result.number} has no PR state`);
  const status: EntryStatus = prState
    ? { tone: "muted", label: t(PR_LOOK[prState].word) }
    : result.state === "open"
      ? { tone: "open", label: t("issue.state.open") }
      : { tone: "muted", label: t("issue.state.closed") };
  return {
    id: `github:${result.kind}:${result.repository}#${result.number}`,
    title: `#${result.number} ${result.title}`,
    subtitle: t(pr ? "search.githubPrSubtitle" : "search.githubIssueSubtitle", { repository: result.repository, state: status.label }),
    kind: pr ? "pr" : "issue",
    group: GITHUB_GROUP,
    number: result.number,
    url: result.url,
    repository: result.repository,
    external: true,
    prState,
    status,
  };
}

/**
 * The results the core searched, without the ones ⌘K already holds: a result
 * whose GitHub address a held issue or pull request has is that row, so it
 * shows once (PRD B17).
 */
export function githubEntries(results: readonly GithubSearchResult[], held: readonly SearchEntry[], t: TFunction<"translation">): SearchEntry[] {
  const known = new Set(held.map((entry) => entry.url).filter((url): url is string => Boolean(url)));
  return results.filter((result) => !known.has(result.url)).map((result) => githubEntry(result, t));
}

/**
 * The snapshot rows ⌘K searches: every device's agents, projects and
 * checkouts and the devices themselves (PRD home-device-rail D-16, B40), so a
 * pick on another device moves rail, sidebar and center there, and this Mac's
 * issues and pull requests the core already holds (PRD cmdk-navigation D-12,
 * D-14). A row not on the device in front carries that device's chip.
 */
export function searchEntries(rest: SnapshotRest | null, t: TFunction<"translation">): SearchEntry[] {
  if (!rest) return [];
  const entries: SearchEntry[] = [startAgentEntry(t)];
  const front = frontDeviceId(rest);
  const devices = searchDevices(rest);
  for (const scope of devices) {
    const places = checkoutPlaces(scope.allWorkspaces);
    for (const agent of scope.agents) entries.push(agentEntry(scope, agent, places.get(agent.pane_id) ?? null, front, t));
    for (const workspace of scope.workspaces) {
      entries.push(projectEntry(scope, workspace, front));
      for (const checkout of workspace.checkouts) entries.push(checkoutEntry(scope, workspace, checkout, front));
      if (!scope.local) continue;
      const seen = new Set<number>();
      for (const pr of [...(workspace.pull_requests ?? []), ...workspace.checkouts.flatMap((checkout) => (checkout.pull_request ? [checkout.pull_request] : []))]) {
        if (seen.has(pr.number)) continue;
        seen.add(pr.number);
        entries.push(pullRequestEntry(scope, workspace, pr, front, t));
      }
      for (const task of workspace.tasks?.tasks ?? []) entries.push(issueEntry(scope, workspace, task, front, t));
    }
  }
  // With this Mac alone there is no device to move to, so no device rows.
  for (const { device, name, own } of devices.length > 1 ? devices : []) {
    entries.push({
      id: `device:${device.id}`,
      title: name,
      subtitle: own ? t("common.thisDevice") : t("search.remoteDevice"),
      kind: "device",
      group: DEVICES_GROUP,
      deviceId: device.id,
    });
  }
  return entries;
}

/** The digits of `273` or a hash-prefixed `273`, which name an issue or a pull request, else null. */
export function numberQuery(query: string): number | null {
  const match = /^#?(\d{1,9})$/.exec(query.trim());
  return match ? Number(match[1]) : null;
}

/** Whether the core links the agent of `entry` to pull request `number` (`request.pull_requests`: its session made it or its branch has it), every one it links, a merged one included, which is more than the PR mark draws. */
function agentHasPr(entry: SearchEntry, number: number): boolean {
  return entry.kind === "agent" && (entry.agent?.request?.pull_requests ?? []).some((pr) => pr.number === number);
}

/** The agents whose pane's Herdr id holds `query`, in its case since Herdr tells `pB` from `pb`, the shortest id (the whole one) first. */
function paneIdMatches(entries: SearchEntry[], query: string): SearchEntry[] {
  return entries
    .flatMap((entry) => (entry.kind === "agent" && entry.paneId !== undefined ? [{ entry, id: herdrPaneId(entry.paneId) }] : []))
    .filter(({ id }) => id.includes(query))
    .sort((left, right) => left.id.length - right.id.length)
    .map(({ entry }) => entry);
}

/**
 * The entries matching `query`, best first. A query holding `:`, as every
 * Herdr pane id does (`w9J:p52`, as `$HERDR_PANE_ID` gives it or Copy name
 * and ID carries it), puts the agents whose id holds it ahead of everything, on every device;
 * a name with a colon (`fix: bug`) is held by no id, and without a colon no id
 * matches, so a name or a number never finds a pane by its id. A number (`273`, or with a hash) puts the
 * issues and then the pull requests numbered exactly that ahead of every title
 * match, each as its own row (PRD B12), then the agents the core links to a
 * pull request of that number; the other rows must hold the digits.
 */
export function filterEntries(entries: SearchEntry[], query: string, limit = 80): SearchEntry[] {
  const trimmed = query.trim();
  const needle = trimmed.toLowerCase();
  if (!needle) return entries.slice(0, limit);
  const number = numberQuery(needle);
  const exact = [
    ...(trimmed.includes(":") ? paneIdMatches(entries, trimmed) : []),
    ...(number === null ? [] : [...entries.filter((entry) => entry.kind === "issue" && entry.number === number), ...entries.filter((entry) => entry.kind === "pr" && entry.number === number), ...entries.filter((entry) => agentHasPr(entry, number))]),
  ];
  const taken = new Set(exact);
  const digits = number === null ? null : String(number);
  const scored = entries
    .filter((entry) => !taken.has(entry))
    .map((entry) => {
      const text = `${entry.title} ${entry.subtitle}`.toLowerCase();
      return { entry, score: digits !== null && !text.includes(digits) ? null : fuzzyScore(text, needle) };
    })
    .filter((row): row is { entry: SearchEntry; score: number } => row.score !== null);
  scored.sort((left, right) => right.score - left.score || left.entry.title.localeCompare(right.entry.title));
  return [...exact, ...scored.map((row) => row.entry)].slice(0, limit);
}

export type SearchSection = { group: SearchGroup; entries: SearchEntry[] };

/**
 * The ranked entries under their headers. A group stands where its best entry
 * ranked and its entries keep their rank, so the first row is still the best
 * match and grouping never reorders what `filterEntries` ranked inside a group.
 */
export function groupEntries(entries: SearchEntry[]): SearchSection[] {
  const sections = new Map<string, SearchSection>();
  for (const entry of entries) {
    const section = sections.get(entry.group.id);
    if (section) section.entries.push(entry);
    else sections.set(entry.group.id, { group: entry.group, entries: [entry] });
  }
  return [...sections.values()];
}
