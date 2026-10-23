import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { scopeAgents, type LensAgent } from "./overviewLens";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, GraphProjectKey, OverviewGraphFocus, GraphScope, GraphSessionKey, OverviewGraphChoices, OverviewGraphMetadata, OverviewGraphParty } from "./snapshot";

export type GraphAgent = LensAgent & { id: string; projectId: string; deviceId: string; digest: string | null; launchModel: string | null; launchEffort: string | null };
export type GraphProject = { id: string; project: BoardProject; agents: GraphAgent[]; roots: number; shown: GraphAgent[]; revealSlot: string | null; revealed: GraphAgent | null };
export type GraphFocus = OverviewGraphFocus;
/** `chosen`: the session whose details are open, drawn whatever the filter says (PRD pane-open-in-overview D-05). */
export type GraphFocusOptions = { focus: GraphFocus; includeResting: boolean; pinned?: readonly string[]; selected?: string | null; chosen?: string | null };
export type GraphActivity = Exclude<GraphFocus, "all"> | "unknown" | "idle" | "resting";
export const GRAPH_FOCUS_DEFAULTS: GraphFocusOptions = { focus: "all", includeResting: false };

/** Seen demands and sleeping raised asks still carry turn; bucket is not activity. */
export function graphActivity(agent: AgentRow): GraphActivity {
  switch (agent.mark) {
    case "turn": return "turn";
    case "going": case "waiting": return "working";
    case "done": case "unread": return "results";
    case "quiet": return "idle";
    case "asleep": return "resting";
    default: return "unknown";
  }
}
const PRIORITY: Record<GraphActivity, number> = { turn: 0, working: 1, results: 2, unknown: 3, idle: 4, resting: 5 };
/** One pair of sessions and every letter and Herdr input between them, by direction. */
export type Communication = { id: string; source: string; target: string; letters: number; direct: number; directions: { from: string; to: string; letters: number; direct: number }[] };
/** A project row of the map's side column: its graph members and their states. */
export type ProjectSummary = { id: string; project: BoardProject; sessions: number; turn: number; working: number; results: number; admitted: boolean };
export type OverviewGraphModel = {
  projects: GraphProject[]; agents: GraphAgent[]; allAgents: GraphAgent[]; visible: GraphAgent[];
  context: ReadonlySet<string>; roots: ReadonlySet<string>;
  /** Sessions whose every descendant sleeps, with how many descendants: a team the map opens folded. */
  quietBranches: ReadonlyMap<string, number>;
  /** Each current session's verified lineage parent and lineage root. */
  parents: ReadonlyMap<string, string>; teams: ReadonlyMap<string, string>;
  /** Each drawn session's nearest drawn ancestor, or null for a root. */
  layoutParents: ReadonlyMap<string, string | null>;
  counts: Record<GraphFocus, number>; summaries: ProjectSummary[];
  communication: Communication[];
  limited: boolean; structure: string;
};

export const graphProjectId = (project: BoardProject) => `project:${JSON.stringify([project.workspace.device_id, project.workspace.id])}`;
export const graphSessionId = (deviceId: string, paneId: string, session: string | null) => `session:${JSON.stringify([deviceId, paneId, session])}`;
/** The core's record of a project or session names the same three things the graph id does (PRD overview-graph-remembered D-09). */
export const graphProjectKey = (workspace: Pick<BoardProject["workspace"], "device_id" | "id">): GraphProjectKey => ({ device_id: workspace.device_id, workspace_id: workspace.id });
export const graphProjectIdOf = (key: GraphProjectKey) => `project:${JSON.stringify([key.device_id, key.workspace_id])}`;
export const graphSessionKey = (row: GraphAgent): GraphSessionKey => ({ device_id: row.deviceId, pane_id: row.agent.pane_id, session: row.digest ?? row.agent.session_id ?? null });
export const graphSessionIdOf = (key: GraphSessionKey) => graphSessionId(key.device_id, key.pane_id, key.session);
export const sameGraphScope = (a: GraphScope, b: GraphScope) => a.kind === b.kind && a.device_id === b.device_id && (a.kind === "all" || b.kind === "all" || a.workspace_id === b.workspace_id);
/** What the core holds before the operator chose anything. */
export const GRAPH_CHOICES_DEFAULTS: OverviewGraphChoices = { focus: "all", include_resting: false, hidden_projects: [], folded_projects: [], team_folds: [], project_places: [], session_places: [] };

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

