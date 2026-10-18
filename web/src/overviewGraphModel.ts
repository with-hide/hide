import { LAYOUT_NODE_LIMIT } from "./dependencyLayout";
import { scopeAgents, type LensAgent } from "./overviewLens";
import type { BoardProject } from "./projectBoard";
import type { OverviewGraphMetadata, OverviewGraphParty } from "./snapshot";

export type GraphAgent = LensAgent & { id: string; projectId: string; deviceId: string; digest: string | null; launchModel: string | null };
export type GraphProject = { id: string; project: BoardProject; agents: GraphAgent[]; expanded: boolean; roots: number };
export type Communication = { id: string; source: string; target: string; count: number; latest: number; kinds: Record<string, number>; directions: { from: string; to: string; count: number }[] };
export type OverviewGraphModel = {
  projects: GraphProject[]; agents: GraphAgent[]; visible: GraphAgent[];
  lineage: { from: string; to: string }[]; communication: Communication[];
  limited: boolean; hidden: number; structure: string;
};

export const graphProjectId = (project: BoardProject) => `project:${JSON.stringify([project.workspace.device_id, project.workspace.id])}`;
const partyKey = (party: OverviewGraphParty) => JSON.stringify([party.device_id, party.pane_id, party.session]);
const paneKey = (deviceId: string, paneId: string) => JSON.stringify([deviceId, paneId]);

/** Count current, core-verified relationships, including children outside the visible-node budget. */
export function overviewGraphFamily(row: GraphAgent, agents: readonly GraphAgent[]): { direct: number; descendants: number } {
  const children = new Map<string, GraphAgent[]>();
  for (const child of agents) {
    const parent = child.agent.lineage_parent_pane_id;
    if (!parent) continue;
    const key = paneKey(child.deviceId, parent);
    const siblings = children.get(key) ?? [];
    siblings.push(child);
    children.set(key, siblings);
  }
  const direct = children.get(paneKey(row.deviceId, row.agent.pane_id)) ?? [];
  const seen = new Set([row.id]);
  const pending = [...direct];
  while (pending.length > 0) {
    const child = pending.pop()!;
    if (seen.has(child.id)) continue;
    seen.add(child.id);
    pending.push(...(children.get(paneKey(child.deviceId, child.agent.pane_id)) ?? []));
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
    const panes = new Set(agents.map((row) => row.agent.pane_id));
    return { id, project, agents, roots: agents.filter((row) => !row.agent.lineage_parent_pane_id || !panes.has(row.agent.lineage_parent_pane_id)).length };
  });
}

export type ProjectGraphCounts = { total: number; roots: number; children: number };

/** The Projects comparison shares graph identity and scope, before any visible-node cap. */
export function overviewProjectGraphCounts(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined): ReadonlyMap<string, ProjectGraphCounts> {
  return new Map(projectSessions(projects, metadata).map((group) => [group.id, { total: group.agents.length, roots: group.roots, children: group.agents.length - group.roots }]));
}

export function overviewGraphModel(projects: readonly BoardProject[], metadata: OverviewGraphMetadata | undefined, expanded: ReadonlySet<string>, projectId: string | null): OverviewGraphModel {
  const scoped = projects.filter((project) => projectId === null || project.workspace.id === projectId);
  const all = projectSessions(scoped, metadata).map((group) => ({ ...group, expanded: expanded.has(group.id) }))
    .sort((a, b) => Number(b.expanded) - Number(a.expanded) || a.id.localeCompare(b.id));
  // Groups count as visible nodes. Never slice a family's children into a false complete graph.
  const groups = all.slice(0, LAYOUT_NODE_LIMIT);
  let room = LAYOUT_NODE_LIMIT - groups.length;
  let hidden = all.length - groups.length;
  for (const group of groups) {
    if (!group.expanded) continue;
    if (group.agents.length > room) { group.expanded = false; hidden += group.agents.length; }
    else room -= group.agents.length;
  }
  const agents = all.flatMap((group) => group.agents);
  const admitted = new Set(groups.map((group) => group.id));
  const visible = groups.flatMap((group) => group.expanded ? group.agents : []);
  const visibleIds = new Set(visible.map((row) => row.id));
  const byPane = new Map(agents.map((row) => [paneKey(row.deviceId, row.agent.pane_id), row]));
  const endpoint = new Map<string, string>();
  for (const row of agents) if (row.digest && admitted.has(row.projectId)) endpoint.set(partyKey({ device_id: row.deviceId, pane_id: row.agent.pane_id, session: row.digest }), visibleIds.has(row.id) ? row.id : row.projectId);
  const lineage = visible.flatMap((row) => {
    const parent = row.agent.lineage_parent_pane_id ? byPane.get(paneKey(row.deviceId, row.agent.lineage_parent_pane_id)) : undefined;
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
  return { projects: groups, agents, visible, lineage, communication, limited: hidden > 0, hidden, structure };
}

/** The most recently visited checkout's project opens first, then the core's activity order. */
export function initialGraphExpansion(projects: readonly BoardProject[], projectId: string | null, recentCheckoutIds: readonly string[] = []): Set<string> {
  const selected = projectId ? projects.find((project) => project.workspace.id === projectId) : undefined;
  const recent = recentCheckoutIds.map((id) => projects.find((project) => project.workspace.checkouts.some((checkout) => checkout.id === id))).find(Boolean);
  const first = selected ?? recent ?? projects.find((project) => project.workspace.agent_scope.members.length > 0) ?? projects[0];
  return new Set(first ? [graphProjectId(first)] : []);
}
