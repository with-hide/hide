// What a screen draws with of the View menu's display choices
// (`displayPreferences.ts`): another computer's screen the showing window's,
// any other its core's.

import { useSyncExternalStore } from "react";
import { ownDisplayPreferences, type DisplayPreferences } from "./displayPreferences";
import type { PaneTurnRowMode } from "./generated/hided-ws";
import { readViewer, subscribeViewer } from "./remoteView";
import { useShellStore } from "./store";

/** The showing window's choices on another computer's screen, or null on any other screen and from a desktop that sends none. */
function viewerDisplay(): DisplayPreferences | null {
  return readViewer()?.display ?? null;
}

/** The one rule: the showing window's choices on another computer's screen, else the core's. */
export function drawnDisplayPreferences(uiState: Parameters<typeof ownDisplayPreferences>[0]): DisplayPreferences {
  return viewerDisplay() ?? ownDisplayPreferences(uiState);
}

/** Calls `onChange` when either side of the rule moves: the core's choices or the showing window's. */
export function subscribeDrawnDisplay(onChange: () => void): () => void {
  const stops = [useShellStore.subscribe(onChange), subscribeViewer(onChange)];
  return () => stops.forEach((stop) => stop());
}

/** One choice this screen draws with, read through the rule; `select` returns a plain value, so an unrelated change renders nothing. */
export function useDrawnDisplay<T extends string | boolean>(select: (display: DisplayPreferences) => T): T {
  const read = () => select(drawnDisplayPreferences(useShellStore.getState().rest?.ui_state));
  return useSyncExternalStore(subscribeDrawnDisplay, read, read);
}

/** Whether this screen draws the showing window's choices, so its own View menu would change nothing drawn here. */
export function useFollowsViewer(): boolean {
  return useSyncExternalStore(subscribeViewer, () => viewerDisplay() !== null, () => false);
}

/** How agent panes draw their turn row on this screen. */
export function usePaneTurnRowMode(): PaneTurnRowMode {
  return useDrawnDisplay((display) => display.pane_turn_row_mode);
}
