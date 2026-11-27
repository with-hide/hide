// Whether this window holds the operator's keyboard on the Workspace screen,
// and whether it holds it with Desk's desk drawn: the core reads what is in
// front of such a window as seen (PRD unseen-agent-views D-07; 한눈에 for
// Desk's desk). Each is sent only when it changes, and again on each new
// connection, for which the daemon holds nothing yet.

import { deskViewOf } from "./deskDesk";
import { deskShown } from "./deskRoom";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { subscribeKeyboardOwner, windowIsActive } from "./viewFocus";

export type FocusReports = { workspace: (focused: boolean) => void; desk: (focused: boolean) => void };

export function installWindowFocus(send: FocusReports): () => void {
  let sent: { workspace: boolean; desk: boolean } | null = null;
  const report = () => {
    if (useShellStore.getState().connection !== "live") {
      sent = null;
      return;
    }
    const looking = windowIsActive() && document.visibilityState === "visible";
    const ui = useUiStore.getState();
    const workspace = looking && ui.screen?.kind === "workspace";
    const desk = looking && ui.screen?.kind === "desk" && deskShown(ui.deskRoom, deskViewOf(useShellStore.getState().rest)?.layout.display_count ?? 0);
    if (sent?.workspace !== workspace) send.workspace(workspace);
    if (sent?.desk !== desk) send.desk(desk);
    sent = { workspace, desk };
  };
  // The keyboard owner's listeners also hear the window gain and lose focus.
  const unsubscribeOwner = subscribeKeyboardOwner(report);
  // A snapshot can add Desk's first tab or take its last, which shows or hides the desk.
  const unsubscribeShell = useShellStore.subscribe((state, previous) => { if (state.connection !== previous.connection || state.rest !== previous.rest) report(); });
  const unsubscribeUi = useUiStore.subscribe((state, previous) => { if (state.screen?.kind !== previous.screen?.kind || state.deskRoom !== previous.deskRoom) report(); });
  document.addEventListener("visibilitychange", report);
  report();
  return () => {
    unsubscribeOwner();
    unsubscribeShell();
    unsubscribeUi();
    document.removeEventListener("visibilitychange", report);
  };
}
