// What the mission map and the Conductor row read from a mission (PRD
// software-factory-conductor): the core decides every state and number
// (`MissionView`), so this module only finds a mission, words its parts
// and counts what the record holds. Pure, so the rules test on their own;
// `MissionMap.tsx` draws them.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import { agentsByPane } from "./paneAgents";
import { orderByDependencies } from "./projectBoard";
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
      return { state: "done", key: "mission.chip", counts };
    case "cancelled":
      return { state: "cancelled", key: "mission.pane.cancelled", counts: null };
  }
}

/**
 * A node's state as the map's colour says it (PRD mission-plan-visibility
 * D-11, B14): the whole card border, the legend, the chip's cell and the flow
 * row's bar share it. A node that cannot start yet waits; once the mission is
 * cancelled, a node it never finished is cancelled, and a dropped node left
 * the plan, so it counts in neither the legend nor the chip.
 */
export type NodeTone = "done" | "working" | "review" | "hold" | "failed" | "waiting" | "cancelled" | "dropped";

export function nodeTone(mission: Mission, node: MissionNode): NodeTone {
  if (node.status === "dropped") return "dropped";
  if (mission.status === "cancelled" && node.status !== "done" && node.status !== "failed") return "cancelled";
  if (node.blocked) return "waiting";
  switch (node.status) {
    case "done":
      return "done";
    case "working":
      return "working";
    case "review":
      return "review";
    case "held":
      return "hold";
    case "failed":
      return "failed";
    case "planned":
    case "waiting":
      return "waiting";
  }
}

/** The legend's order (B13): what finished, what moves, what waits on the operator, what is still ahead. */
export const LEGEND_TONES = ["done", "review", "working", "hold", "failed", "waiting", "cancelled"] as const satisfies readonly NodeTone[];

export type LegendTone = (typeof LEGEND_TONES)[number];

/** The head's legend (B13): each state with nodes in it and how many, in `LEGEND_TONES` order. */
export function missionLegend(mission: Mission): { tone: LegendTone; count: number }[] {
  const counts = new Map<NodeTone, number>();
  for (const node of mission.nodes) counts.set(nodeTone(mission, node), (counts.get(nodeTone(mission, node)) ?? 0) + 1);
  return LEGEND_TONES.flatMap((tone) => (counts.get(tone) ? [{ tone, count: counts.get(tone)! }] : []));
}

/** The nodes top down by what they wait on, a dropped node where it stood (B17). */
export function orderedNodes(mission: Mission): MissionNode[] {
  return orderByDependencies(mission.nodes, (node) => node.key, (node) => node.depends_on).map(({ item }) => item);
}

/** The chip's and the head's bar (B8, B13): one cell per node in the plan, in order. */
export function missionCells(mission: Mission): LegendTone[] {
  return orderedNodes(mission).flatMap((node) => {
    const tone = nodeTone(mission, node);
    return tone === "dropped" ? [] : [tone];
  });
}

/** The colour a mission's own chip wears on the sidebar row and the Overview (B35): its decision, else its state. */
export function missionTone(mission: Mission): LegendTone {
  if (mission.decision && missionOpen(mission)) return "hold";
  switch (mission.status) {
    case "planning":
      return "waiting";
    case "running":
      return "working";
    case "merge_wait":
      return "review";
    case "done":
      return "done";
    case "cancelled":
      return "cancelled";
  }
}

/** The nodes a worker is at now, working or in review, in order (B9, B10). */
export function liveNodes(mission: Mission): MissionNode[] {
  return orderedNodes(mission).filter((node) => node.status === "working" || node.status === "review");
}

/** The running popover's last line (B10): the nodes done, and those still ahead of any worker. */
export function missionTally(mission: Mission): { done: number; next: number } {
  const ahead = (node: MissionNode) => node.status === "planned" || node.status === "waiting" || node.status === "held";
  return { done: mission.nodes.filter((node) => node.status === "done").length, next: mission.nodes.filter(ahead).length };
}

/**
 * The Conductor pane's second line (B9, D-21): the decision waiting on the
 * operator first, else `도는 중 n · <first> 외 k`; nothing with no worker at it.
 */
export function missionWorkLine(mission: Mission, t: TFunction<"translation">): { decision: boolean; text: string } | null {
  if (mission.decision && missionOpen(mission)) return { decision: true, text: `! ${mission.decision}` };
  if (!missionOpen(mission)) return null;
  const live = liveNodes(mission);
  const [first] = live;
  if (!first) return null;
  const head = t("mission.line.running", { count: live.length });
  const rest = live.length > 1 ? t("mission.line.others", { title: first.title, count: live.length - 1 }) : first.title;
  return { decision: false, text: `${head} · ${rest}` };
}

