// Whether this window holds the operator's keyboard, and on which screen: the
// Workspace screen, or Desk's room with Desk's desk drawn. The core reads what
// is in front of such a window as seen (PRD unseen-agent-views D-07; 한눈에 for
// Desk's desk). A window showing another computer's screen draws neither of
// this machine's. It is sent only when it changes, and again on each new
// connection, for which the daemon holds nothing yet.

import { deskViewOf } from "./deskDesk";
import { deskShown } from "./deskRoom";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { subscribeKeyboardOwner, windowIsActive } from "./viewFocus";

/** The screen a window holding the keyboard draws, whose View in front is seen, or null. */
export type FocusScreen = "workspace" | "desk" | null;

export function installWindowFocus(send: (screen: FocusScreen) => void): () => void {
  let sent: FocusScreen | undefined;
  const report = () => {
    if (useShellStore.getState().connection !== "live") {
      sent = undefined;
      return;
    }
    const looking = windowIsActive() && document.visibilityState === "visible";
    const ui = useUiStore.getState();
    // Another computer's screen in this window draws nothing of this machine's (`windowScreen.ts`).
    const screen: FocusScreen = !looking || ui.remoteScreen !== null
      ? null
      : ui.screen?.kind === "workspace"
        ? "workspace"
        : ui.screen?.kind === "desk" && deskShown(ui.deskRoom, deskViewOf(useShellStore.getState().rest)?.layout.display_count ?? 0)
          ? "desk"
          : null;
    if (screen === sent) return;
    sent = screen;
    send(screen);
  };
  // The keyboard owner's listeners also hear the window gain and lose focus.
  const unsubscribeOwner = subscribeKeyboardOwner(report);
  // A snapshot can add Desk's first tab or take its last, which shows or hides the desk.
  const unsubscribeShell = useShellStore.subscribe((state, previous) => { if (state.connection !== previous.connection || state.rest !== previous.rest) report(); });
  const unsubscribeUi = useUiStore.subscribe((state, previous) => { if (state.screen?.kind !== previous.screen?.kind || state.deskRoom !== previous.deskRoom || state.remoteScreen !== previous.remoteScreen) report(); });
  document.addEventListener("visibilitychange", report);
  report();
  return () => {
    unsubscribeOwner();
    unsubscribeShell();
    unsubscribeUi();
    document.removeEventListener("visibilitychange", report);
  };
}
