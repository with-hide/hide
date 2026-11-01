import { LOCAL } from "./legacyAgentRow";
import { legacyWork } from "./legacyRequestWork";
import { legacyRelations } from "./legacyRelations";
import * as beforeClose from "./legacyClose";
import * as drawClose from "../src/close";
import type { CloseScope, CloseSubtree } from "../src/agentScope";
import type { PaneRow } from "../src/snapshot";
import { projectRows as drawProjectRows, checkoutPresentation as drawCheckoutPresentation, checkoutCard as drawCheckoutCard } from "../src/projects";
import { projectListNumbers as drawProjectNumbers } from "./legacyProjectNumbers";
import { checkoutAgentRows as beforeCheckoutAgentRows } from "./legacyAgentTree";
import type { BoardRow } from "../src/projectBoard";
import { legacyAgentRow } from "./legacyAgentRow";
// Frozen scope fixture adapter from main 9f144877. Test data only.
// Assertions keep the old screen values; production reads the core wire.
import type { AgentScope } from "../src/agentScope";
import type { RowWork } from "./legacyRequestWork";
import type { AgentRow, Workspace, Checkout, RequestVerb, SnapshotRest } from "../src/snapshot";
import type { BoardProject } from "../src/projectBoard";
import { scopeAgents as drawScopeAgents, type LensAgent } from "../src/overviewLens";
import { requestRows as drawRequestRows, requestGroups as drawRequestGroups, type RequestRow } from "./legacyRequestList";

const verbs: RequestVerb[] = ["answer", "blocked", "fix", "review", "stopped", "result", "working", "waiting", "idle"];
const todo = verbs.slice(0, 6);
/** The frozen scope with the retired `work` map the legacy request list still reads. */
export type LegacyScope = AgentScope & { work: Record<string, RowWork> };
export function emptyScope(): AgentScope {
  return { overview_needs_you: 0, has_working: false, relations: {}, listed: [], places: {}, places_live: false, closes: {}, raised: [], owners: {}, badge_total: 0, prs: { rows: [], groups: [], needs_you: 0 }, rows: [], group_rows: [], descendants: {}, children: {}, folded: {}, tree: { rows: [], visible_rows: [], shown: [], more: 0, needs_you: false, turn_kind: null }, sidebar_tree: { rows: [], visible_rows: [], shown: [], more: 0, needs_you: false, turn_kind: null }, global_tree: { rows: [], visible_rows: [], shown: [], more: 0, needs_you: false, turn_kind: null }, total: 0, overview_total: 0, roots: [], groups: { needs_you: 0, done: 0, working: 0, seen: 0 }, marks: { turn: 0, going: 0, done: 0, unread: 0 }, sections: [], members: [], buckets: { turn: 0, working: 0, delegating: 0, resting: 0 }, turns: { question: 0, approval: 0, error: 0, done: 0 }, requests: { rows: [], groups: [], counts: Object.fromEntries(verbs.map((verb) => [verb, 0])) as Record<RequestVerb, number>, todo: 0, answer: 0 } };
}

export function legacyScope(lens: readonly LensAgent[], all: readonly AgentRow[] = lens.map((l) => l.agent), workers: ReadonlySet<string> = new Set()): LegacyScope {
  const scope: LegacyScope = { ...emptyScope(), work: {} };
  scope.members = lens.map(({ agent, project, checkout }) => ({ pane_id: agent.pane_id, project_id: project.id, checkout_id: checkout.id }));
  scope.overview_total = lens.length;
  const inScope = new Set(lens.map((l) => l.agent.pane_id));
  const byPane = new Map(all.map((a) => [a.pane_id, a]));
  lens.forEach(({ agent }, member) => {
    const bucket = agent.group === "needs_you" || agent.group === "done" ? "turn" : agent.wait ? "delegating" : agent.group === "working" ? "working" : "resting";
    scope.buckets[bucket]++;
    if (agent.group === "done") scope.turns.done++;
    if (agent.group === "needs_you" && ["question", "approval", "error"].includes(agent.demand ?? "")) scope.turns[agent.demand as "question" | "approval" | "error"]++;
    if (agent.delegated && agent.lineage_parent_pane_id && inScope.has(agent.lineage_parent_pane_id)) return;
    if (workers.has(agent.pane_id)) return;
    scope.requests.rows.push({ member, children: (agent.close_descendant_pane_ids ?? []).filter((id) => byPane.has(id)).reverse() });
  });
  const agentAt = (index: number) => lens[scope.requests.rows[index]!.member]!.agent;
  for (const verb of verbs) {
    const rows = scope.requests.rows.map((_, i) => i).filter((i) => (agentAt(i).request?.verb ?? (agentAt(i).group === "working" ? "working" : "idle")) === verb);
    rows.sort(todo.includes(verb) ? (a, b) => (agentAt(a).request?.verb_since_unix_ms ?? Number.MAX_SAFE_INTEGER) - (agentAt(b).request?.verb_since_unix_ms ?? Number.MAX_SAFE_INTEGER) : (a, b) => (agentAt(b).last_activity ?? "").localeCompare(agentAt(a).last_activity ?? ""));
    if (rows.length) scope.requests.groups.push({ verb, rows });
    scope.requests.counts[verb] = rows.length;
    if (todo.includes(verb)) scope.requests.todo += rows.length;
  }
  scope.requests.answer = scope.requests.counts.answer;
  return scope;
}

