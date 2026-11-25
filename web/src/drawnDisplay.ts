// What a screen draws with of the View menu's display choices
// (`displayPreferences.ts`): another computer's screen the showing window's,
// any other its core's.

import { useSyncExternalStore } from "react";
import { ownDisplayPreferences, type DisplayPreferences } from "./displayPreferences";
import type { PaneTurnRowMode } from "./generated/hided-ws";
import { readViewer, subscribeViewer } from "./remoteView";
import { useShellStore } from "./store";

/** The one rule: the showing window's choices on another computer's screen, else the core's. */
export function drawnDisplayPreferences(uiState: Parameters<typeof ownDisplayPreferences>[0]): DisplayPreferences {
  return readViewer()?.display ?? ownDisplayPreferences(uiState);
}

/** Whether this screen draws the showing window's choices, so its own View menu would change nothing drawn here. */
export function useFollowsViewer(): boolean {
  return useSyncExternalStore(subscribeViewer, () => (readViewer()?.display ?? null) !== null, () => false);
}

/** How agent panes draw their turn row on this screen. */
export function usePaneTurnRowMode(): PaneTurnRowMode {
  const own = useShellStore((state) => state.rest?.ui_state?.pane_turn_row_mode) ?? "one_line";
  const viewer = useSyncExternalStore(subscribeViewer, () => readViewer()?.display?.pane_turn_row_mode ?? null, () => null);
  return viewer ?? own;
}
