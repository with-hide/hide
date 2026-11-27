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

import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** What a window shows: this machine's own screen, or another computer's. */
export type WindowScreen = "own" | "computer";

/** The screen this window shows now. */
export function windowScreen(): WindowScreen {
  return useUiStore.getState().remoteScreen === null ? "own" : "computer";
}

/** Calls `tell` now and each time the page's connection is live again, a new connection that has heard nothing. */
export function installWindowScreen(tell: () => void): () => void {
  const unsubscribe = useShellStore.subscribe((state, previous) => {
    if (state.connection !== previous.connection && state.connection === "live") tell();
  });
  tell();
  return unsubscribe;
}
