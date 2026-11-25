// An item that is its own Needs You line, with no agent row to carry what
// waits on the operator: a mission's (PRD mission-node-simple D-19, B14), an
// automation stopped for a person to fix (PRD automations D-12, B23), or a
// once automation whose run ended and waits to be opened (mission-104 S-4). The
// device's scope names both and counts them in its Needs You number, so each
// list that draws the number draws these rows with its agents.

import type { MissionAsk } from "./missions";
import type { Automation, AutomationPauseReason, AutomationRecord, AutomationsSection, SnapshotRest } from "./snapshot";

/** An automation stopped for a person to fix (B23): why, since when, and the machine that runs it. */
export type AutomationAsk = { automation: Automation; reason: AutomationPauseReason; since_unix_ms: number; device_id: string };

/** A once automation whose run ended (S-4): the run (`look_run`), with the agent's answer, when it ended, and the machine that runs it. */
export type AutomationLook = { automation: Automation; run: Extract<AutomationRecord, { kind: "run" }> | null; since_unix_ms: number; device_id: string };

export type OwnAsk = { kind: "mission"; ask: MissionAsk } | { kind: "automation"; ask: AutomationAsk } | { kind: "automation_look"; ask: AutomationLook };

export function missionOwnAsks(asks: readonly MissionAsk[]): OwnAsk[] {
  return asks.map((ask) => ({ kind: "mission", ask }));
}

export function automationOwnAsks(asks: readonly AutomationAsk[], looks: readonly AutomationLook[] = []): OwnAsk[] {
  return [...asks.map((ask): OwnAsk => ({ kind: "automation", ask })), ...looks.map((ask): OwnAsk => ({ kind: "automation_look", ask }))];
}

/** When it began to wait, which orders it among the unread asks. */
export function ownAskSince(own: OwnAsk): number {
  return own.ask.since_unix_ms;
}

export function ownAskDevice(own: OwnAsk): string {
  return own.kind === "mission" ? own.ask.mission.device_id : own.ask.device_id;
}

/** The project it stands under in 프로젝트별: a mission's; an automation's none, since it is the machine's own work. */
export function ownAskProject(own: OwnAsk): string | null {
  return own.kind === "mission" ? own.ask.mission.workspace_id : null;
}

export function ownAskKey(own: OwnAsk): string {
  if (own.kind === "mission") return `mission:${own.ask.mission.id}`;
  return `${own.kind}:${own.ask.automation.id}`;
}

/**
 * The automations `ids` names (a scope's `automation_asks`) on `deviceId`, in
 * its order, from this core's automations; one this window's core does not
 * keep, or one no longer stopped for a person, is left out.
 */
export function automationAsks(section: AutomationsSection | null, ids: readonly string[], deviceId: string): AutomationAsk[] {
  if (!section || ids.length === 0) return [];
  const byId = new Map(section.items.map((automation) => [automation.id, automation]));
  return ids.flatMap((id): AutomationAsk[] => {
    const automation = byId.get(id);
    const paused = automation?.paused;
    if (!automation || !paused || paused.reason === "operator") return [];
    return [{ automation, reason: paused.reason, since_unix_ms: paused.at_unix_ms, device_id: deviceId }];
  });
}

/**
 * The once automations `ids` names (a scope's `automation_looks`) on
 * `deviceId`, in its order, from this core's automations, each with the run
 * that waits to be opened; one this window's core does not keep, or whose run
 * no longer waits, is left out.
 */
export function automationLooks(section: AutomationsSection | null, ids: readonly string[], deviceId: string): AutomationLook[] {
  if (!section || ids.length === 0) return [];
  const byId = new Map(section.items.map((automation) => [automation.id, automation]));
  return ids.flatMap((id): AutomationLook[] => {
    const automation = byId.get(id);
    if (!automation || automation.look_run === undefined) return [];
    const run = automation.records.find((record): record is Extract<AutomationRecord, { kind: "run" }> => record.kind === "run" && record.number === automation.look_run) ?? null;
    return [{ automation, run, since_unix_ms: run ? (run.ended_at_unix_ms ?? run.started_at_unix_ms) : 0, device_id: deviceId }];
  });
}

/** Every device's once lines from one snapshot, `frontId`'s first: the one loop the Overview and the window's notices read. */
export function everyDeviceLooks(rest: SnapshotRest | null, section: AutomationsSection | null, frontId?: string): AutomationLook[] {
  const devices = rest?.navigator?.devices ?? [];
  const ordered = frontId === undefined ? devices : [...devices.filter((device) => device.id === frontId), ...devices.filter((device) => device.id !== frontId)];
  return ordered.flatMap((device) => automationLooks(section, device.agent_scope.automation_looks ?? [], device.id));
}
