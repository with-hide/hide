// The pure decisions behind Settings › External tools (PRD
// hide-external-control B2, B11, B13): what a request's and a connected
// tool's second line say, and when a row warns that its work would become the
// operator's. Every value is the core's `access` section; nothing here reads
// the store.

import type { TFunction } from "i18next";
import type { InterfaceLanguage } from "./i18n/locale";
import { lastSeen } from "./mobileSettings";
import type { AccessConnection, AccessPending } from "./snapshot";

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