/** Each session's lineage root; a cycle's root is its smallest member, so the walk always ends. */
function lineageRoots(agents: readonly GraphAgent[], parents: ReadonlyMap<string, string>): Map<string, string> {
  const team = new Map<string, string>();
  for (const row of agents) {
    const path: string[] = [];
    const seen = new Set<string>();
    let current: string | undefined = row.id;
    while (current !== undefined && !team.has(current) && !seen.has(current)) { seen.add(current); path.push(current); current = parents.get(current); }
    const root = current === undefined ? path.at(-1)! : team.has(current) ? team.get(current)! : path.slice(path.indexOf(current)).sort()[0]!;
    for (const id of path) team.set(id, root);
  }
  return team;
}

/** A session's current, core-verified descendants in tree order, each with its depth below it. */
export function overviewGraphTeam(row: GraphAgent, model: Pick<OverviewGraphModel, "allAgents" | "parents">): { row: GraphAgent; depth: number }[] {
  const children = new Map<string, GraphAgent[]>();
  for (const child of model.allAgents) {
    const parent = model.parents.get(child.id);
    if (parent) children.set(parent, [...(children.get(parent) ?? []), child]);
  }
  const seen = new Set([row.id]);
  const team: { row: GraphAgent; depth: number }[] = [];
  const visit = (id: string, depth: number) => {
    for (const child of [...(children.get(id) ?? [])].sort((a, b) => a.id.localeCompare(b.id))) {
      if (seen.has(child.id)) continue;
      seen.add(child.id);
      team.push({ row: child, depth });
      visit(child.id, depth + 1);
    }
  };
  visit(row.id, 1);
  return team;
}

/** Membership is the core's scope, never a cwd prefix, registration history or a fork origin. */
function projectSessions(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined): Pick<GraphProject, "id" | "project" | "agents" | "roots">[] {
  const proof = new Map((metadata?.sessions ?? []).map((session) => [JSON.stringify([session.device_id, session.pane_id]), session]));
  const seen = new Set<string>();
  return projects.map((project) => {
    const id = graphProjectId(project);
    const agents = scopeAgents([project]).flatMap((row): GraphAgent[] => {
      const remoteDevice = row.agent.pane_id.startsWith("remote:") ? row.agent.pane_id.slice(7).split(":pane:")[0] : null;
      const deviceId = row.agent.device_id ?? remoteDevice ?? row.project.device_id;
      const session = proof.get(JSON.stringify([deviceId, row.agent.pane_id]));
      const digest = session?.session_digest ?? null;
      const agentId = graphSessionId(deviceId, row.agent.pane_id, digest ?? row.agent.session_id ?? null);
      if (seen.has(agentId)) return [];
      seen.add(agentId);
      return [{ ...row, id: agentId, projectId: id, deviceId, digest, launchModel: session?.launch_model ?? null, launchEffort: session?.launch_effort ?? null }];
    });
    agents.sort((a, b) => a.id.localeCompare(b.id));
    const parents = lineageParents(agents);
    return { id, project, agents, roots: agents.filter((row) => !parents.has(row.id)).length };
  });
}

export type ProjectGraphCounts = { total: number; roots: number; children: number };

/** The Projects comparison shares graph identity and scope, before any visible-node cap. */
export function overviewProjectGraphCounts(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined): ReadonlyMap<string, ProjectGraphCounts> {
  return new Map(projectSessions(projects, metadata).map((group) => [group.id, { total: group.agents.length, roots: group.roots, children: group.agents.length - group.roots }]));
}

