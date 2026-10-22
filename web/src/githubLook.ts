// The operator looks at a project: its window comes forward, or the project
// does. The core then reads GitHub for that project again when its last good
// answer is old enough (`Runtime::read_when_looked_at`), so a merge, a check
// or a review shows when the operator looks, not up to five minutes later.
// The decision is the core's; this only says what is being looked at.

import { useEffect } from "react";
import type { Actions } from "./actions";
import { readsHere } from "./devices";
import { focusedRemoteDevice, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

/** The id of the local Git project in front, or null when a device, a plain folder or nothing is. */
export function gitProjectInFront(rest: SnapshotRest | null): string | null {
  const checkoutId = rest?.navigator?.focused_checkout_id;
  if (!checkoutId || focusedRemoteDevice(rest)) return null;
  const devices = rest?.navigator?.devices;
  const workspace = rest?.navigator?.workspaces?.find((candidate) => candidate.checkouts.some((checkout) => checkout.id === checkoutId));
  return workspace && workspace.is_git && !workspace.remote_target_id && readsHere(workspace, devices) ? workspace.id : null;
}

/**
 * Says the project in front is being looked at when it comes to the front and
 * each time the page gains focus or becomes visible. A burst of these adds no
 * read: the core answers only the first that finds the answer old.
 */
export function useGithubLook(actions: Actions): void {
  const live = useShellStore((s) => s.connection === "live");
  const project = useShellStore((s) => gitProjectInFront(s.rest));
  useEffect(() => {
    if (!live || !project) return undefined;
    const look = () => {
      if (document.visibilityState === "visible") actions.lookAtProject(project);
    };
    look();
    window.addEventListener("focus", look);
    document.addEventListener("visibilitychange", look);
    return () => {
      window.removeEventListener("focus", look);
      document.removeEventListener("visibilitychange", look);
    };
  }, [actions, live, project]);
}

/**
 * Says the window came forward, for the background fetch, each time the page
 * gains focus or becomes visible, whichever device's checkout is in front. A
 * burst adds no fetch: the core fetches only when the last one is old.
 */
export function useFetchLook(actions: Actions): void {
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (!live) return undefined;
    const look = () => {
      if (document.visibilityState === "visible") actions.lookForFetch();
    };
    window.addEventListener("focus", look);
    document.addEventListener("visibilitychange", look);
    return () => {
      window.removeEventListener("focus", look);
      document.removeEventListener("visibilitychange", look);
    };
  }, [actions, live]);
}
