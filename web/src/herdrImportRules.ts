// What the screen shows for moving this machine's Herdr save into Hide's own engine (PRD
// engine-bundle-4-herdr-removal D-6 to D-8). The core decides the offer and owns the move
// (`status.herdr_import`); these rules only say which part shows where.

import type { HerdrImport } from "./snapshot";

/**
 * Whether the question stands: Hide's own Herdr runs with something to move, and the operator has not put it off
 * on this page. "Later" is this page's: the next launch of the app asks again (B5).
 */
export function asksToMove(found: HerdrImport | null | undefined, later: boolean): boolean {
  return !!found && found.kind === "move" && found.workspaces > 0 && !later;
}

/** What the sidebar's line says, or null when it shows nothing (B4, B5, B11, B16). */
export type ImportLine =
  | { kind: "move"; agents: number; tabs: number }
  | { kind: "import"; tabs: number; running: boolean }
  | { kind: "waiting"; count: number }
  | { kind: "moving" }
  | { kind: "unreadable"; reason: string }
  | { kind: "refused"; refused: HerdrImport["refused"] };

export function importLine(found: HerdrImport | null | undefined, asking: boolean): ImportLine | null {
  if (!found || asking) return null;
  if (found.stage === "moving") return { kind: "moving" };
  if (found.failure === "unreadable") return { kind: "unreadable", reason: found.reason ?? "" };
  if (found.refused.length > 0) return { kind: "refused", refused: found.refused };
  if (found.workspaces === 0) return found.kind === "import" && found.waiting > 0 ? { kind: "waiting", count: found.waiting } : null;
  if (found.kind === "move") return { kind: "move", agents: found.agents, tabs: found.tabs };
  if (found.kind === "import") return { kind: "import", tabs: found.tabs, running: found.herdr_running };
  return null;
}

/** Whether the first run and Settings offer the read-only import (B14, B16). */
export function offersImport(found: HerdrImport | null | undefined): boolean {
  return !!found && found.kind === "import" && found.workspaces > 0 && found.stage !== "moving";
}
