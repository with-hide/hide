// What the screen offers for the operator's own Herdr server on this machine
// (PRD herdr-takeover). The core decides the offer and owns the move
// (`status.herdr_takeover`); these rules only say which parts show where.

import type { HerdrTakeover, HerdrVersions, SnapshotRest } from "./snapshot";

/** Whether the wizard and Help > Set up Hide have a Herdr step: a move or restart to make, a look to try again, or a move whose step waits to be read (B1, B5, B8, B17). */
export function herdrStepShown(takeover: HerdrTakeover | null | undefined): boolean {
  if (!takeover) return false;
  return takeover.stage === "moved" || takeover.kind === "move" || takeover.kind === "restart" || takeover.kind === "look";
}

/** Not now: on a server Hide reads as it is from the start, and on any server once its move, restart or look failed (D-16, D-29, D-31). */
export function leavesForLater(takeover: HerdrTakeover): boolean {
  return takeover.stage !== "moving" && (takeover.same_protocol || takeover.stage === "failed");
}

/** "Stop Herdr yourself": only for a server Hide cannot read whose move or restart failed here (D-29); a failed look says it in its own words. */
export function stopYourselfShown(takeover: HerdrTakeover): boolean {
  return takeover.stage === "failed" && !takeover.same_protocol && takeover.kind !== "look";
}

/**
 * What holds all of Hide while this machine's Herdr speaks another protocol: nothing Hide opens works on such a server,
 * so no window, menu item or key reaches a workspace until it is moved, restarted or Hide is updated. `move`,
 * `restart` and `update_hide` are the core's offer; `look` a server it could not read; `newer` and `older` a mismatch
 * the core offers nothing for (a daemon without a Herdr of its own, or a look still on its way). A server on Hide's
 * protocol never holds it, whatever its release (D-16), and a moved server's step is the wizard's or its line's.
 */
export type HerdrGate = "move" | "restart" | "update_hide" | "look" | "newer" | "older";

export function herdrGate(rest: SnapshotRest | null | undefined): HerdrGate | null {
  const takeover = rest?.status?.herdr_takeover;
  const herdr = rest?.status?.herdr;
  // A server just moved onto Hide's Herdr is Hide's, though the connection reads the old one for a moment longer.
  if (takeover?.stage === "moved") return null;
  if (takeover && !takeover.same_protocol && takeover.kind !== "look") return takeover.kind;
  if (herdr?.state !== "protocol_mismatch") return null;
  if (takeover?.kind === "look") return "look";
  return (herdr.received_protocol ?? 0) > (herdr.expected_protocol ?? Infinity) ? "newer" : "older";
}

/** The operator's own `herdr` that stays behind a moved server: its version, or null (B5). */
export function movedLine(takeover: HerdrTakeover | null | undefined): string | null {
  return takeover?.stage === "moved" ? takeover.path_herdr_behind : null;
}

/**
 * The command that brings the operator's own `herdr` up, as the core composed it for the file their terminal finds:
 * Homebrew's upgrade where Homebrew installed it, else a copy of Hide's Herdr over it. Hide shows it to copy and never
 * runs it, since it changes a file that is the operator's. Null when the core read no `herdr` there.
 */
export function pathHerdrUpdate(versions: HerdrVersions | null | undefined): { command: string; brew: boolean; hide: string } | null {
  const command = versions?.path_herdr?.update_command;
  if (!versions || !command) return null;
  return { command, brew: command.startsWith("brew "), hide: versions.hide };
}

/** The core's diagnostic log beside its state file (`<state>/Logs/core.jsonl`), where a move's or look's failure is written. */
export function coreLogPath(statePath: string): string {
  const cut = Math.max(statePath.lastIndexOf("/"), statePath.lastIndexOf("\\"));
  const separator = statePath[cut] ?? "/";
  return `${statePath.slice(0, cut)}${separator}Logs${separator}core.jsonl`;
}