export function legacyProject<T extends Workspace>(project: T, agents: readonly AgentRow[]): T {
  agents = agents.map((agent) => agent.state ? agent : legacyAgentRow(agent));
  const owners = new Map<string, Checkout>();
  for (const checkout of project.checkouts) for (const tab of checkout.tabs ?? []) for (const pane of tab.panes) if (!owners.has(pane.id)) owners.set(pane.id, checkout);
  const seen = new Set<string>();
  const members: LensAgent[] = [];
  for (const agent of agents) {
    const checkout = owners.get(agent.pane_id);
    if (!checkout || seen.has(agent.pane_id)) continue;
    seen.add(agent.pane_id);
    members.push({ agent, checkout, project, device: null, bucket: agent.state?.bucket ?? "resting", task: (project.tasks?.tasks ?? []).find(t => t.key === checkout.task_key) ?? null });
  }
  const scope = legacyScope(members, agents);
  for (const lens of members) scope.work[lens.agent.pane_id] = legacyWork(lens);
  scope.places_live = true;
  for (const checkout of project.checkouts) for (const tab of checkout.tabs ?? []) for (const pane of tab.panes) scope.places[pane.id] ??= { project_id: project.id, checkout_id: checkout.id, kind: project.is_home ? "home" : !project.is_git && project.checkouts.length === 1 ? "folder" : "checkout" };
  const physical = agents.filter((agent) => owners.has(agent.pane_id));
  physicalScope(scope, physical);
  for (const checkout of project.checkouts) {
    const marks = checkout.agent_summary?.marks;
    if (marks) for (const key of Object.keys(scope.marks) as (keyof AgentScope["marks"])[]) scope.marks[key] += marks[key];
  }
  const treeProject = { ...project, checkouts: project.checkouts.map((c) => ({ ...c, tabs: c.tabs ?? [] })) };
  const trees = beforeCheckoutAgentRows(treeProject, [...agents]);
  const checkouts = project.checkouts.map((checkout) => {
    const physical = agents.filter((agent) => (checkout.tabs ?? []).some((tab) => tab.panes.some((p) => p.id === agent.pane_id)));
    const value = legacyPhysicalScope(physical);
    value.marks = checkout.agent_summary?.marks ?? emptyScope().marks;
    value.has_working = (checkout.agent_summary?.working ?? 0) > 0;
    value.badge_total = legacyBadgeTotal(checkout);
    value.tree = legacyTree(trees.get(checkout.id) ?? [], agents);
    value.global_tree = value.tree;
    return { ...checkout, agent_scope: value };
  });
  return { ...project, inactive_checkouts: project.inactive_checkouts ?? { expanded: false, checkout_ids: [] }, checkouts, agent_scope: scope };
}