const PR_STAGES: readonly MessageKey[] = ["mission.stage.code", "mission.stage.pr", "mission.stage.review", "mission.stage.merge"];
const DIRECT_STAGES: readonly MessageKey[] = ["mission.stage.code", "mission.stage.check", "mission.stage.review", "mission.stage.main"];

/** Where a node's work is, named (B10): a change node's step of four, else its kind's one word. */
export function stageWord(node: MissionNode): MessageKey {
  if (node.kind === "research") return "mission.stage.research";
  if (node.kind === "verify") return "mission.stage.verify";
  const stages = node.delivery === "direct" ? DIRECT_STAGES : PR_STAGES;
  return stages[Math.min(Math.max(node.stage ?? 1, 1), STAGE_STEPS) - 1]!;
}

/**
 * What a node left behind, by its kind (B22, D-14): its pull request, a
 * direct change's branch, a research node's conclusion or a verify node's
 * line. A research or verify node's line is a result only once the node
 * ended (done, or a failed check); while it runs the line is its progress.
 * Null when the record holds none, so nothing is drawn for it.
 */
export type NodeResult =
  | { kind: "pr"; number: number; state: MissionNode["pr_state"] }
  | { kind: "branch"; branch: string }
  | { kind: "conclusion"; line: string }
  | { kind: "verify"; line: string };

export function nodeResult(node: MissionNode): NodeResult | null {
  if (node.pr != null) return { kind: "pr", number: node.pr, state: node.pr_state };
  if (node.kind === "change") return node.branch ? { kind: "branch", branch: node.branch } : null;
  if (!node.line) return null;
  if (node.kind === "research") return node.status === "done" ? { kind: "conclusion", line: node.line } : null;
  return node.status === "done" || node.status === "failed" ? { kind: "verify", line: node.line } : null;
}

/** A finished or cancelled mission's results (B11, B23, B24): each done node that left something, in order. */
export function missionResults(mission: Mission): { node: MissionNode; result: NodeResult }[] {
  return orderedNodes(mission).flatMap((node) => {
    const result = node.status === "done" ? nodeResult(node) : null;
    return result ? [{ node, result }] : [];
  });
}

/** The flow table's time axis (B17): the mission's start to its close, or to now while it is open. */
export function flowSpan(mission: Mission, now: number): { from: number; to: number } {
  const from = mission.started_at_unix_ms ?? mission.created_at_unix_ms;
  return { from, to: Math.max(from + 1, mission.closed_at_unix_ms ?? now) };
}

/**
 * A node's one bar on that axis, as percentages (B19): from its start to its
 * finish, or to the axis end while it has not finished; none before it starts.
 */
export function nodeBar(node: MissionNode, span: { from: number; to: number }): { left: number; width: number } | null {
  if (node.started_at_unix_ms == null) return null;
  const length = span.to - span.from;
  const start = Math.min(Math.max(node.started_at_unix_ms, span.from), span.to);
  const end = Math.min(Math.max(node.finished_at_unix_ms ?? span.to, start), span.to);
  return { left: ((start - span.from) / length) * 100, width: ((end - start) / length) * 100 };
}

/** The flow table's filter (B17): every node, the ones not finished, or the finished ones. */
export type FlowFilter = "all" | "now" | "finished";

export function flowShows(node: MissionNode, filter: FlowFilter): boolean {
  const finished = node.status === "done" || node.status === "failed" || node.status === "dropped";
  return filter === "all" || (filter === "finished") === finished;
}

/**
 * The missions whose first plan landed between two snapshots this page
 * received (PRD mission-pane-chip D-06, B12-B14): one that went from no
 * nodes to some, or first appeared with nodes. With no snapshot before, as
 * after a load or a reconnect, nothing landed.
 */
export function firstPlans(before: SnapshotRest | null | undefined, after: SnapshotRest | null | undefined): Mission[] {
  const previous = before?.missions;
  // A frame that left the missions alone keeps their section as it was.
  if (!previous || previous.failed || previous === after?.missions) return [];
  const known = new Map(previous.items.map((mission) => [mission.id, mission]));
  return (after?.missions?.items ?? []).filter((mission) => mission.nodes.length > 0 && (known.get(mission.id)?.nodes.length ?? 0) === 0);
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
