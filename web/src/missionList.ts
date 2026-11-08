// The mission screen (PRD mission-screen D-06, B1, B3-B5, B10; PRD
// mission-tool B5-B7): which missions a device lists, the group each stands
// in, the project chips and the sidebar row's number. Every state is the
// core's mission record; this module only sorts it, and what waits on the
// operator is `turnItems`, the same rule the map's `내 차례` draws, so the
// table, the number and the map never disagree.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { attentionReasonText, heldLettersText, liveNodes, missionCounts, missionOpen, turnItems, type EscalationCause, type TurnItem, type WorkerEscalation } from "./missions";
import type { Mission, MissionHeldLetters, MissionBrief, MissionNodeBrief, MissionStatus, Workspace } from "./snapshot";

/** The list's groups, top down (B5); a group with no mission is not drawn. */
export const MISSION_GROUPS = ["turn", "running", "merge_wait", "closed"] as const;
export type MissionGroup = (typeof MISSION_GROUPS)[number];

/** How many closed missions the list shows before `모두 ›` (B7). */
export const CLOSED_SHOWN = 3;

/** The missions a device works on (D-06), every project of it. */
export function deviceMissions<M extends Pick<Mission, "device_id">>(missions: readonly M[], deviceId: string): M[] {
  return missions.filter((mission) => mission.device_id === deviceId);
}

/** A project chip (B3): the catalog project a mission belongs to, by its id, and the name it shows. */
export type MissionProject = { id: string; label: string };

/**
 * The chips over the table (B3): each project one of `missions` belongs to,
 * once, by name. A mission whose project the catalog does not hold is named
 * by its path's last folder, so its chip still narrows to it.
 */
export function missionProjects(missions: readonly Pick<Mission, "workspace_id" | "project">[], catalog: readonly Pick<Workspace, "id" | "label">[]): MissionProject[] {
  const seen = new Map<string, MissionProject>();
  for (const mission of missions) {
    if (seen.has(mission.workspace_id)) continue;
    const label = catalog.find((row) => row.id === mission.workspace_id)?.label ?? mission.project.split(/[\\/]/).filter(Boolean).at(-1) ?? mission.project;
    seen.set(mission.workspace_id, { id: mission.workspace_id, label });
  }
  return [...seen.values()].sort((a, b) => a.label.localeCompare(b.label));
}

/**
 * The missions the list shows in its order (B10, B11): the groups top down,
 * 끝남 cut to its newest three until `모두 ›`, so ↑ and ↓ walk what is drawn.
 */
export function listedMissions<M extends MissionBrief>(groups: readonly { group: MissionGroup; entries: MissionEntry<M>[] }[], closedAll: boolean): MissionEntry<M>[] {
  return groups.flatMap(({ group, entries }) => (group === "closed" && !closedAll ? entries.slice(0, CLOSED_SHOWN) : entries));
}

/** One mission as the list draws it: its group and, open, what waits on the operator. The phone's trimmed mission is one too (`MissionBrief`). */
export type MissionEntry<M extends MissionBrief = Mission> = { mission: M; group: MissionGroup; turn: TurnItem<M["nodes"][number]>[] };

/**
 * A mission's group (B5): an open mission with anything in its `내 차례`
 * stands there, so the group's count is the sidebar row's number (B1); else a
 * plan or a running mission is 진행 중, a merge wait with nothing for the
 * operator is 머지 기다림, and a finished or cancelled one is 끝남.
 */
export function missionEntry<M extends MissionBrief>(mission: M, escalationOf: (node: M["nodes"][number]) => WorkerEscalation | null): MissionEntry<M> {
  const turn = turnItems(mission, escalationOf);
  if (!missionOpen(mission)) return { mission, group: "closed", turn };
  if (turn.length > 0) return { mission, group: "turn", turn };
  return { mission, group: mission.status === "merge_wait" ? "merge_wait" : "running", turn };
}

/**
 * The list in its groups (B5-B7): open missions newest first in each group,
 * closed ones by when they closed, newest first. Groups with none are absent.
 */
export function groupMissions<M extends MissionBrief>(entries: readonly MissionEntry<M>[]): { group: MissionGroup; entries: MissionEntry<M>[] }[] {
  const closedAt = (mission: MissionBrief) => mission.closed_at_unix_ms ?? mission.created_at_unix_ms;
  return MISSION_GROUPS.flatMap((group) => {
    const rows = entries.filter((entry) => entry.group === group);
    if (rows.length === 0) return [];
    const sorted = [...rows].sort((a, b) => (group === "closed" ? closedAt(b.mission) - closedAt(a.mission) : b.mission.created_at_unix_ms - a.mission.created_at_unix_ms));
    return [{ group, entries: sorted }];
  });
}