export function overviewGraphModel(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined, projectId: string | null, options: GraphFocusOptions = GRAPH_FOCUS_DEFAULTS): OverviewGraphModel {
  const scoped = projects.filter((project) => projectId === null || project.workspace.id === projectId);
  const sessions = projectSessions(scoped, metadata);
  const allAgents = sessions.flatMap((group) => group.agents);
  const parentRows = lineageParents(allAgents);
  const parents = new Map([...parentRows].map(([id, parent]) => [id, parent.id]));
  const counts = { all: 0, turn: 0, working: 0, results: 0 };
  const present = (row: GraphAgent) => options.includeResting || graphActivity(row.agent) !== "resting";
  const matches = allAgents.filter((row) => {
    const activity = graphActivity(row.agent);
    if (present(row)) counts.all += 1;
    if (activity === "turn" || activity === "working" || activity === "results") counts[activity] += 1;
    return present(row) && (options.focus === "all" || activity === options.focus);
  });
  const matching = new Set(matches.map((row) => row.id));
  // The map holds every present session and its ancestors whatever the filter,
  // so a filter dims sessions instead of moving them.
  const retained = new Set<string>();
  const visited = new Set<string>();
  // Stop at already visited ancestors: linear work across all paths, cycles included.
  for (const row of allAgents.filter((row) => present(row) || row.id === options.chosen)) {
    let current: GraphAgent | undefined = row;
    while (current && !visited.has(current.id)) {
      visited.add(current.id);
      retained.add(current.id);
      current = parentRows.get(current.id);
    }
  }
  const context = new Set([...retained].filter((id) => !matching.has(id) && id !== options.chosen));
  const children = new Map<string, GraphAgent[]>();
  for (const row of allAgents) {
    const parent = parentRows.get(row.id);
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
    const parent = parentRows.get(row.id);
    if (parent) {
      const left = pending.get(parent.id)! - 1;
      pending.set(parent.id, left);
      if (left === 0) ready.push(parent);
    }
  }
  const all = sessions.filter((group) => group.agents.some((row) => retained.has(row.id))).map((group) => ({ ...group, agents: group.agents.filter((row) => retained.has(row.id)) }))
    .sort((a, b) => a.id.localeCompare(b.id));
  // Project names share the 400-node budget with their sessions. Counts and the list stay uncapped.
  const pinned = new Set(options.pinned ?? []);
  const admissionPriority = new Map(all.map((group) => [group.id, Number(group.agents.some((row) => row.id === options.selected)) * 4 + Number(group.agents.some((row) => pinned.has(row.id))) * 2]));
  const priority = [...all].sort((a, b) => admissionPriority.get(b.id)! - admissionPriority.get(a.id)! || a.id.localeCompare(b.id));
  // Admit complete name + access-slot pairs first. A greedy large project
  // must not consume the one position another project needs.
  const groups: GraphProject[] = [];
  let minimum = 0;
  for (const group of priority) {
    const protectedCount = group.agents.filter((row) => pinned.has(row.id)).length;
    const access = Math.min(group.agents.length, protectedCount + 1);
    if (minimum + 1 + access > LAYOUT_NODE_LIMIT) break;
    groups.push({ ...group, shown: [], revealSlot: null, revealed: null });
    minimum += 1 + access;
  }
  let room = LAYOUT_NODE_LIMIT - minimum;
  let hidden = all.length - groups.length;
  // Admitted projects share the remaining room in identity order, so choosing a
  // session reveals it in its reserved slot without moving another project's members.
  groups.sort((a, b) => a.id.localeCompare(b.id));
  for (const group of groups) {
    const projectedParents = lineageParents(group.agents);
    group.roots = group.agents.filter((row) => !projectedParents.has(row.id)).length;
    const protectedRows = group.agents.filter((row) => pinned.has(row.id));
    const ordered = [...protectedRows, ...group.agents.filter((row) => !pinned.has(row.id))];
    const reserved = Math.min(group.agents.length, protectedRows.length + 1);
    // One admission position is already reserved above. A complete project
    // needs no reveal slot, while every summary can reveal any of its list rows.
    if (group.agents.length <= room + reserved) {
      group.shown = ordered.slice(0, room + reserved);
      room -= group.shown.length - reserved;
    } else {
      group.shown = ordered.slice(0, Math.max(protectedRows.length, Math.min(6, room + reserved - 1)));
      room -= group.shown.length + 1 - reserved;
      group.revealSlot = `${group.id}:reveal`;
      group.revealed = group.agents.find((row) => row.id === options.selected && !group.shown.some((shown) => shown.id === row.id))
        ?? protectedRows.find((row) => !group.shown.some((shown) => shown.id === row.id)) ?? null;
    }
    // Places follow identity, never which member happens to be protected.
    group.shown.sort((a, b) => a.id.localeCompare(b.id));
    hidden += group.agents.length - group.shown.length - Number(group.revealed !== null);
  }
  const agents = matches.sort((a, b) => PRIORITY[graphActivity(a.agent)] - PRIORITY[graphActivity(b.agent)] || a.id.localeCompare(b.id));
  const admitted = new Set(groups.map((group) => group.id));
  const visible = groups.flatMap((group) => [...group.shown, ...(group.revealed ? [group.revealed] : [])]);
  const placed = new Set(groups.flatMap((group) => group.shown.map((row) => row.id)));
  const layoutParents = new Map<string, string | null>();
  for (const row of visible) {
    // A revealed member stands in its project's access slot, outside any team.
    let parent: string | undefined = placed.has(row.id) ? parents.get(row.id) : undefined;
    const seen = new Set([row.id]);
    while (parent !== undefined && !placed.has(parent) && !seen.has(parent)) { seen.add(parent); parent = parents.get(parent); }
    layoutParents.set(row.id, parent !== undefined && placed.has(parent) && parent !== row.id ? parent : null);
  }
  const summaries = sessions.flatMap((group): ProjectSummary[] => {
    const members = group.agents.filter((row) => retained.has(row.id));
    if (!members.length) return [];
    const activity = members.map((row) => graphActivity(row.agent));
    return [{ id: group.id, project: group.project, sessions: members.length, turn: activity.filter((value) => value === "turn").length, working: activity.filter((value) => value === "working").length, results: activity.filter((value) => value === "results").length, admitted: admitted.has(group.id) }];
  }).sort((a, b) => a.id.localeCompare(b.id));
  // Every proven session keeps its counterparts, drawn or not; the view decides which pairs it draws.
  const endpoint = new Map<string, string>();
  for (const row of allAgents) if (row.digest) endpoint.set(partyKey({ device_id: row.deviceId, pane_id: row.agent.pane_id, session: row.digest }), row.id);
  const pairs = new Map<string, Communication>();
  const limit = Math.max(0, Math.min(2000, metadata?.limit ?? 2000));
  const records = [
    ...(metadata?.available ? metadata.letters.slice(0, Math.min(2000, metadata.limit)).map((letter) => ({ ...letter, type: "letters" as const })) : []),
    ...(metadata?.direct_inputs ?? []).slice(0, 2000).map((input) => ({ ...input, type: "direct" as const })),
  ].sort((a, b) => b.at_unix_ms - a.at_unix_ms || b.id.localeCompare(a.id)).slice(0, limit);
  for (const record of records) {
    const from = endpoint.get(partyKey(record.sender));
    const to = endpoint.get(partyKey(record.recipient));
    if (!from || !to || from === to) continue;
    const [source, target] = [from, to].sort() as [string, string];
    const key = `pair:${JSON.stringify([source, target])}`;
    let pair = pairs.get(key);
    if (!pair) { pair = { id: key, source, target, letters: 0, direct: 0, directions: [] }; pairs.set(key, pair); }
    pair[record.type] += 1;
    let direction = pair.directions.find((row) => row.from === from && row.to === to);
    if (!direction) { direction = { from, to, letters: 0, direct: 0 }; pair.directions.push(direction); }
    direction[record.type] += 1;
  }
  const communication = [...pairs.values()].sort((a, b) => a.id.localeCompare(b.id));
  for (const pair of communication) if (pair.directions.length === 1) {
    pair.source = pair.directions[0]!.from;
    pair.target = pair.directions[0]!.to;
  }
  // Labels, status, filters, letters, clocks and selection deliberately do not participate.
  const structure = JSON.stringify(groups.map((group) => [group.id, group.shown.map((row) => [row.id, layoutParents.get(row.id)]), group.revealSlot]));
  return {
    projects: groups, agents, allAgents, visible, context, roots: new Set(allAgents.filter((row) => !parents.has(row.id)).map((row) => row.id)), quietBranches,
    parents, teams: lineageRoots(allAgents, parents), layoutParents, counts, summaries, communication, limited: hidden > 0, structure,
  };
}
