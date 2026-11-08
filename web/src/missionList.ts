// The rail's Mission tool (PRD mission-tool D-01, D-08, D-09, B2, B5-B7):
// which missions it lists, the group each stands in and the rail's number.
// Every state is the core's mission record; this module only sorts it, and
// what waits on the operator is `turnItems`, the same rule the map's
// `내 차례` draws, so the list, the number and the map never disagree.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { attentionReasonText, heldLettersText, liveNodes, missionCounts, missionOpen, turnItems, type EscalationCause, type TurnItem, type WorkerEscalation } from "./missions";
import type { DetailSlot, Mission, MissionHeldLetters, MissionBrief, MissionNodeBrief, MissionStatus, Workspace } from "./snapshot";

/** The list's groups, top down (B5); a group with no mission is not drawn. */
export const MISSION_GROUPS = ["turn", "running", "merge_wait", "closed"] as const;
export type MissionGroup = (typeof MISSION_GROUPS)[number];

/** How many closed missions the list shows before `모두 ›` (B7). */
export const CLOSED_SHOWN = 3;

/**
 * The missions of the front Workspace's project (D-09): those whose catalog
 * project is it, or that were made in its main worktree on its device, in
 * any of its checkouts.
 */
export function projectMissions(missions: readonly Mission[], project: Pick<Workspace, "id" | "path" | "device_id">): Mission[] {
  return missions.filter((mission) => mission.workspace_id === project.id || (mission.device_id === project.device_id && mission.project === project.path));
}

/** One mission as the list draws it: its group and, open, what waits on the operator. The phone's trimmed mission is one too (`MissionBrief`). */
export type MissionEntry<M extends MissionBrief = Mission> = { mission: M; group: MissionGroup; turn: TurnItem<M["nodes"][number]>[] };

/**
 * A mission's group (B5): an open mission with anything in its `내 차례`
 * stands there, so the group's count is the rail's number (D-08); else a
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

/** The rail's number (D-08, B2): open missions with anything in their `내 차례`. */
export function turnCount(entries: readonly Pick<MissionEntry<MissionBrief>, "group">[]): number {
  return entries.filter((entry) => entry.group === "turn").length;
}

/** The mission the 보기 slot shows now, whichever place opened it (B6, B23); null while it shows anything else. */
export function shownMission(detail: DetailSlot | null | undefined): string | null {
  const top = detail?.shown ? detail.items.at(-1) : null;
  return top?.kind === "mission" ? top.mission : null;
}

/** The state pill's word (B6): what waits on the operator, `!` in the decision colour, else the mission's own state. */
export function missionPillKey(mission: Pick<Mission, "status">, turn: Pick<TurnItem<MissionNodeBrief>, "kind"> & { git?: boolean } | null): MessageKey {
  if (turn) return turn.kind === "landing" && turn.git ? "missionTool.pill.join" : TURN_PILL[turn.kind];
  return STATE_PILL[mission.status];
}

const TURN_PILL: Record<TurnItem["kind"], MessageKey> = {
  decision: "missionTool.pill.decision",
  takeover: "missionTool.pill.takeover",
  letters: "missionTool.pill.letters",
  attention: "mission.attention",
  worker_blocked: "missionTool.pill.blocked",
  landing: "missionTool.pill.merge",
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
 * else the plan being made. The desktop's Mission tool and the phone's list
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
      return line.left > 0 ? t("missionTool.takeoverLeft", { count: line.left }) : t("missionTool.takeoverReport");
    case "letters":
      return heldLettersText(line.letters, t);
    case "worker_blocked":
      return `${line.node.title} · ${line.line ?? t(`mission.turn.cause.${line.cause}`)}`;
    case "attention":
      return `${line.node.title} · ${attentionReasonText(line.attention.reason, line.attention.pr, t, line.attention.unmet_nodes)}`;
    case "landing":
      return `${t(line.join ? "mission.turn.join" : "mission.turn.merge")} ${line.nodes.map((node) => (node.pr != null ? `#${node.pr}` : node.title)).join(" → ")}`;
    case "planning":
      return t("mission.planning");
    case "now":
      return t("missionTool.now", { title: line.others > 0 ? t("mission.line.others", { title: line.first.title, count: line.others }) : line.first.title });
  }
}