function physicalScope(scope: AgentScope, agents: readonly AgentRow[]) {
  scope.rows = legacyRefs(agents, agents);
  scope.total = agents.length;
  scope.overview_needs_you = agents.filter(a => a.group === "needs_you").length;
  scope.roots = legacyRefs(agents.filter((a) => !a.delegated), agents);
  // Session-first-ui B1/B7 fixture wire: roots only, default-open checkouts.
  const sidebar = legacyRefs(agents.filter((a) => !a.delegated && !a.resolved), agents).map((row) => ({ ...row, depth: 0 }));
  scope.sidebar_tree = { rows: sidebar, visible_rows: sidebar, shown: sidebar.slice(0, 2).map((row) => row.pane_id), more: Math.max(0, sidebar.length - 2), needs_you: agents.some((a) => !a.delegated && a.group === "needs_you"), turn_kind: null };
  for (const agent of agents) if (agent.group in scope.groups) scope.groups[agent.group as keyof AgentScope["groups"]]++;
  const byPane = new Map(agents.map((a) => [a.pane_id, a]));
  for (const agent of agents) {
    const queue = [...(agent.lineage_child_pane_ids ?? [])], seen = new Set<string>();
    while (queue.length) {
      const id = queue.shift()!;
      if (id === agent.pane_id || seen.has(id)) continue;
      const child = byPane.get(id); if (!child) continue;
      seen.add(id); queue.push(...(child.lineage_child_pane_ids ?? []));
    }
    scope.descendants[agent.pane_id] = seen.size;
    scope.children[agent.pane_id] = (agent.lineage_child_pane_ids ?? []).filter((id) => byPane.has(id));
  }
  const names = [...new Set(["needs_you", "done", "working", "seen", ...agents.map((a) => a.group)])];
  for (const group of names) {
    const members = agents.filter((a) => a.group === group);
    if (members.length) scope.group_rows.push({ group, rows: legacyRefs(members, agents) });
    const roots = members.filter((a) => !a.delegated), rows: AgentScope["sections"][number]["rows"] = [];
    const visit = (agent: AgentRow, depth: number, seen: Set<string>) => {
      if (seen.has(agent.pane_id)) return;
      seen.add(agent.pane_id); rows.push({ ...legacyRefs([agent], agents)[0]!, depth, descendants: scope.descendants[agent.pane_id]! });
      if (agent.lineage_collapsed !== false) return;
      for (const id of agent.lineage_child_pane_ids ?? []) { const child = byPane.get(id); if (child) visit(child, depth + 1, seen); }
    };
    for (const root of roots) visit(root, 0, new Set());
    if (roots.length) scope.sections.push({ group, rows, count: roots.reduce((n, a) => n + 1 + scope.descendants[a.pane_id]!, 0) });
  }
}

export function scopeAgents(projects: readonly BoardProject[]): LensAgent[] {
  return projects.flatMap((p) => {
    const agents = p.agents.map((a) => a.state ? a : legacyAgentRow(a));
    return drawScopeAgents([{ ...p, agents, workspace: legacyProject(p.workspace, agents) }]).map((row) => ({ ...row, project: p.workspace }));
  });
}
const scopeOfRows = new WeakMap<readonly RequestRow[], AgentScope>();
export function requestRows(agents: readonly LensAgent[], all: readonly AgentRow[], workers: ReadonlySet<string> = new Set()): RequestRow[] {
  const scope = legacyScope(agents, all, workers);
  const rows = drawRequestRows(agents, all, scope);
  scopeOfRows.set(rows, scope);
  return rows;
}
export function requestScope(rows: readonly RequestRow[]): AgentScope {
  return scopeOfRows.get(rows) ?? legacyScope(rows.map((row) => row.lens));
}
export function requestGroups(rows: readonly RequestRow[]) { return drawRequestGroups(rows, requestScope(rows)); }

