import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { scopeAgents, type LensAgent } from "./overviewLens";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, OverviewGraphMetadata, OverviewGraphParty } from "./snapshot";

export type GraphAgent = LensAgent & { id: string; projectId: string; deviceId: string; digest: string | null; launchModel: string | null };
export type GraphProject = { id: string; project: BoardProject; agents: GraphAgent[]; expanded: boolean; roots: number; matches: number };
export type GraphFocus = "all" | "turn" | "working" | "results";
export type GraphFocusOptions = { focus: GraphFocus; includeResting: boolean; folded?: ReadonlySet<string> };
export type GraphActivity = Exclude<GraphFocus, "all"> | "unknown" | "resting";
export const GRAPH_FOCUS_DEFAULTS: GraphFocusOptions = { focus: "all", includeResting: false };

/** Seen demands and sleeping raised asks still carry turn; bucket is not activity. */
export function graphActivity(agent: AgentRow): GraphActivity {
  switch (agent.mark) {
    case "turn": return "turn";
    case "going": case "waiting": return "working";
    case "done": case "unread": return "results";
    case "quiet": case "asleep": return "resting";
    default: return "unknown";
  }
}
const PRIORITY: Record<GraphActivity, number> = { turn: 0, working: 1, results: 2, unknown: 3, resting: 4 };
export type Communication = { id: string; source: string; target: string; count: number; latest: number; kinds: Record<string, number>; directions: { from: string; to: string; count: number }[] };
export type OverviewGraphModel = {
  projects: GraphProject[]; agents: GraphAgent[]; allAgents: GraphAgent[]; visible: GraphAgent[];
  context: ReadonlySet<string>; quietBranches: ReadonlyMap<string, number>; folded: ReadonlySet<string>;
  omitted: { id: string; project: BoardProject; count: number; reason: "empty" | "resting" | "filtered" }[];
  counts: Record<GraphFocus, number>;
  lineage: { from: string; to: string }[]; communication: Communication[];
  limited: boolean; hidden: number; singleProject: boolean; structure: string;
};

export const graphProjectId = (project: BoardProject) => `project:${JSON.stringify([project.workspace.device_id, project.workspace.id])}`;
const partyKey = (party: OverviewGraphParty) => JSON.stringify([party.device_id, party.pane_id, party.session]);

/** Core-qualified parents can cross devices; reused plain pane IDs need one device match. */
function lineageParents(agents: readonly GraphAgent[]): ReadonlyMap<string, GraphAgent> {
  const byPane = new Map<string, GraphAgent | null>();
  const byDevicePane = new Map<string, GraphAgent | null>();
  for (const row of agents) {
    const pane = row.agent.pane_id;
    const key = JSON.stringify([row.deviceId, pane]);
    byPane.set(pane, byPane.has(pane) ? null : row);
    byDevicePane.set(key, byDevicePane.has(key) ? null : row);
  }
  const parents = new Map<string, GraphAgent>();
  for (const child of agents) {
    const pane = child.agent.lineage_parent_pane_id;
    if (!pane) continue;
    const exact = byPane.get(pane);
    const parent = exact === null && !pane.startsWith("remote:") ? byDevicePane.get(JSON.stringify([child.deviceId, pane])) : exact;
    if (parent && parent.id !== child.id) parents.set(child.id, parent);
  }
  return parents;
}

/** Count current, core-verified relationships, including children outside the visible-node budget. */
export function overviewGraphFamily(row: GraphAgent, agents: readonly GraphAgent[]): { direct: number; descendants: number } {
  const children = new Map<string, GraphAgent[]>();
  const parents = lineageParents(agents);
  for (const child of agents) {
    const parent = parents.get(child.id);
    if (!parent) continue;
    const siblings = children.get(parent.id) ?? [];
    siblings.push(child);
    children.set(parent.id, siblings);
  }
  const direct = children.get(row.id) ?? [];
  const seen = new Set([row.id]);
  const pending = [...direct];
  while (pending.length > 0) {
    const child = pending.pop()!;
    if (seen.has(child.id)) continue;
    seen.add(child.id);
    pending.push(...(children.get(child.id) ?? []));
  }
  return { direct: direct.filter((child) => child.id !== row.id).length, descendants: seen.size - 1 };
}

