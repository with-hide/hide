// What the phone's mission screens read of the record (PRD mobile-companion-ui
// B28, B31-B36): the groups and the badge are the desktop Mission tool's own
// rules (`missionList.ts`) run over the missions hided sent, and a mission
// page's steps use the shared desktop graph, so a
// mission reads the same on both screens. Pure, so every rule is tested
// without a socket.

import { useMemo } from "react";
import { missionEntry, turnCount, type MissionEntry } from "../missionList";
import { missionCounts, missionOpen, type WorkerEscalation } from "../missions";
import type { LineNode } from "../missionLine";
import type { AgentKey, PhoneMission, PhoneMissionNode, PhonePr } from "./protocol";
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

/** The escalation hided read from a step's worker row and sent with the node. */
export function workerOf(node: PhoneMissionNode): WorkerEscalation | null {
  return node.escalation;
}

/**
 * A step as the card's bottom line reads it (`branchLine`, `resultLine`, PRD
 * mission-destination D-52, B59): the flat fields the Mac sends the phone put
 * back in the desktop node's shape, so both screens run one computation. The
 * frame carries no branch, conclusion or verdict, so the line names none. A
 * field a Mac before destinations does not send stays absent and the line
 * names nothing for it (B60); an outside step from such a Mac still names the
 * pull request it watches, the one the frame carries as its own. The pull
 * request a stacked step goes into takes its state from the step whose current
 * pull request it is; the frame carries no older ones, so another stays grey
 * (D-57).
 */
export function phoneLineNode(node: PhoneMissionNode, nodes: readonly Pick<PhoneMissionNode, "pr" | "pr_state">[] = []): LineNode {
  const external = node.external_pr != null ? { pr: node.external_pr }
    : node.external_branch != null ? { branch: node.external_branch }
      : node.kind === "external" && node.pr != null ? { pr: node.pr } : null;
  const outside = node.external_base != null || node.external_state != null ? { base: node.external_base, state: node.external_state } : null;
  const base = node.base_pr == null ? null : nodes.find((one) => one.pr === node.base_pr);
  return { ...node, external, outside, pr_base: node.pr_base ?? null, pr_more: node.pr_more ?? 0, base_state: base?.pr_state ?? null };
}

/** The progress words and counts of the head: `n / m` of the steps in the plan. */
export function progress(mission: PhoneMission): { done: number; total: number } {
  const { done, total } = missionCounts(mission);
  return { done, total };
}

/** What an agent's detail links to: an open mission it conducts (no step) or works a step of, with whether its 내 차례 waits, and each listed pull request that names it. */
export type AgentLinks = { missions: { mission: PhoneMission; node: PhoneMissionNode | null; turn: boolean }[]; prs: PhonePr[] };

const sameAgent = (a: AgentKey | null, b: AgentKey) => a !== null && a.device_id === b.device_id && a.pane_id === b.pane_id;

/**
 * The links of one agent, by its exact row only (the Conductor's or a step worker's row, a pull request's named agents): a mission
 * or a pull request that merely shares its checkout is not one. A finished mission links nothing; a pull request links while the
 * PR tab lists it.
 */
export function agentLinks(key: AgentKey, missions: readonly PhoneMission[] | null, prs: readonly PhonePr[] | null): AgentLinks {
  const linked: AgentLinks["missions"] = [];
  for (const mission of missions ?? []) {
    if (!missionOpen(mission)) continue;
    const node = sameAgent(mission.conductor.agent, key) ? null : mission.nodes.find((each) => sameAgent(each.agent, key));
    if (node === undefined) continue;
    linked.push({ mission, node, turn: missionEntry(mission, workerOf).turn.length > 0 });
  }
  return { missions: linked, prs: (prs ?? []).filter((pr) => pr.agents.some((agent) => sameAgent(agent, key))) };
}
