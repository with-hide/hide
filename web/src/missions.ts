// What the mission map and the Conductor row read from a mission (PRD
// software-factory-conductor): the core decides every state and number
// (`MissionView`), so this module only finds a mission, words its parts
// and counts what the record holds. Pure, so the rules test on their own;
// `MissionMap.tsx` draws them.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import { agentsByPane } from "./paneAgents";
import { layerDependencies, type LayeredGraph } from "./projectBoard";
import type { AgentRow, AgentStartChoice, Mission, MissionNode, RowMark, SnapshotRest } from "./snapshot";

export function missionOf(rest: SnapshotRest | null | undefined, id: string): Mission | null {
  return rest?.missions?.items.find((mission) => mission.id === id) ?? null;
}

/** Whether a mission is still the Conductor's to carry: planned, running or waiting on a merge. */
export function missionOpen(mission: Mission): boolean {
  return mission.status === "planning" || mission.status === "running" || mission.status === "merge_wait";
}

/** The pane or the session a mission's Conductor is known by: a live row's pane, an archived row's session. */
export type ConductorOwner = { pane_id?: string; device_id?: string; session?: { device_id: string; session_id: string } | null };

/**
 * The mission a row conducts (B14, B46): its open one, else the last it
 * closed, so a done mission's map stays reachable from its Conductor, a
 * live row by its pane and an archived one by its session.
 */
export function conductedBy(rest: SnapshotRest | null | undefined, owner: ConductorOwner): Mission | null {
  const mine = (rest?.missions?.items ?? []).filter((mission) => {
    const session = mission.conductor.session;
    if (owner.session && session) return session.device_id === owner.session.device_id && session.session_id === owner.session.session_id;
    return owner.pane_id !== undefined && mission.conductor.pane_id === owner.pane_id && (!owner.device_id || mission.device_id === owner.device_id);
  });
  return mine.find(missionOpen) ?? mine.filter((mission) => mission.status !== "planning").sort((a, b) => (b.closed_at_unix_ms ?? 0) - (a.closed_at_unix_ms ?? 0))[0] ?? null;
}

/**
 * The mission a pane's session conducts, found through the pane's agent row as
 * its sidebar row finds it (PRD mission-pane-chip B1, B10): none while the
 * record cannot be read, so no count is invented.
 */
export function missionOfPane(rest: SnapshotRest | null | undefined, agents: AgentRow[], paneId: string): Mission | null {
  if (rest?.missions?.failed) return null;
  const agent = agentsByPane(agents).get(paneId);
  return agent ? conductedBy(rest, agent) : null;
}

/** What the record counts (B21): done nodes of those still in the plan, and the ones dropped. */
export function missionCounts(mission: Mission): { done: number; total: number; dropped: number } {
  const dropped = mission.nodes.filter((node) => node.status === "dropped").length;
  return { done: mission.nodes.filter((node) => node.status === "done").length, total: mission.nodes.length - dropped, dropped };
}

/** The pull requests of a stack still waiting on a lower one: open, with a base the mission holds. */
export function stackWaiting(mission: Mission): number {
  return mission.nodes.filter((node) => node.base_pr != null && node.pr_state !== "merged" && node.pr_state !== "closed" && node.status !== "dropped").length;
}

const KIND_WORD: Record<MissionNode["kind"], MessageKey> = {
  change: "mission.kind.change",
  research: "mission.kind.research",
  verify: "mission.kind.verify",
};

export function kindWord(kind: MissionNode["kind"]): MessageKey {
  return KIND_WORD[kind];
}

/**
 * The card's last line (B15, B51, B53): why it waits on a failed or dropped
 * node, else what its worker last said, else its kind. A node that waits
 * on a node not done yet says nothing of it: the edge above it does.
 */
export function nodeLine(node: MissionNode, t: TFunction<"translation">): string {
  if (node.blocked === "upstream_failed") return t("mission.blocked.failed");
  if (node.blocked === "upstream_dropped") return t("mission.blocked.dropped");
  if (node.line) return node.line;
  return t(kindWord(node.kind));
}

/**
 * The model a worker's pane runs with: the launch Hide recorded for the
 * node's pane on its machine (`overview_graph.sessions`), null when it has
 * no pane, the pane was not started by Hide or the launch named no model.
 */
export function launchModelOf(rest: SnapshotRest | null | undefined, mission: Mission, node: MissionNode): string | null {
  if (!node.pane_id) return null;
  const device = node.machine ?? mission.device_id;
  return rest?.overview_graph?.sessions.find((session) => session.device_id === device && session.pane_id === node.pane_id)?.launch_model ?? null;
}

