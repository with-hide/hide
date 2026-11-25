import { useEffect, useRef } from "react";
import type { Actions } from "./actions";
import { useShellStore } from "./store";

/**
 * Holds `paneId`'s session thread open while the caller is shown, and lets it
 * go when it is not. A reconnect is a new connection, which holds nothing,
 * and the daemon let the old one's hold go (closing the thread when it was the
 * last), so the open is said again each time the page is live.
 */
export function useSessionThread(actions: Pick<Actions, "openThread" | "closeThread">, paneId: string | null): void {
  const live = useShellStore((state) => state.connection === "live");
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    if (!paneId) return;
    return () => latest.current.closeThread();
  }, [paneId]);
  useEffect(() => {
    if (paneId && live) latest.current.openThread(paneId);
  }, [paneId, live]);
}