export function legacyRest(rest: SnapshotRest, agents: AgentRow[]): SnapshotRest {
  const localId = rest.navigator?.devices?.find((d) => d.kind !== "remote")?.id ?? "";
  // The core names each row's machine by the session it publishes the row in.
  // A fixture row left on the default machine is the snapshot's own machine's,
  // and every row a device's session holds is that device's.
  const onLocal = (rows: readonly AgentRow[] | undefined) => { for (const row of rows ?? []) if (row.device_id === undefined || row.device_id === LOCAL) row.device_id = localId; };
  onLocal(agents);
  onLocal(rest.navigator?.agents);
  for (const remote of rest.status?.remote ?? []) for (const row of remote.session?.agents ?? []) row.device_id = remote.target_id;
  const projects = (rest.navigator?.workspaces ?? []).map((p) => legacyProject(p, agents));
  const remotes = (rest.status?.remote ?? []).map((r) => ({ ...r, session: r.session ? { ...r.session, workspaces: (r.session.workspaces ?? []).map((p) => legacyProject(p, r.session!.agents ?? [])) } : r.session }));
  const deviceScopes = new Map<string, AgentScope>();
  const local = legacyScope(scopeAgents(projects.filter((p) => !p.is_home).map((workspace) => ({ workspace, agents, device: null }))), agents);
  physicalScope(local, agents);
  deviceScopes.set(localId, local);
  for (const remote of remotes) {
    const all = remote.session?.agents ?? [];
    const members = scopeAgents((remote.session?.workspaces ?? []).filter((p) => !p.is_home).map((workspace) => ({ workspace, agents: all, device: remote.target_id })));
    const scope = legacyScope(members, all);
    physicalScope(scope, remote.state === "connected" ? all : []);
    deviceScopes.set(remote.target_id, scope);
  }
  const liveWorkspaces = [...projects, ...remotes.filter((r) => r.state === "connected").flatMap((r) => r.session?.workspaces ?? [])];
  const localRows = agents.map((a) => ({ ...a, device_id: localId, device_label: rest.navigator?.devices?.find((d) => d.id === localId)?.label ?? undefined }));
  const liveRows = [...localRows, ...remotes.filter((r) => r.state === "connected").flatMap((r) => (r.session?.agents ?? []).map((a) => ({ ...a, device_id: r.target_id, device_label: rest.navigator?.devices?.find((d) => d.id === r.target_id)?.label ?? r.target_id })))];
  for (const project of liveWorkspaces) {
    const trees = beforeCheckoutAgentRows(project, liveRows);
    for (const checkout of project.checkouts) checkout.agent_scope.global_tree = legacyTree(trees.get(checkout.id) ?? [], liveRows);
  }
  const listed = (rows: readonly AgentRow[], id: string, label: string | null, remote: boolean) => rows.map((a, index) => ({ pane_id: a.pane_id, device_id: id, device_label: label, remote, index }));
  local.listed = listed(agents, localId, rest.navigator?.devices?.find((d) => d.id === localId)?.label ?? null, false);
  local.places_live = true;
  const assignPlaces = (projects: Workspace[], scope: AgentScope, live: boolean) => {
    for (const project of projects) for (const [id, place] of Object.entries(project.agent_scope.places)) scope.places[id] ??= place;
    for (const project of projects) { project.agent_scope.places = Object.fromEntries(Object.entries(scope.places).filter(([, place]) => place.project_id === project.id)); project.agent_scope.places_live = live; }
    scope.places_live = live;
  };
  assignPlaces(projects, local, true);
  local.relations = legacyRelations(projects, agents.map((a) => a.state ? a : legacyAgentRow(a)));
  for (const remote of remotes) {
    const scope = deviceScopes.get(remote.target_id)!;
    scope.listed = remote.state === "connected" ? listed(remote.session?.agents ?? [], remote.target_id, rest.navigator?.devices?.find((d) => d.id === remote.target_id)?.label ?? remote.target_id, true) : [];
    assignPlaces(remote.session?.workspaces ?? [], scope, remote.state === "connected");
    scope.relations = legacyRelations(remote.session?.workspaces ?? [], (remote.session?.agents ?? []).map((a) => a.state ? a : legacyAgentRow(a)));
  }
  const overall = legacyPhysicalScope(liveRows);
  overall.listed = [...local.listed, ...remotes.flatMap((r) => deviceScopes.get(r.target_id)!.listed)];
  overall.closes = legacyConsequences(projects, agents, liveRows);
  for (const remote of remotes) Object.assign(overall.closes, legacyConsequences(remote.session?.workspaces ?? [], remote.session?.agents ?? [], liveRows));
  overall.folded = foldedScopes(liveRows, liveWorkspaces);
  Object.assign(local, legacyRaised(projects, liveRows.map((agent) => ({ agent, device: null }))));
  for (const remote of remotes) Object.assign(deviceScopes.get(remote.target_id)!, legacyRaised(remote.session?.workspaces ?? [], liveRows.map((agent) => ({ agent, device: null }))));
  local.folded = foldedScopes(localRows, liveWorkspaces);
  for (const remote of remotes) deviceScopes.get(remote.target_id)!.folded = foldedScopes((remote.session?.agents ?? []).map((a) => ({ ...a, device_id: remote.target_id, device_label: rest.navigator?.devices?.find((d) => d.id === remote.target_id)?.label ?? remote.target_id })), liveWorkspaces);
  return { ...rest, navigator: { ...rest.navigator, agent_scope: overall, workspaces: projects, devices: rest.navigator?.devices?.map((d) => ({ ...d, agent_scope: deviceScopes.get(d.id) ?? emptyScope() })) }, status: { ...rest.status, remote: remotes } };
}