/** Membership is the core's scope, never a cwd prefix, registration history or a fork origin. */
function projectSessions(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined): Omit<GraphProject, "expanded">[] {
  const proof = new Map((metadata?.sessions ?? []).map((session) => [JSON.stringify([session.device_id, session.pane_id]), session]));
  const seen = new Set<string>();
  return projects.map((project) => {
    const id = graphProjectId(project);
    const agents = scopeAgents([project]).flatMap((row): GraphAgent[] => {
      const remoteDevice = row.agent.pane_id.startsWith("remote:") ? row.agent.pane_id.slice(7).split(":pane:")[0] : null;
      const deviceId = row.agent.device_id ?? remoteDevice ?? row.project.device_id;
      const session = proof.get(JSON.stringify([deviceId, row.agent.pane_id]));
      const digest = session?.session_digest ?? null;
      const agentId = `session:${JSON.stringify([deviceId, row.agent.pane_id, digest ?? row.agent.session_id ?? null])}`;
      if (seen.has(agentId)) return [];
      seen.add(agentId);
      return [{ ...row, id: agentId, projectId: id, deviceId, digest, launchModel: session?.launch_model ?? null }];
    });
    agents.sort((a, b) => a.id.localeCompare(b.id));
    const parents = lineageParents(agents);
    return { id, project, agents, matches: agents.length, roots: agents.filter((row) => !parents.has(row.id)).length };
  });
}

export type ProjectGraphCounts = { total: number; roots: number; children: number };

/** The Projects comparison shares graph identity and scope, before any visible-node cap. */
export function overviewProjectGraphCounts(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined): ReadonlyMap<string, ProjectGraphCounts> {
  return new Map(projectSessions(projects, metadata).map((group) => [group.id, { total: group.agents.length, roots: group.roots, children: group.agents.length - group.roots }]));
}

