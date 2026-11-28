import { useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "./actions";
import { answerPlace } from "./answerPaths";
import { laneCards } from "./desk";
import { outputNamed } from "./deskRoom";
import { allAgents } from "./paneAgents";
import { askingSession } from "./sessionWork";
import { checkoutOfPane, localDeviceId } from "./snapshot";
import { useShellStore } from "./store";
import { ThreadPanel } from "./components/desk/thread-panel";

/** The same session panel Desk uses, over a pane's still-mounted terminal. */
export function PaneConversation({ paneId, actions }: { paneId: string; actions: Actions }) {
  const row = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote).find((row) => row.pane_id === paneId) ?? null);
  const place = useShellStore(useShallow((s) => answerPlace(s.rest, paneId)));
  const checkout = useShellStore((s) => checkoutOfPane(s.rest, paneId)?.checkout.path ?? null);
  // Its card as beside Desk: a job, a worker or a session Desk named; what its agent does is its row's (`ThreadPanel`).
  const job = useShellStore((s) => (s.deskJobs ? laneCards(s.deskJobs).find((one) => one.pane_id === paneId) ?? null : null));
  const location = useMemo(() => place && checkout ? { device: place.device, checkout, folder: place.cwd ?? checkout } : null, [place, checkout]);
  // Opening the conversation over the pane is its press: the box takes the keyboard once it stands, and once only.
  const [answer, setAnswer] = useState(true);
  return <ThreadPanel
    job={job} paneId={paneId} actions={actions} paneConversation focused answer={answer} onAnswered={() => setAnswer(false)}
    asks={row?.open_asks?.items}
    onAsk={(id, status) => { const session = row ? askingSession(row) : null; if (session) actions.setMissedStatus(session, id, status); }}
    onTerminal={() => actions.togglePaneConversation(paneId)}
    outputOf={(output, viewId) => outputNamed(job?.outputs ?? [], output, location, localDeviceId(useShellStore.getState().rest), viewId)}
    onOutput={(output) => {
      if (output.url) actions.openLink(output.url, false, paneId);
      else if (output.path && location) actions.openThreadFile({ kind: "project", path: output.path, device: output.device_id ?? location.device, root: output.checkout ?? location.checkout }, paneId);
    }}
    onMission={(id) => actions.openMission(id)}
    onSession={(session) => { if (session.pane_id) actions.openListedAgent(session.pane_id); }}
    onCopy={(text, what) => actions.copyText(text, what)}
    onFile={(file) => actions.openThreadFile(file, paneId)} place={place}
  />;
}
