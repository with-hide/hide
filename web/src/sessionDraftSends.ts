// What happens to a session's draft beside Desk as the snapshot moves, whether or not its panel is drawn: a send's
// words wait for the pane's answer (`sending` in `sessionDrafts.ts`), and a gone pane's words go with it. A panel can
// close before the pane answers, so the answer is read here, once for the window, rather than in the panel.
//
// The core answers every input through the thread section that stands when the answer lands, a closed one or another
// pane's included, and carries the last answer into the section that replaces it, so an answer that lands after the
// panel closed is still read here by its request id and no send waits for ever.

import { useShellStore } from "./store";
import { useUiStore } from "./ui";

useShellStore.subscribe((state, previous) => {
  const input = state.sessionThread?.input;
  if (input && input !== previous.sessionThread?.input && input.state !== "sending") {
    // Words the pane's waiting line took are as taken as typed ones: the draft is cleared the same way (PRD desk-queue B1).
    useUiStore.getState().settleSessionSend(input.request_id, input.state === "sent" || input.state === "queued");
  }
  // A refusal hided makes before the core (a file it never staged) names the input's request too.
  const refusal = state.composerRefusal;
  if (refusal && refusal !== previous.composerRefusal) useUiStore.getState().settleSessionSend(refusal.request_id, false);
  // A pane's words go once the pane's own machine, connected, no longer lists it (`sessionDraftsLeft`).
  if (state.connection === "live" && (state.agents !== previous.agents || state.rest?.status?.remote !== previous.rest?.status?.remote || state.connection !== previous.connection)) {
    useUiStore.getState().keepSessionDraftsOf(state.agents, state.rest?.status?.remote);
  }
});
