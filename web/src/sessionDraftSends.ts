// What happens to a session's draft beside Desk as the snapshot moves, whether or not its panel is drawn: a send's
// words wait for the pane's answer (`sending` in `sessionDrafts.ts`), and a gone pane's words go with it. A panel can
// close before the pane answers, so the answer is read here, once for the window, rather than in the panel.
//
// Each input has its own completion frame. Display snapshots can coalesce
// another window's refusal and a sent input, so they never settle a draft.

import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { uncertainSessionSends } from "./sessionDrafts";
import type { SessionThreadSection } from "./snapshot";

export function receiveThreadInputResult(input: NonNullable<SessionThreadSection["input"]>): void {
  useShellStore.setState({ threadInputResult: input });
}

/** A socket can close before its first snapshot made the connection live. */
export function loseThreadInputConnection(): void {
  useUiStore.setState((local) => ({ sessionDrafts: uncertainSessionSends(local.sessionDrafts) }));
}

useShellStore.subscribe((state, previous) => {
  const input = state.threadInputResult;
  if (input && input !== previous.threadInputResult && input.state !== "sending") {
    if (input.state === "refused" && input.reason === "uncertain") {
      useUiStore.setState((local) => ({ sessionDrafts: uncertainSessionSends(local.sessionDrafts, input.request_id) }));
    } else useUiStore.getState().settleSessionSend(input.request_id, input.state === "sent");
  }
  // A refusal hided makes before the core (a file it never staged) names the input's request too.
  const refusal = state.composerRefusal;
  if (refusal && refusal !== previous.composerRefusal) useUiStore.getState().settleSessionSend(refusal.request_id, false);
  if (state.connection !== "live" && previous.connection === "live") {
    loseThreadInputConnection();
  }
  // A pane's words go once the pane's own machine, connected, no longer lists it (`sessionDraftsLeft`).
  if (state.connection === "live" && (state.agents !== previous.agents || state.rest?.status?.remote !== previous.rest?.status?.remote || state.connection !== previous.connection)) {
    useUiStore.getState().keepSessionDraftsOf(state.agents, state.rest?.status?.remote);
  }
});
