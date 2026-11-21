// An item that is its own Needs You line, with no agent row to carry what
// waits on the operator: a mission's (PRD mission-node-simple D-19, B14) or
// an automation stopped for a person to fix (PRD automations D-12, B23). The
// device's scope names both and counts them in its Needs You number, so each
// list that draws the number draws these rows with its agents.

import type { MissionAsk } from "./missions";
import type { Automation, AutomationPauseReason, AutomationsSection } from "./snapshot";

/** An automation stopped for a person to fix (B23): why, since when, and the machine that runs it. */
export type AutomationAsk = { automation: Automation; reason: AutomationPauseReason; since_unix_ms: number; device_id: string };

export type OwnAsk = { kind: "mission"; ask: MissionAsk } | { kind: "automation"; ask: AutomationAsk };

export function missionOwnAsks(asks: readonly MissionAsk[]): OwnAsk[] {
  return asks.map((ask) => ({ kind: "mission", ask }));
}

export function automationOwnAsks(asks: readonly AutomationAsk[]): OwnAsk[] {
  return asks.map((ask) => ({ kind: "automation", ask }));
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
  return own.kind === "mission" ? `mission:${own.ask.mission.id}` : `automation:${own.ask.automation.id}`;
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
