import { useEffect, useRef } from "react";
import type { Actions } from "./actions";
import { allAgents } from "./paneAgents";
import { useShellStore } from "./store";
import { useUiStore, type ThreadLease } from "./ui";

/** One window's last bounded thread pages. A replaced hold is reclaimed only by a press. */
export function useSessionThread(actions: Pick<Actions, "openThread" | "closeThread">, paneId: string | null, paneConversation = false) {
  const live = useShellStore((state) => state.connection === "live");
  const lease = useUiStore((state) => state.threadLease);
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    if (!paneId) return;
    const previous = useUiStore.getState().threadLease;
    useUiStore.setState({ threadLease: previous?.paneId === paneId ? { ...previous, acquired: false } : { paneId, acquired: false, taken: false, thread: null, history: null } });
    // The first published result may already be a later window's open: the
    // core coalesces notifications, so seeing our own pane is not required.
    const beforeOpen = useShellStore.getState().sessionThread;
    const observe = () => {
      const held = useUiStore.getState().threadLease;
      if (held?.paneId !== paneId || held.taken) return;
      const { sessionThread: thread, sessionThreadHistory: history } = useShellStore.getState();
      const changedHold = thread && thread.generation > 0 && thread.generation !== (held.thread?.generation ?? beforeOpen?.generation ?? 0);
      if (thread?.pane_id === paneId && thread.state !== "closed") {
        if (!held.acquired || held.thread !== thread || held.history !== history) {
          useUiStore.setState({ threadLease: { ...held, acquired: true, thread, history } });
        }
      } else if (thread && (changedHold || (held.acquired && thread.pane_id && thread.state !== "closed"))) {
        useUiStore.setState({ threadLease: { ...held, taken: true } });
      }
    };
    const stop = useShellStore.subscribe(observe);
    observe();
    return () => {
      stop();
      latest.current.closeThread();
    };
  }, [paneId]);
  useEffect(() => {
    if (!paneId || !live || useUiStore.getState().threadLease?.taken) return;
    if (paneConversation) latest.current.openThread(paneId, true);
    else latest.current.openThread(paneId);
  }, [paneId, live, paneConversation]);
  const reclaim = () => {
    const held = useUiStore.getState().threadLease;
    if (!paneId || !live || held?.paneId !== paneId) return;
    useUiStore.setState({ threadLease: { ...held, taken: false, acquired: false } });
    if (paneConversation) latest.current.openThread(paneId, true);
    else latest.current.openThread(paneId);
  };
  const shown: ThreadLease | null = lease?.paneId === paneId ? lease : null;
  return { thread: shown?.thread ?? null, history: shown?.history ?? null, taken: shown?.taken ?? false, reclaim };
}

// Session identity, rather than pane id alone, bounds the window's choice.
useShellStore.subscribe((state, previous) => {
  const selected = useUiStore.getState().paneConversation;
  if (!selected || (state.agents === previous.agents && state.rest === previous.rest)) return;
  const row = allAgents(state.agents, state.rest?.status?.remote).find((row) => row.pane_id === selected.paneId);
  if (!row || row.thread_identity !== selected.session) useUiStore.setState({ paneConversation: null });
});
