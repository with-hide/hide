// What the mission map and the Conductor row read from a mission (PRD
// software-factory-conductor): the core decides every state and number
// (`MissionView`), so this module only finds a mission, words its parts
// and counts what the record holds. Pure, so the rules test on their own;
// `MissionMap.tsx` draws them.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { PROVIDER_KINDS, type ProviderKind } from "./agentPicker";
import type { AgentStartChoice, Mission, MissionNode, RowMark, SnapshotRest } from "./snapshot";

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
 * The card's last line (B15, B51, B53): why it waits on an earlier node,
 * else what its worker last said, else its kind. A node that waits for a
 * node not done yet names that node.
 */
export function nodeLine(mission: Mission, node: MissionNode, t: TFunction<"translation">): string {
  if (node.blocked === "upstream_failed") return t("mission.blocked.failed");
  if (node.blocked === "upstream_dropped") return t("mission.blocked.dropped");
  if (node.line) return node.line;
  if (node.status === "planned" && mission.status !== "planning") {
    const waiting = node.depends_on.map((key) => mission.nodes.find((row) => row.key === key)).find((row) => row && row.status !== "done");
    if (waiting) return t("mission.waitingFor", { title: waiting.title });
  }
  return t(kindWord(node.kind));
}

/** A worker as the card names it: its model (or agent) and its effort, `자동` while the Conductor will choose it. */
export function workerWords(node: MissionNode, auto: string): string | null {
  const worker = node.worker;
  if (!worker) return null;
  const who = worker.model ?? worker.agent;
  const effort = worker.effort ?? (worker.effort_auto ? auto : null);
  return effort ? `${who}·${effort}` : who;
}

/** A change node's steps: work, PR or branch, review, merge or main (D-42 B, D-48). */
export const STAGE_STEPS = 4;

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

/** The first prompt of a takeover: the skill and the command that make the new pane the mission's Conductor. */
export function takeOverPrompt(mission: Mission): string {
  return `Use the hide-conductor skill: take over mission ${mission.id} ("${mission.title}"). Run \`hide mission take ${mission.id}\`, then \`hide mission show\` and \`hide inbox\`, read each worker's state, and go on from there.`;
}
