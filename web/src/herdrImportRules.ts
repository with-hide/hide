// What the screen shows for importing this machine's Herdr save into Hide's own engine (PRD
// engine-bundle-4-herdr-removal D-7, D-8). The core decides the offer and owns the import
// (`status.herdr_import`); these rules only say which part shows where.

import type { HerdrImport } from "./snapshot";

/** What the sidebar's line says, or null when it shows nothing (B5, B11, B15, B16). */
export type ImportLine =
  | { kind: "import"; tabs: number; running: boolean }
  | { kind: "waiting"; count: number }
  | { kind: "moving" }
  | { kind: "unreadable"; reason: string }
  | { kind: "refused"; refused: HerdrImport["refused"] };

export function importLine(found: HerdrImport | null | undefined): ImportLine | null {
  if (!found) return null;
  if (found.stage === "moving") return { kind: "moving" };
  if (found.failure === "unreadable") return { kind: "unreadable", reason: found.reason ?? "" };
  if (found.refused.length > 0) return { kind: "refused", refused: found.refused };
  if (found.workspaces === 0) return found.kind === "import" && found.waiting > 0 ? { kind: "waiting", count: found.waiting } : null;
  if (found.kind === "import") return { kind: "import", tabs: found.tabs, running: found.herdr_running };
  return null;
}

/** Whether the first run and Settings offer the read-only import (B14, B16). */
export function offersImport(found: HerdrImport | null | undefined): boolean {
  return !!found && found.kind === "import" && found.workspaces > 0 && found.stage !== "moving";
}