export function overviewGraphModel(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined, expanded: ReadonlySet<string>, projectId: string | null, options: GraphFocusOptions = GRAPH_FOCUS_DEFAULTS): OverviewGraphModel {
  const scoped = projects.filter((project) => projectId === null || project.workspace.id === projectId);
  const sessions = projectSessions(scoped, metadata);
  const allAgents = sessions.flatMap((group) => group.agents);
  const parents = lineageParents(allAgents);
  const counts = { all: 0, turn: 0, working: 0, results: 0 };
  const matches = allAgents.filter((row) => {
    const activity = graphActivity(row.agent);
    if (activity !== "resting" || options.includeResting) counts.all += 1;
    if (activity === "turn" || activity === "working" || activity === "results") counts[activity] += 1;
    return (options.includeResting || activity !== "resting") && (options.focus === "all" || activity === options.focus);
  });
  const retained = new Set(matches.map((row) => row.id));
  const matching = new Set(retained);
  // Stop at already visited ancestors: linear work across all matched paths, cycles included.
  const visited = new Set<string>();
  for (const row of matches) {
    let current: GraphAgent | undefined = row;
    while (current && !visited.has(current.id)) {
      visited.add(current.id);
      retained.add(current.id);
      current = parents.get(current.id);
    }
  }
  const context = new Set([...retained].filter((id) => !matching.has(id)));
  const children = new Map<string, GraphAgent[]>();
  for (const row of allAgents) {
    const parent = parents.get(row.id);
    if (!parent) continue;
    const siblings = children.get(parent.id) ?? [];
    siblings.push(row);
    children.set(parent.id, siblings);
  }
  // Leaves settle first. A cycle never settles and therefore never offers an unsafe fold.
  const pending = new Map(allAgents.map((row) => [row.id, children.get(row.id)?.length ?? 0]));
  const quiet = new Map<string, boolean>();
  const descendantCount = new Map<string, number>();
  const ready = allAgents.filter((row) => pending.get(row.id) === 0);
  const quietBranches = new Map<string, number>();
  for (let index = 0; index < ready.length; index += 1) {
    const row = ready[index]!;
    const kids = children.get(row.id) ?? [];
    const safe = kids.every((child) => graphActivity(child.agent) === "resting" && quiet.get(child.id));
    const count = kids.reduce((total, child) => total + Number(retained.has(child.id)) + (descendantCount.get(child.id) ?? 0), 0);
    quiet.set(row.id, safe);
    descendantCount.set(row.id, count);
    if (safe && count > 0 && retained.has(row.id)) quietBranches.set(row.id, count);
    const parent = parents.get(row.id);
    if (parent) {
      const left = pending.get(parent.id)! - 1;
      pending.set(parent.id, left);
      if (left === 0) ready.push(parent);
    }
  }
  const folded = new Set([...(options.folded ?? [])].filter((id) => quietBranches.has(id)));
  const tucked = new Set<string>();
  const tuck = [...folded].flatMap((id) => children.get(id) ?? []);
  while (tuck.length > 0) {
    const row = tuck.pop()!;
    if (tucked.has(row.id)) continue;
    tucked.add(row.id);
    tuck.push(...(children.get(row.id) ?? []));
  }
  const omitted = sessions.filter((group) => !group.agents.some((row) => retained.has(row.id))).map((group) => ({ id: group.id, project: group.project, count: group.agents.length, reason: group.agents.length === 0 ? "empty" as const : group.agents.every((row) => graphActivity(row.agent) === "resting") ? "resting" as const : "filtered" as const }));
  const all = sessions.filter((group) => group.agents.some((row) => retained.has(row.id))).map((group) => ({ ...group, matches: group.agents.filter((row) => matching.has(row.id)).length, agents: group.agents.filter((row) => retained.has(row.id) && !tucked.has(row.id)), expanded: projectId !== null || expanded.has(group.id) }))
    .sort((a, b) => Number(b.expanded) - Number(a.expanded) || a.id.localeCompare(b.id));
  // Groups count as visible nodes. Never slice a family's children into a false complete graph.
  const groups = all.slice(0, LAYOUT_NODE_LIMIT);
  for (const group of groups) {
    const projectedParents = lineageParents(group.agents);
    group.roots = group.agents.filter((row) => !projectedParents.has(row.id)).length;
  }
  let room = LAYOUT_NODE_LIMIT - groups.length;
  let hidden = all.length - groups.length;
  for (const group of groups) {
    if (!group.expanded) continue;
    if (group.agents.length > room) { group.expanded = false; hidden += group.agents.length; }
    else room -= group.agents.length;
  }
  const agents = matches.sort((a, b) => PRIORITY[graphActivity(a.agent)] - PRIORITY[graphActivity(b.agent)] || a.id.localeCompare(b.id));
  const admitted = new Set(groups.map((group) => group.id));
  const visible = groups.flatMap((group) => group.expanded ? group.agents : []);
  const visibleIds = new Set(visible.map((row) => row.id));
  const endpoint = new Map<string, string>();
  for (const row of allAgents) if (retained.has(row.id) && !tucked.has(row.id) && row.digest && admitted.has(row.projectId)) endpoint.set(partyKey({ device_id: row.deviceId, pane_id: row.agent.pane_id, session: row.digest }), visibleIds.has(row.id) ? row.id : row.projectId);
  const lineage = visible.flatMap((row) => {
    const parent = parents.get(row.id);
    return parent && visibleIds.has(parent.id) ? [{ from: parent.id, to: row.id }] : [];
  }).sort((a, b) => a.from.localeCompare(b.from) || a.to.localeCompare(b.to));
  const bundles = new Map<string, Communication>();
  for (const letter of metadata?.available ? metadata.letters.slice(0, metadata.limit) : []) {
    const from = endpoint.get(partyKey(letter.sender));
    const to = endpoint.get(partyKey(letter.recipient));
    if (!from || !to || from === to) continue;
    const [source, target] = [from, to].sort() as [string, string];
    const key = `letters:${JSON.stringify([source, target])}`;
    let bundle = bundles.get(key);
    if (!bundle) { bundle = { id: key, source, target, count: 0, latest: 0, kinds: {}, directions: [] }; bundles.set(key, bundle); }
    bundle.count += 1;
    bundle.latest = Math.max(bundle.latest, letter.at_unix_ms);
    bundle.kinds[letter.kind] = (bundle.kinds[letter.kind] ?? 0) + 1;
    const direction = bundle.directions.find((row) => row.from === from && row.to === to);
    if (direction) direction.count += 1;
    else bundle.directions.push({ from, to, count: 1 });
  }
  // Labels, status, model, letters, clocks and selection deliberately do not participate.
  const structure = JSON.stringify([groups.map((group) => [group.id, group.expanded, group.expanded ? group.agents.map((row) => row.id) : []]), lineage.map((edge) => [edge.from, edge.to])]);
  const communication = [...bundles.values()];
  for (const bundle of communication) if (bundle.directions.length === 1) {
    bundle.source = bundle.directions[0]!.from;
    bundle.target = bundle.directions[0]!.to;
  }
  return { projects: groups, agents, allAgents, visible, context, quietBranches, folded, omitted, counts, lineage, communication, limited: hidden > 0, hidden, singleProject: projectId !== null, structure };
}

/** The most recently visited checkout's project opens first, then the core's activity order. */
export function initialGraphExpansion(projects: readonly BoardProject[], projectId: string | null, recentCheckoutIds: readonly string[] = []): Set<string> {
  const selected = projectId ? projects.find((project) => project.workspace.id === projectId) : undefined;
  const recent = recentCheckoutIds.map((id) => projects.find((project) => project.workspace.checkouts.some((checkout) => checkout.id === id))).find(Boolean);
  const active = (project: BoardProject) => scopeAgents([project]).some((row) => graphActivity(row.agent) !== "resting");
  const first = selected ?? (recent && active(recent) ? recent : undefined) ?? projects.find(active) ?? recent ?? projects.find((project) => project.workspace.agent_scope.members.length > 0) ?? projects[0];
  return new Set(first ? [graphProjectId(first)] : []);
}
