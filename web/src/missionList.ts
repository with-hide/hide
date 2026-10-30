// The rail's Mission tool (PRD mission-tool D-01, D-08, D-09, B2, B5-B7):
// which missions it lists, the group each stands in and the rail's number.
// Every state is the core's mission record; this module only sorts it, and
// what waits on the operator is `turnItems`, the same rule the map's
// `내 차례` draws, so the list, the number and the map never disagree.

import { missionOpen, turnItems, type TurnItem } from "./missions";
import type { AgentRow, DetailSlot, Mission, MissionNode, Workspace } from "./snapshot";

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

/** One mission as the list draws it: its group and, open, what waits on the operator. */
export type MissionEntry = { mission: Mission; group: MissionGroup; turn: TurnItem[] };

/**
 * A mission's group (B5): an open mission with anything in its `내 차례`
 * stands there, so the group's count is the rail's number (D-08); else a
 * plan or a running mission is 진행 중, a merge wait with nothing for the
 * operator is 머지 기다림, and a finished or cancelled one is 끝남.
 */
export function missionEntry(mission: Mission, workerRow: (node: MissionNode) => AgentRow | null): MissionEntry {
  const turn = turnItems(mission, workerRow);
  if (!missionOpen(mission)) return { mission, group: "closed", turn };
  if (turn.length > 0) return { mission, group: "turn", turn };
  return { mission, group: mission.status === "merge_wait" ? "merge_wait" : "running", turn };
}

/**
 * The list in its groups (B5-B7): open missions newest first in each group,
 * closed ones by when they closed, newest first. Groups with none are absent.
 */
export function groupMissions(entries: readonly MissionEntry[]): { group: MissionGroup; entries: MissionEntry[] }[] {
  const closedAt = (mission: Mission) => mission.closed_at_unix_ms ?? mission.created_at_unix_ms;
  return MISSION_GROUPS.flatMap((group) => {
    const rows = entries.filter((entry) => entry.group === group);
    if (rows.length === 0) return [];
    const sorted = [...rows].sort((a, b) => (group === "closed" ? closedAt(b.mission) - closedAt(a.mission) : b.mission.created_at_unix_ms - a.mission.created_at_unix_ms));
    return [{ group, entries: sorted }];
  });
}

/** The rail's number (D-08, B2): open missions with anything in their `내 차례`. */
export function turnCount(entries: readonly MissionEntry[]): number {
  return entries.filter((entry) => entry.group === "turn").length;
}

/** The mission the 보기 slot shows now, whichever place opened it (B6, B23); null while it shows anything else. */
export function shownMission(detail: DetailSlot | null | undefined): string | null {
  const top = detail?.shown ? detail.items.at(-1) : null;
  return top?.kind === "mission" ? top.mission : null;
}