export function legacyPhysicalScope(agents: readonly AgentRow[]): AgentScope {
  const scope = emptyScope();
  physicalScope(scope, agents);
  return scope;
}

import { foldedLineage as beforeFoldedLineage } from "./legacyFoldedLineage";
import { foldedLineage as drawFoldedLineage } from "../src/lineageSummary";
import { agentTree as drawAgentTree, groupCounts as drawGroupCounts, agentSections as drawAgentSections, liveDescendantCounts as drawDescendantCounts, type ListedAgent } from "../src/navigation";

function foldedScopes(agents: AgentRow[], workspaces: Workspace[]): AgentScope["folded"] {
  return Object.fromEntries(agents.map((agent) => {
    const value = beforeFoldedLineage(agent, agents, workspaces);
    return [agent.pane_id, {
      tiers: value.lines.map((line) => [{ key: line.key, candidates: [line.agent.pane_id], branch: line.branch, pull_request: line.pullRequest, device_id: line.deviceId }]),
      overflow: value.overflow, badge_descendants: value.badgeDescendants, badge_counts: value.badgeCounts, badge_children: value.badgeChildren.map((a) => a.pane_id),
    }];
  }));
}
export function foldedLineage(parent: AgentRow, agents: AgentRow[], workspaces: Workspace[]) {
  const scope = emptyScope(); scope.folded = foldedScopes(agents, workspaces);
  return drawFoldedLineage(parent, agents, scope);
}
export function agentTree(listed: ListedAgent[]) { return drawAgentTree(listed, legacyPhysicalScope(listed.map((r) => r.agent))); }
export function groupCounts(agents: AgentRow[]) { return drawGroupCounts(legacyPhysicalScope(agents)); }
export function agentSections(agents: AgentRow[]) { return drawAgentSections(legacyPhysicalScope(agents), agents); }
export function liveDescendantCounts(agents: AgentRow[]) { return drawDescendantCounts(legacyPhysicalScope(agents)); }

export function legacyTree(rows: readonly BoardRow[], source: readonly AgentRow[] = rows.map(r => r.agent)): AgentScope["tree"] {
  let foldedAt: number | null = null;
  const visible = rows.filter((row) => {
    if (foldedAt !== null && row.depth > foldedAt) return false;
    foldedAt = row.agent.lineage_collapsed === false ? null : row.depth;
    return true;
  });
  const ranked = [...rows].sort((a,b) => a.agent.state.attention_rank - b.agent.state.attention_rank);
  const needs = rows.some((r) => r.agent.group === "needs_you" || (r.depth === 0 && r.agent.group === "done"));
  const ids = (rows: readonly BoardRow[]) => rows.map((r) => ({ ...legacyRefs([r.agent], source)[0]!, depth: r.depth }));
  return { rows: ids(rows), visible_rows: ids(visible), shown: ranked.slice(0,2).map((r) => r.agent.pane_id), more: Math.max(0, ranked.length-2), needs_you: needs, turn_kind: needs ? (rows.some((r) => r.agent.group === "needs_you") ? "question" : "review") : null };
}

