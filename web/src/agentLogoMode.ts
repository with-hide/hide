// Whether a list row or tab draws its agent's logo (PRD agent-logo-view-mode):
// when the drawn `agent_logo_mode` is shown (the core's, or on another
// computer's screen the showing window's: `drawnDisplay.ts`), or while an ⌥ hold has revealed
// the Agents family's numbers (D-05). The hold is the keyboard's own state
// (`hints.ts`), read here and never listened for a second time.

import { useSyncExternalStore } from "react";
import { drawnDisplayPreferences, subscribeDrawnDisplay } from "./drawnDisplay";
import type { AgentLogoMode } from "./generated/hided-ws";
import { useShellStore } from "./store";
import type { NumberedFamily } from "./shortcuts";
import { useUiStore } from "./ui";

/** The one rule: the mode is shown, or the Agents hold is revealed. Absent reads as hidden (D-03). */
export function agentLogosShown(mode: AgentLogoMode | undefined, hint: NumberedFamily | null): boolean {
  return mode === "shown" || hint === "agents";
}

function subscribe(onChange: () => void): () => void {
  const stops = [subscribeDrawnDisplay(onChange), useUiStore.subscribe(onChange)];
  return () => stops.forEach((stop) => stop());
}

function current(): boolean {
  return agentLogosShown(drawnDisplayPreferences(useShellStore.getState().rest?.ui_state).agent_logo_mode, useUiStore.getState().hint);
}

/**
 * One boolean per row, so a snapshot or a hold that leaves it as it was
 * renders nothing: a hold in the shown mode adds only the keycap.
 */
export function useAgentLogosShown(): boolean {
  return useSyncExternalStore(subscribe, current, current);
}
