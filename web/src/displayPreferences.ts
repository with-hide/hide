// The View menu's display choices as a screen draws them (UI_BEHAVIOR.md,
// Agent logo view mode, Pane turn row mode): a window's own screen draws its
// core's choices, and another computer's screen draws the choices of the
// window that shows it, which the desktop sends with that window's language
// and name (`Viewer` in `remoteView.ts`; UI_BEHAVIOR.md, Device rail, Another
// computer's screen). Nothing is written to that computer's core: its own
// window keeps its own choices. What a screen draws is `drawnDisplay.ts`;
// this module keeps no state and imports nothing at run time, since the
// desktop host checks the shell's choices with it.

import type { AgentLogoMode, PaneTurnRowMode } from "./generated/hided-ws";

/** The View menu's two display choices. */
export type DisplayPreferences = { agent_logo_mode: AgentLogoMode; pane_turn_row_mode: PaneTurnRowMode };

/** The two fields of a core's `ui_state` (`snapshot.ts`) that hold them. */
type UiState = { agent_logo_mode?: AgentLogoMode; pane_turn_row_mode?: PaneTurnRowMode };

// Every mode the wire names, so a mode added to the contract fails to compile until it is listed.
const LOGO_MODES: Record<AgentLogoMode, true> = { shown: true, hidden: true };
const TURN_ROW_MODES: Record<PaneTurnRowMode, true> = { one_line: true, two_lines: true, off: true };

/** A core's own choices: logos hidden and one line when never chosen. */
export function ownDisplayPreferences(uiState: UiState | null | undefined): DisplayPreferences {
  return { agent_logo_mode: uiState?.agent_logo_mode ?? "hidden", pane_turn_row_mode: uiState?.pane_turn_row_mode ?? "one_line" };
}

/**
 * The choices another side sent (the shell to its desktop host, the host to
 * a computer's screen), or null when it sent none or one this build does not
 * know, which leaves the screen on its own core's choices.
 */
export function readDisplayPreferences(value: unknown): DisplayPreferences | null {
  if (typeof value !== "object" || value === null) return null;
  const { agent_logo_mode, pane_turn_row_mode } = value as Record<string, unknown>;
  if (typeof agent_logo_mode !== "string" || !Object.hasOwn(LOGO_MODES, agent_logo_mode)) return null;
  if (typeof pane_turn_row_mode !== "string" || !Object.hasOwn(TURN_ROW_MODES, pane_turn_row_mode)) return null;
  return { agent_logo_mode: agent_logo_mode as AgentLogoMode, pane_turn_row_mode: pane_turn_row_mode as PaneTurnRowMode };
}

export function sameDisplayPreferences(a: DisplayPreferences | null, b: DisplayPreferences | null): boolean {
  return a?.agent_logo_mode === b?.agent_logo_mode && a?.pane_turn_row_mode === b?.pane_turn_row_mode;
}
