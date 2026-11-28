// Which machine's screen this window shows: its own, or another computer's
// (a paired machine's own Hide in the rail's tile, `RemoteStage.tsx`). The
// core counts a look chosen on this machine only while a window of it shows
// its own screen (`docs/status-model.md`, Hide owns the read axis), so the
// daemon hears it whether or not the window holds the keyboard: a window
// behind other apps still shows what it showed.
//
// The action that changes the screen tells it after its own events, on the
// same socket (`actions.ts`, `noteWindowScreen`), so the core never reads a
// look the action passes through: a tile that brings a device forward sends
// `focus_device` first, and only then does this machine's screen count
// again. Each new connection hears it once, for the daemon holds nothing of
// a connection it has not met.

import { ownPaneShownThere } from "./computerWork";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** What a window shows: this machine's own screen, or another computer's. */
export type WindowScreen = "own" | "computer";

/**
 * What this window tells: the screen it shows and, on a paired machine's
 * screen that shows this machine's own work there (PRD device-connect-iroh
 * D-55), that pane by this core's id, which the core reads as looked at
 * while it shows.
 */
export type WindowScreenReport = { screen: WindowScreen; shown_there?: string };

/** What this window shows now. */
export function windowScreen(): WindowScreenReport {
  const ui = useUiStore.getState();
  if (ui.remoteScreen === null) return { screen: "own" };
  const pane = ownPaneShownThere(useShellStore.getState().rest, ui.remoteScreen, ui.remoteShowing[ui.remoteScreen] ?? null);
  return pane === null ? { screen: "computer" } : { screen: "computer", shown_there: pane };
}

/** Whether two reports say the same. */
export function sameWindowScreen(a: WindowScreenReport, b: WindowScreenReport): boolean {
  return a.screen === b.screen && a.shown_there === b.shown_there;
}

/**
 * Calls `tell` now and each time the page's connection is live again, a new
 * connection that has heard nothing, and `changed` when a snapshot arrives
 * while another computer's screen shows, since which of this machine's
 * panes that screen shows is read from the snapshot (the caller tells only
 * a report that differs).
 */
export function installWindowScreen(tell: () => void, changed: () => void): () => void {
  const unsubscribe = useShellStore.subscribe((state, previous) => {
    if (state.connection !== previous.connection && state.connection === "live") tell();
    else if (state.rest !== previous.rest && useUiStore.getState().remoteScreen !== null) changed();
  });
  tell();
  return unsubscribe;
}