/**
 * A worker as the card names it: the model and the effort, never the agent's
 * name (its logo stands beside the words). A working or review node shows
 * the model its pane runs (`launchModel`) and falls back to the plan's; a
 * planned node shows the plan's. With no model (an agent that takes none)
 * the effort stands alone, `자동` while the Conductor will choose it.
 */
export function workerWords(node: MissionNode, launchModel: string | null, auto: string): string | null {
  const worker = node.worker;
  if (!worker) return null;
  const live = node.status === "working" || node.status === "review";
  const model = (live ? launchModel : null) ?? worker.model ?? null;
  const effort = worker.effort ?? (worker.effort_auto ? auto : null);
  if (model && effort) return `${model}·${effort}`;
  return model ?? effort;
}

/** A change node's steps: work, PR or branch, review, merge or main (D-42 B, D-48). */
export const STAGE_STEPS = 4;

/** How many step cells a card draws: the four of a change node, none for a research or verify node, whose status mark already says where it is. */
export function stepsOf(node: MissionNode): number {
  return node.kind === "change" ? STAGE_STEPS : 0;
}

/**
 * The card's status mark (B15): its worker's own mark while its row is
 * listed, so a node reads as its agent does everywhere; else the node's
 * state in the same four shapes, a node that waits on another dashed.
 */
export function nodeMark(node: MissionNode, worker: RowMark | null): RowMark {
  if (worker && (node.status === "working" || node.status === "review")) return worker;
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "working":
    case "review":
      return "going";
    case "waiting":
    case "held":
      return "waiting";
    case "failed":
      return "turn";
    case "done":
      return "done";
    case "planned":
    case "dropped":
      return "quiet";
  }
}

/** The Conductor row's mark (B14): `미션 n/m` while it runs, its plan or its wait otherwise. */
export function chipWords(mission: Mission): { key: MessageKey; counts: string | null } {
  const { done, total } = missionCounts(mission);
  switch (mission.status) {
    case "planning":
      return { key: "mission.chip.plan", counts: null };
    case "merge_wait":
      return { key: "mission.chip.mergeWait", counts: null };
    case "done":
      return { key: "mission.chip.done", counts: null };
    case "cancelled":
      return { key: "mission.chip.cancelled", counts: null };
    case "running":
      return { key: "mission.chip", counts: `${done}/${total}` };
  }
}

/** The Conductor pane's chip (PRD mission-pane-chip B2): the state it reads as, its word and its count. */
export type PaneChipState = "planning" | "running" | "decision" | "merge_wait" | "done" | "cancelled";

/** A mission holds at most one decision (`decision`), so the chip's count is always one. */
export function paneChipWords(mission: Mission): { state: PaneChipState; key: MessageKey; counts: string | null } {
  const { done, total } = missionCounts(mission);
  const counts = `${done}/${total}`;
  if (mission.decision && missionOpen(mission)) return { state: "decision", key: "mission.pane.decision", counts: null };
  switch (mission.status) {
    case "planning":
      // A plan put up and not started yet counts from none done (approved board 4).
      return mission.nodes.length === 0 ? { state: "planning", key: "mission.pane.planning", counts: null } : { state: "running", key: "mission.chip", counts };
    case "running":
      return { state: "running", key: "mission.chip", counts };
    case "merge_wait":
      return { state: "merge_wait", key: "mission.pane.mergeWait", counts: null };
    case "done":
      return { state: "done", key: "mission.pane.done", counts };
    case "cancelled":
      return { state: "cancelled", key: "mission.pane.cancelled", counts: null };
  }
}

/** The card the small map's paths end in; a node key is letters, digits, `-`, `_` and `.`, so none is this. */
export const MINI_REPORT = "#report";

/** A row of the small map keeps this many cards; past it, its first card and one `+N` card (B8). */
export const MINI_ROW_CARDS = 2;

/** A card of the small map: a node, the `+N` card standing for a row's folded nodes, or the report. */
export type MiniCard = { key: string; kind: "node"; node: MissionNode } | { key: string; kind: "more"; nodes: MissionNode[] } | { key: string; kind: "report" };

/** An edge of the small map, and whether a stack's base runs along it (B7). */
export type MiniEdge = { from: string; to: string; stacked: boolean };

/**
 * The Conductor pane's small map (PRD mission-pane-chip D-04, D-05, B6-B8):
 * the done nodes counted apart, the ones still to do top down by what they
 * wait on into the report card, and a row of more than two cards as its
 * first card and one `+N` card that every edge of a folded node meets. A
 * dropped node left the plan, as `missionCounts` reads it, so it is not drawn.
 */
