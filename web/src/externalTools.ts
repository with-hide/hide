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
 * A name a bot may take, as the core reads it (`hide_core::access::valid_name`):
 * letters, digits, `.`, `_` and `-`, starting with a letter or a digit, 64 at
 * most. The core also refuses a name that is no file name on Windows (`con`),
 * which the dialog words from its answer.
 */
export const BOT_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

/** The words for a make the core refused, by its code; one it did not name reads as a failure to try again. */
export function createRefusal(code: string): `externalTools.create.error.${"access_name_taken" | "access_limit" | "access_invite_limit" | "invalid_request" | "other"}` {
  switch (code) {
    case "access_name_taken":
    case "access_limit":
    case "access_invite_limit":
    case "invalid_request":
      return `externalTools.create.error.${code}`;
    default:
      return "externalTools.create.error.other";
  }
}

/**
 * Whether the row says that disconnecting hands the bot's work to the
 * operator (B13, D-14): only a bot in own mode keeps what it started; an
 * all-mode bot's work is the operator's already.
 */
export function warnsOfWork(connection: AccessConnection): boolean {
  return connection.mode.kind === "own" && connection.agent_count + connection.mission_count > 0;
}
