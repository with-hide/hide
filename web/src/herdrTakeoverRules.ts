// What the screen offers for the operator's own older Herdr server on this
// machine (PRD herdr-takeover). The core decides the offer and owns the move
// (`status.herdr_takeover`); these rules only say which parts show where.

import type { HerdrTakeover } from "./snapshot";

/** Whether the wizard and Help > Set up Hide have a Herdr step: a move or restart to make, or a move whose step waits to be read (B1, B5, B17). */
export function herdrStepShown(takeover: HerdrTakeover | null | undefined): boolean {
  if (!takeover) return false;
  return takeover.stage === "moved" || takeover.kind === "move" || takeover.kind === "restart";
}

/** Not now: on a server Hide reads as it is from the start, and on any server once its move failed (D-16, D-29, D-31). */
export function leavesForLater(takeover: HerdrTakeover): boolean {
  return takeover.stage !== "moving" && (takeover.same_protocol || takeover.stage === "failed");
}

/** "Stop Herdr yourself": only for a server Hide cannot read whose move failed here (D-29). */
export function stopYourselfShown(takeover: HerdrTakeover): boolean {
  return takeover.kind === "move" && takeover.stage === "failed" && !takeover.same_protocol;
}

/**
 * What Overview shows in the Herdr place (B16, B18): a server Hide cannot
 * read gets its move or restart, a newer protocol the update; a server Hide
 * reads as it is adds nothing there (B16), and a moved server's line is
 * `movedLine`'s.
 */
export function overviewTakeover(takeover: HerdrTakeover | null | undefined): "move" | "restart" | "update_hide" | null {
  if (!takeover || takeover.stage === "moved") return null;
  if (takeover.kind === "update_hide") return "update_hide";
  return takeover.same_protocol ? null : takeover.kind;
}

/** The operator's own `herdr` that stays behind a moved server: what to update, or null (B5). */
export function movedLine(takeover: HerdrTakeover | null | undefined): string | null {
  return takeover?.stage === "moved" ? takeover.path_herdr_behind : null;
}

/** The one command that brings the operator's own `herdr` up to the server (D-12). */
export const HERDR_UPDATE_COMMAND = "herdr update";