export function missionMiniGraph(mission: Mission): { done: number; graph: LayeredGraph<MiniCard>; stacked: ReadonlySet<string> } {
  const done = mission.nodes.filter((node) => node.status === "done").length;
  const nodes = mission.nodes.filter((node) => node.status !== "done" && node.status !== "dropped");
  const empty = { layers: [], edges: [], unrelated: [] };
  if (nodes.length === 0) return { done, graph: empty, stacked: new Set() };
  const cards: MiniCard[] = [...nodes.map((node): MiniCard => ({ key: node.key, kind: "node", node })), { key: MINI_REPORT, kind: "report" }];
  const byKey = new Map(cards.map((card) => [card.key, card]));
  const depended = new Set(nodes.flatMap((node) => node.depends_on));
  const layered = layerDependencies(cards, (card) => card.key, (card) => {
    // Every node no other node still to do waits on ends in the report.
    if (card.kind === "report") return nodes.filter((node) => !depended.has(node.key)).map((node) => byKey.get(node.key)!);
    return card.kind === "node" ? card.node.depends_on.flatMap((key) => byKey.get(key) ?? []) : [];
  });
  const into = new Map<string, string>();
  const layers = layered.layers.map((layer, index) => {
    if (layer.length <= MINI_ROW_CARDS) return layer;
    const [first, ...rest] = layer;
    const more: MiniCard = { key: `#more-${index}`, kind: "more", nodes: rest.flatMap((card) => (card.kind === "node" ? [card.node] : [])) };
    for (const card of rest) into.set(card.key, more.key);
    return [first!, more];
  });
  const nodeOf = new Map(nodes.map((node) => [node.key, node]));
  const edges = new Map<string, MiniEdge>();
  for (const edge of layered.edges) {
    const from = into.get(edge.from) ?? edge.from;
    const to = into.get(edge.to) ?? edge.to;
    const below = nodeOf.get(edge.from);
    const above = nodeOf.get(edge.to);
    const stacked = below?.pr != null && above?.base_pr === below.pr;
    const id = `${from}>${to}`;
    edges.set(id, { from, to, stacked: (edges.get(id)?.stacked ?? false) || stacked });
  }
  const stacked = new Set([...edges.entries()].flatMap(([id, edge]) => (edge.stacked ? [id] : [])));
  return { done, graph: { layers, edges: [...edges.values()].map(({ from, to }) => ({ from, to })), unrelated: [] }, stacked };
}

/**
 * The missions whose first plan landed between two snapshots this page
 * received (PRD mission-pane-chip D-06, B12-B14): one that went from no
 * nodes to some, or first appeared with nodes. With no snapshot before, as
 * after a load or a reconnect, nothing landed.
 */
export function firstPlans(before: SnapshotRest | null | undefined, after: SnapshotRest | null | undefined): Mission[] {
  const previous = before?.missions;
  if (!previous || previous.failed) return [];
  const known = new Map(previous.items.map((mission) => [mission.id, mission]));
  return (after?.missions?.items ?? []).filter((mission) => mission.nodes.length > 0 && (known.get(mission.id)?.nodes.length ?? 0) === 0);
}

/** Done nodes fold away once a mission has more than this many (B19). */
export const FOLD_DONE_OVER = 12;

/** The nodes the map draws: every one, or with done ones folded when the plan is long and the operator has not opened them. */
export function shownNodes(mission: Mission, unfolded: boolean): { nodes: MissionNode[]; folded: number } {
  if (unfolded || mission.nodes.length <= FOLD_DONE_OVER) return { nodes: mission.nodes, folded: 0 };
  const nodes = mission.nodes.filter((node) => node.status !== "done");
  return { nodes, folded: mission.nodes.length - nodes.length };
}

/**
 * The agent a takeover starts (D-40, B44): the gone Conductor's own kind when
 * Hide can start it, else the start panel's remembered one, else the first
 * kind Hide starts.
 */
export function takeOverProvider(mission: Mission, remembered: AgentStartChoice | undefined): ProviderKind {
  const kinds = PROVIDER_KINDS as readonly string[];
  if (kinds.includes(mission.conductor.agent_kind)) return mission.conductor.agent_kind as ProviderKind;
  if (remembered?.kind && kinds.includes(remembered.kind)) return remembered.kind as ProviderKind;
  const first = PROVIDER_KINDS[0];
  if (!first) throw new Error("No agent kind Hide can start");
  return first;
}

/**
 * The first prompt of a takeover: the guide and the command that make the new
 * pane the mission's Conductor. It names the mission by id only: the title can
 * come from an issue someone else wrote, and the new agent reads it with
 * `hide mission show` as the record's data rather than as its instructions.
 */
export function takeOverPrompt(mission: Mission): string {
  return `Read \`hide mission help\` and follow it as this mission's Conductor: take over mission ${mission.id}. Run \`hide mission take ${mission.id}\`, then \`hide mission show\` and \`hide inbox\`, read each worker's state, and go on from there.`;
}
