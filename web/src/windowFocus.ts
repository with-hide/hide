// Whether this window holds the operator's keyboard on the Workspace screen:
// the core reads what is in front of such a window as seen (PRD
// unseen-agent-views D-07). It is sent only when it changes, and again on each
// new connection, for which the daemon holds nothing yet.

import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { subscribeKeyboardOwner, windowIsActive } from "./viewFocus";

export function installWindowFocus(send: (focused: boolean) => void): () => void {
  let sent: boolean | null = null;
  const report = () => {
    if (useShellStore.getState().connection !== "live") {
      sent = null;
      return;
    }
    const focused = windowIsActive() && document.visibilityState === "visible" && useUiStore.getState().screen?.kind === "workspace";
    if (focused === sent) return;
    sent = focused;
    send(focused);
  };
  // The keyboard owner's listeners also hear the window gain and lose focus.
  const unsubscribeOwner = subscribeKeyboardOwner(report);
  const unsubscribeShell = useShellStore.subscribe((state, previous) => { if (state.connection !== previous.connection) report(); });
  const unsubscribeUi = useUiStore.subscribe((state, previous) => { if (state.screen?.kind !== previous.screen?.kind) report(); });
  document.addEventListener("visibilitychange", report);
  report();
  return () => {
    unsubscribeOwner();
    unsubscribeShell();
    unsubscribeUi();
    document.removeEventListener("visibilitychange", report);
  };
}
