// The pure decisions behind Settings › External tools (PRD
// hide-external-control B2, B11, B13, B15): what a request's and a connected
// tool's second line say, when a row warns that its work would become the
// operator's, and what the row lists when it is opened. Every value is the
// core's `access` section; nothing here reads the store.

import type { TFunction } from "i18next";
import type { InterfaceLanguage } from "./i18n/locale";
import { lastSeen } from "./mobileSettings";
import type { AccessConnection, AccessPending, AgentRow, Mission } from "./snapshot";

// Times read as a paired phone's last seen does (`lastSeen`), on the window's
// shared clock: the `access` section carries no clock of the core's.
type Translate = TFunction<"translation">;

/** A request's line under its name (B2): the machine, the program that asked and when. */
export function pendingLine(request: AccessPending, machine: string, nowMs: number, t: Translate, language: InterfaceLanguage): { machine: string; program: string; requested: string } {
  return {
    machine,
    program: request.program ?? t("externalTools.unknownProgram"),
    requested: t("externalTools.requested", { when: lastSeen(request.requested_ms, nowMs, false, t, language) }),
  };
}

/**
 * A connected tool's line under its name (B11), as the pieces the row joins
 * with ` · ` and keeps whole when it wraps: the machine, its last use, and the
 * agents and missions it started.
 */
export function connectionLine(connection: AccessConnection, machine: string, nowMs: number, t: Translate, language: InterfaceLanguage): string[] {
  return [
    machine,
    t("externalTools.lastUsed", { when: lastSeen(connection.last_used_ms, nowMs, false, t, language) }),
    t("externalTools.agents", { agents: connection.agent_count }),
    t("externalTools.missions", { missions: connection.mission_count }),
  ];
}

/**
 * Whether the row says that disconnecting or View only hands the tool's work
 * to the operator (B13, D-14): only an Act tool's work is the tool's, so a
 * View only tool's agents are already the operator's.
 */
export function warnsOfWork(connection: AccessConnection): boolean {
  return connection.level === "act" && connection.agent_count + connection.mission_count > 0;
}

/** One line of an opened tool row (B15): an agent opens its pane, a mission its map. */
export type ToolWork =
  | { kind: "agent"; agentId: string; paneId: string; row: AgentRow | null }
  | { kind: "mission"; id: string; mission: Mission | null };

/**
 * What an opened row lists, agents first, then missions, each in the
 * section's order. An agent or a mission the window has not drawn yet keeps
 * its line under its id, so a count never names more than the list shows.
 */
export function toolWork(connection: AccessConnection, rowsByPane: ReadonlyMap<string, AgentRow>, missions: readonly Mission[]): ToolWork[] {
  return [
    ...connection.agents.map((agent): ToolWork => ({ kind: "agent", agentId: agent.agent_id, paneId: agent.pane_id, row: rowsByPane.get(agent.pane_id) ?? null })),
    ...connection.missions.map((id): ToolWork => ({ kind: "mission", id, mission: missions.find((mission) => mission.id === id) ?? null })),
  ];
}
