// What the phone's mission screens read of the record (PRD mobile-companion-ui
// B28, B31-B36): the groups and the badge are the desktop Mission tool's own
// rules (`missionList.ts`) run over the missions hided sent, and a mission
// page's steps are the map summary's own split (`summaryGroups`), so a
// mission reads the same on both screens. Pure, so every rule is tested
// without a socket.

import type { TFunction } from "i18next";
import { useMemo } from "react";
import { missionEntry, turnCount, type MissionEntry, type RowLine } from "../missionList";
import { attentionReasonText, missionCounts, summaryGroups } from "../missions";
import type { PhoneMission, PhoneMissionNode } from "./protocol";
import { usePhone } from "./store";

/** Each mission with its group and what waits on the operator; the worker's escalation is the one hided read from its row. */
export function missionEntries(missions: readonly PhoneMission[]): MissionEntry<PhoneMission>[] {
  return missions.map((mission) => missionEntry(mission, (node) => node.escalation));
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
 * A mission page's steps (B33, B36): `지금` holds what waits on the operator or
 * the Conductor and what works; `끝남` what finished. The step the decision
 * stopped, and a worker the core raised, stand in `hold` beside the nodes
 * the map marks `!`, so no step that waits on someone reads as working.
 *
 * This is not `turnItems`: that decides what waits on the operator (the
 * badge and `내 차례`), while `hold` is every step that waits on anyone, the
 * Conductor included, so the page never shows a stopped step as working.
 */
export function missionSteps(mission: PhoneMission): { hold: PhoneMissionNode[]; working: PhoneMissionNode[]; done: PhoneMissionNode[] } {
  const groups = summaryGroups(mission);
  const stopped = mission.decision ? mission.question?.node ?? null : null;
  const waiting = (node: PhoneMissionNode) => node.key === stopped || (node.status === "working" && node.escalation !== null);
  const hold = [...groups.attention, ...groups.working.filter(waiting)];
  return { hold, working: groups.working.filter((node) => !waiting(node)), done: groups.done };
}

/** The progress words and counts of the head: `n / m` of the steps in the plan. */
export function progress(mission: PhoneMission): { done: number; total: number } {
  const { done, total } = missionCounts(mission);
  return { done, total };
}

/** The row's one line as text (B31): what `rowLine` decides, worded for a phone's row. */
export function rowLineText(line: RowLine<PhoneMissionNode>, t: TFunction<"translation">): string {
  switch (line.kind) {
    case "decision":
      return line.text;
    case "takeover":
      return line.left > 0 ? t("missionTool.takeoverLeft", { count: line.left }) : t("missionTool.takeoverReport");
    case "worker_blocked":
      return `${line.node.title} · ${line.line ?? t(`mission.turn.cause.${line.cause}`)}`;
    case "attention":
      return `${line.node.title} · ${attentionReasonText(line.attention.reason, line.attention.pr, t)}`;
    case "landing":
      return `${t(line.join ? "mission.turn.join" : "mission.turn.merge")} ${line.nodes.map((node) => (node.pr != null ? `#${node.pr}` : node.title)).join(" → ")}`;
    case "planning":
      return t("mission.planning");
    case "now":
      return t("missionTool.now", { title: line.others > 0 ? t("mission.line.others", { title: line.first.title, count: line.others }) : line.first.title });
  }
}
