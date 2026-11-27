// What happens to a session's draft beside Desk as the snapshot moves, whether or not its panel is drawn: a send's
// words wait for the pane's answer (`sending` in `sessionDrafts.ts`), and a gone pane's words go with it. A panel can
// close before the pane answers, so the answer is read here, once for the window, rather than in the panel.
//
// The core never drops a thread's input answer: closing a thread leaves a closed thread section, so an answer that
// lands after the panel closed is still read from it, or from the next thread opened, by its request id.

import { useShellStore } from "./store";
import { useUiStore } from "./ui";

useShellStore.subscribe((state, previous) => {
  const input = state.sessionThread?.input;
  if (input && input !== previous.sessionThread?.input && input.state !== "sending") {
    useUiStore.getState().settleSessionSend(input.request_id, input.state === "sent");
  }
  // A refusal hided makes before the core (a file it never staged) names the input's request too.
  const refusal = state.composerRefusal;
  if (refusal && refusal !== previous.composerRefusal) useUiStore.getState().settleSessionSend(refusal.request_id, false);
  // A pane's words go once the pane's own machine, connected, no longer lists it (`sessionDraftsLeft`).
  if (state.connection === "live" && (state.agents !== previous.agents || state.rest?.status?.remote !== previous.rest?.status?.remote || state.connection !== previous.connection)) {
    useUiStore.getState().keepSessionDraftsOf(state.agents, state.rest?.status?.remote);
  }
});