/** The sidebar row's number (B1): open missions with anything in their `내 차례`. */
export function turnCount(entries: readonly Pick<MissionEntry<MissionBrief>, "group">[]): number {
  return entries.filter((entry) => entry.group === "turn").length;
}

/** The state pill's word (B6): what waits on the operator, `!` in the decision colour, else the mission's own state. */
export function missionPillKey(mission: Pick<Mission, "status">, turn: Pick<TurnItem<MissionNodeBrief>, "kind"> & { git?: boolean } | null): MessageKey {
  if (turn) return turn.kind === "landing" && turn.git ? "missionScreen.pill.join" : TURN_PILL[turn.kind];
  return STATE_PILL[mission.status];
}

const TURN_PILL: Record<TurnItem["kind"], MessageKey> = {
  decision: "missionScreen.pill.decision",
  takeover: "missionScreen.pill.takeover",
  letters: "missionScreen.pill.letters",
  attention: "mission.attention",
  worker_blocked: "missionScreen.pill.blocked",
  landing: "missionScreen.pill.merge",
};

/** A mission state's pill word, the same on the map's head and in both lists. */
export const STATE_PILL: Record<MissionStatus, MessageKey> = {
  planning: "mission.pill.planning",
  running: "mission.pill.running",
  merge_wait: "mission.pill.mergeWait",
  done: "mission.pill.done",
  cancelled: "mission.pill.cancelled",
};

/**
 * The row's one line (B6): what waits on the operator, else the node at work,
 * else the plan being made. The desktop's mission screen and the phone's list
 * word and draw the same decision, so a mission never reads one way on one
 * screen and another on the other.
 */
export type RowLine<N extends MissionNodeBrief = MissionNodeBrief> =
  | { kind: "decision"; text: string }
  | { kind: "takeover"; left: number }
  | { kind: "letters"; letters: MissionHeldLetters }
  | { kind: "worker_blocked"; node: N; cause: EscalationCause; line: string | null }
  | { kind: "attention"; node: N; attention: NonNullable<N["attention"]> }
  | { kind: "landing"; nodes: N[]; join: boolean }
  | { kind: "planning" }
  | { kind: "now"; first: N; others: number };

export function rowLine<M extends MissionBrief>(mission: M, turn: TurnItem<M["nodes"][number]> | null): RowLine<M["nodes"][number]> | null {
  if (turn) {
    switch (turn.kind) {
      case "decision":
        return { kind: "decision", text: mission.decision ?? "" };
      case "takeover": {
        const { done, total } = missionCounts(mission);
        return { kind: "takeover", left: total - done };
      }
      case "letters":
        return { kind: "letters", letters: turn.letters };
      case "worker_blocked":
        return { kind: "worker_blocked", node: turn.node, cause: turn.cause, line: turn.line };
      case "attention":
        return { kind: "attention", node: turn.node, attention: turn.attention };
      case "landing":
        return { kind: "landing", nodes: turn.nodes, join: turn.git };
    }
  }
  if (mission.status === "planning" && mission.nodes.length === 0) return { kind: "planning" };
  const live = liveNodes(mission);
  const [first] = live;
  return first ? { kind: "now", first, others: live.length - 1 } : null;
}

/**
 * The row's one line in words (B6; PRD mobile-companion-ui B31), the same on
 * both lists: the desktop draws marks around some of it (the worker's mark
 * before `지금:`, each change's PR mark in `머지 순서`), the phone the words.
 */
export function rowLineText<N extends MissionNodeBrief>(line: RowLine<N>, t: TFunction<"translation">): string {
  switch (line.kind) {
    case "decision":
      return line.text;
    // The pill already says no Conductor holds it; the line says what is left for the one who takes it.
    case "takeover":
      return line.left > 0 ? t("missionScreen.takeoverLeft", { count: line.left }) : t("missionScreen.takeoverReport");
    case "letters":
      return heldLettersText(line.letters, t);
    case "worker_blocked":
      return `${line.node.title} · ${line.line ?? t(`mission.turn.cause.${line.cause}`)}`;
    case "attention":
      return `${line.node.title} · ${attentionReasonText(line.attention.reason, line.attention.pr, t)}`;
    case "landing":
      return `${t(line.join ? "mission.turn.join" : "mission.turn.merge")} ${line.nodes.map((node) => (node.pr != null ? `#${node.pr}` : node.title)).join(" → ")}`;
    case "planning":
      return t("mission.planning");
    case "now":
      return t("missionScreen.now", { title: line.others > 0 ? t("mission.line.others", { title: line.first.title, count: line.others }) : line.first.title });
  }
}
