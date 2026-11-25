// What Desk's agent opened for the operator with `hide file open` (the
// `desk` section's `opens`). The tab panel's tabs are this window's
// (`deskRoom.ts`), so the core only keeps the opens waiting; a window that
// shows Desk opens each one as a click would and tells the core it took it.
// A window on another screen takes none, so Desk never pulls the operator
// away from what they look at: the open waits until Desk is on screen.

import type { DeskOpen } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/**
 * Calls `open`, then `taken`, once for each of Desk's waiting opens while
 * this window shows Desk and its connection is live. An open already handed
 * on is not handed again while the core still lists it; a new connection
 * starts over, since a restarted core numbers its opens again.
 */
export function watchDeskOpens(open: (entry: DeskOpen) => void, taken: (id: number) => void): () => void {
  let handed = new Set<number>();
  const check = () => {
    const shell = useShellStore.getState();
    if (shell.connection !== "live") {
      handed = new Set();
      return;
    }
    const opens = shell.desk?.opens ?? [];
    // Only ids the core still lists are kept, so the set stays as small as the list.
    handed = new Set(opens.filter((entry) => handed.has(entry.id)).map((entry) => entry.id));
    if (useUiStore.getState().screen?.kind !== "desk") return;
    for (const entry of opens) {
      if (handed.has(entry.id)) continue;
      handed.add(entry.id);
      open(entry);
      taken(entry.id);
    }
  };
  const unsubscribeUi = useUiStore.subscribe((state, previous) => {
    if (state.screen?.kind !== previous.screen?.kind) check();
  });
  const unsubscribeShell = useShellStore.subscribe((state, previous) => {
    if (state.connection !== previous.connection || state.desk?.opens !== previous.desk?.opens) check();
  });
  check();
  return () => {
    unsubscribeUi();
    unsubscribeShell();
  };
}