function legacyRaised(projects: readonly Workspace[], listed: readonly ListedAgent[]) {
  const drawn = new Set<string>(), owners: Record<string, string> = {};
  for (const project of projects) for (const checkout of project.checkouts) for (const tab of checkout.tabs ?? []) for (const pane of tab.panes) {
    drawn.add(pane.id);
    if (!project.is_home && owners[pane.id] === undefined) owners[pane.id] = checkout.id;
  }
  const raised: AgentScope["raised"] = [];
  for (const [group, cap] of [["needs_you", 5], ["done", 3]] as const) {
    const ids = legacyRefs(listed.filter((r) => r.agent.group === group && drawn.has(r.agent.pane_id)).map(r => r.agent), listed.map(r => r.agent));
    if (ids.length) raised.push({ group, shown: ids.slice(0, cap), more: ids.slice(cap) });
  }
  return { raised, owners };
}
export function projectRows(workspaces: Workspace[], groups: Parameters<typeof drawProjectRows>[1], listed: ListedAgent[], home: Workspace | null = null, open: readonly string[] = []) {
  return drawProjectRows(workspaces, groups, listed, { ...emptyScope(), ...legacyRaised(home ? [...workspaces, home] : workspaces, listed) }, open);
}
export function projectListNumbers(numbers: Parameters<typeof drawProjectNumbers>[0], rows: Parameters<typeof drawProjectNumbers>[1], workspaces: readonly Workspace[]) {
  return drawProjectNumbers(numbers, rows, { ...emptyScope(), ...legacyRaised(workspaces, []) });
}
function legacyBadgeTotal(checkout: Checkout) {
  const s = checkout.agent_summary;
  return s ? s.needs_you + s.done + s.working + s.seen : 0;
}
export function checkoutPresentation(project: Workspace, checkout: Checkout, ...args: [number, Parameters<typeof drawCheckoutPresentation>[3]]) {
  return drawCheckoutPresentation(project, { ...checkout, agent_scope: { ...emptyScope(), badge_total: legacyBadgeTotal(checkout) } }, ...args);
}
export function checkoutCard(project: Workspace, checkout: Checkout, ...args: [number, Parameters<typeof drawCheckoutCard>[3]]) {
  return drawCheckoutCard(project, { ...checkout, agent_scope: { ...emptyScope(), badge_total: legacyBadgeTotal(checkout) } }, ...args);
}

export function legacyCloseScope(panes: readonly PaneRow[], host: readonly AgentRow[], all: readonly AgentRow[] = host): CloseScope {
  const stop = beforeClose.stopWorkOf(panes, host);
  const tree = (value: beforeClose.Subtree | null): CloseSubtree | null => value ? {
    ids: value.ids, rows: value.rows.map((r) => ({ pane_id: r.agent.pane_id, depth: r.depth, target: r.target, state: r.state })), counts: value.counts, unknown: value.unknown, target_unknown: value.targetUnknown,
  } : null;
  return { decision: beforeClose.closeDecision([...panes], [...host]), stop_work: { rows: stop.rows.map((r) => ({ pane_id: r.pane.id, agent: r.agent !== null, label: r.label, state: r.state })), unknown: stop.unknown === null ? null : stop.rows.indexOf(stop.unknown) }, subtree: tree(beforeClose.subtreeOf(panes.map((p) => p.id), all)), subtree_all: tree(beforeClose.subtreeOf(panes.map((p) => p.id), all, { everyTarget: true })) };
}
function legacyConsequences(projects: Workspace[], host: AgentRow[], all: AgentRow[]) {
  const result: AgentScope["closes"] = {};
  const add = (panes: PaneRow[]) => { result[panes.map((p) => p.id).join("\0")] ??= legacyCloseScope(panes, host, all); };
  add([]);
  for (const p of projects) {
    for (const c of p.checkouts) {
      for (const t of c.tabs ?? []) { for (const pane of t.panes) add([pane]); add(t.panes); }
      add((c.tabs ?? []).flatMap((t) => t.panes));
    }
    add(p.checkouts.flatMap((c) => c.tabs ?? []).flatMap((t) => t.panes));
  }
  return result;
}
export function closeDecision(panes: PaneRow[], agents: AgentRow[]) { return drawClose.closeDecision(legacyCloseScope(panes, agents)); }
export function stopWorkOf(panes: readonly PaneRow[], agents: readonly AgentRow[]) { return drawClose.stopWorkOf(panes, agents, legacyCloseScope(panes, agents)); }
export function subtreeOf(inside: readonly string[], agents: readonly AgentRow[], options: { everyTarget?: boolean } = {}) {
  const panes = inside.map((id) => ({ id })) as PaneRow[];
  return drawClose.subtreeOf(legacyCloseScope(panes, agents), agents, options);
}
export function closeSheet(panes: readonly PaneRow[], host: readonly AgentRow[], all: readonly AgentRow[]) { return drawClose.closeSheet(panes, host, all, legacyCloseScope(panes, host, all)); }


function legacyRefs(selected: readonly AgentRow[], source: readonly AgentRow[]): import("../src/agentScope").AgentRef[] {
  return selected.map(row => ({ pane_id: row.pane_id, occurrence: source.filter(a => a.pane_id === row.pane_id).indexOf(row) }));
}
