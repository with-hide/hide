// What the phone's mission screens read of the record (PRD mobile-companion-ui
// B28, B31-B36): the groups and the badge are the desktop Mission tool's own
// rules (`missionList.ts`) run over the missions hided sent, and a mission
// page's steps are the map summary's own split (`summaryGroups`), so a
// mission reads the same on both screens. Pure, so every rule is tested
// without a socket.

import { useMemo } from "react";
import { missionEntry, turnCount, type MissionEntry } from "../missionList";
import { missionCounts, summaryGroups, type WorkerEscalation } from "../missions";
import type { PhoneMission, PhoneMissionNode } from "./protocol";
import { usePhone } from "./store";

/** Each mission with its group and what waits on the operator; the worker's escalation is the one hided read from its row. */
export function missionEntries(missions: readonly PhoneMission[]): MissionEntry<PhoneMission>[] {
  return missions.map((mission) => missionEntry(mission, workerOf));
}

/** The entries of the current record; null until the Mac has sent it. */
export function useMissionEntries(): MissionEntry<PhoneMission>[] | null {
  const missions = usePhone((s) => s.missions);
  return useMemo(() => (missions ? missionEntries(missions) : null), [missions]);
}

/** The missions tab's badge (B28): open missions with anything in their `내 차례`, as the desktop's rail counts them; 0 draws none. */
export function useMissionBadge(): number {
  const entries = useMissionEntries();
  return entries ? turnCount(entries) : 0;
}

/** The subtitle's number: missions still the Conductor's to carry. */
export function openCount(entries: readonly MissionEntry<PhoneMission>[]): number {
  return entries.filter((entry) => entry.group !== "closed").length;
}

/**
 * A mission page's steps (B33, B36): `지금` holds what waits on someone and
 * what works, `끝남` what finished. `hold` is the map summary's 확인 필요
 * (`summaryGroups`), so a step the decision stopped or whose worker the core
 * raised waits on both screens and never reads as working. This is not
 * `turnItems`: that decides what waits on the operator (the badge and
 * `내 차례`), while `hold` is every step that waits on anyone, the Conductor
 * included.
 */
export function missionSteps(mission: PhoneMission): { hold: PhoneMissionNode[]; working: PhoneMissionNode[]; done: PhoneMissionNode[] } {
  const { attention, working, done } = summaryGroups(mission, workerOf);
  return { hold: attention, working, done };
}

/** The escalation hided read from a step's worker row and sent with the node. */
export function workerOf(node: PhoneMissionNode): WorkerEscalation | null {
  return node.escalation;
}

/** The progress words and counts of the head: `n / m` of the steps in the plan. */
export function progress(mission: PhoneMission): { done: number; total: number } {
  const { done, total } = missionCounts(mission);
  return { done, total };
}
