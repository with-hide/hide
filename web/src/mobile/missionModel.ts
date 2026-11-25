// What the phone's mission screens read of the record (PRD mobile-companion-ui
// B28, B31-B36): the groups and the badge are the desktop Mission tool's own
// rules (`missionList.ts`) run over the missions hided sent, and a mission
// page's steps use the shared desktop graph, so a
// mission reads the same on both screens. Pure, so every rule is tested
// without a socket.

import type { TFunction } from "i18next";
import { useMemo } from "react";
import { missionEntry, turnCount, type MissionEntry } from "../missionList";
import {
  attentionReasonText, cardGate, escalationWhy, holdReason, missionCounts, missionOpen, nodeLook, nodePace, openAttention, pullsByNumber, workerAsleep, type MissionPulls,
  type NodeLook, type NodePace, type WorkerEscalation,
} from "../missions";
import type { LineNode } from "../missionLine";
import type { MissionHeldNode } from "../snapshot";
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

/** The missions tab's badge (B28): open missions with anything in their `내 차례`; 0 draws none. */
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
      : node.kind === "wait" && node.pr != null ? { pr: node.pr } : null;
  const outside = node.external_base != null || node.external_state != null || node.external_reference != null ? { base: node.external_base, state: node.external_state, reference: node.external_reference } : null;
  const base = node.base_pr == null ? null : nodes.find((one) => one.pr === node.base_pr);
  return { ...node, external, outside, pr_base: node.pr_base ?? null, pr_more: node.pr_more ?? 0, base_state: base?.pr_state ?? null };
}

/** The references a phone mission's steps carry, by number, and a held cell's, so a reason names a gitlab.com merge request `!12`. */
export function phonePulls(mission: PhoneMission, held?: Pick<MissionHeldNode, "pr" | "reference">): MissionPulls {
  return pullsByNumber([...mission.nodes.map((node) => phoneLineNode(node)), ...(held ? [{ attention: held }] : [])]);
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

/** Where a step stands as the phone says it, read once for the map's card and the step list's row (Doop 14). */
export type StepState = {
  look: NodeLook;
  pace: NodePace | null;
  gate: ReturnType<typeof cardGate<NonNullable<PhoneMissionNode["gate"]>>>;
  attention: ReturnType<typeof openAttention<NonNullable<PhoneMissionNode["attention"]>>>;
  /** The step waits on the operator for something beyond its merge: a gate reached, a raised reason, a decision or the worker's turn. */
  turn: boolean;
  /** The core says merging it is the operator's now (D-53); the card's band draws it, the list's row counts it as the operator's turn. */
  mergeDue: boolean;
  /** Stopped, in its words, without the cause or the time, which stay on the Mac (operator decision 2026-11-13). */
  stopped: string | null;
  /** A worker Hide put to sleep, said asleep and never a wait on it (letters-missions-simplify B23). */
  asleep: string | null;
  /** Why it waits on someone, in the desktop's words. */
  why: string | null;
  /** The step's state word, as the card draws it: stopped, else its look. */
  word: string;
  /** Its worker is at work now, so its time since it went to work means something. */
  working: boolean;
};

export function stepState(mission: PhoneMission, node: PhoneMissionNode, t: TFunction<"translation">): StepState {
  const look = nodeLook(mission, node);
  const pace = nodePace(mission, node);
  const reason = holdReason(mission, node, workerOf);
  const attention = openAttention(mission, node);
  const gate = cardGate(node.gate);
  const stopped = pace === "stopped" ? (node.work?.state === "no_worker" ? t("mission.work.stopped", { why: t("mission.work.stop.noWorker") }) : t("mission.work.stoppedPlain")) : null;
  const asleep = pace === "waiting" && (node.work?.state === "asleep" || workerAsleep(node)) ? t("mission.worker.asleep") : null;
  const why = reason?.kind === "attention"
    ? `${reason.attention.to === "conductor" ? `${t("mission.attention.to.conductor")} · ` : ""}${attentionReasonText(reason.attention.reason, reason.attention.pr, t, 0, phonePulls(mission))}`
    : reason?.kind === "worker" ? escalationWhy(reason.escalation, t)
      : reason?.kind === "decision" ? t("mobile.mission.waitsDecision") : null;
  return {
    look, pace, gate, attention,
    turn: gate?.state === "open" || attention?.to === "operator" || reason?.kind === "decision" || reason?.kind === "worker",
    mergeDue: node.merge?.state === "due",
    stopped, asleep, why,
    word: stopped ?? t(`mission.look.${look}`),
    working: look === "working" && pace === "working",
  };
}

