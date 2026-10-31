import { useEffect } from "react";
import type { Actions } from "./actions";
import { useUiStore } from "./ui";

// The project disk the lane measures while it shows it (PRD
// project-panel-results D-15, B20): the Project tool and the worktree table
// may each show a project, and the core keeps one watch per machine
// (`disk_watch`), so the lane's holders are counted here and the core hears
// every project one of them shows, and nothing once the last one goes, which
// stops a measurement still running. The Overview's Projects tab
// watches its own list, and the lane steps aside while the Overview is open.

const holders = new Map<string, number>();
let sent = "";

function settle(actions: Actions) {
  const wanted = [...holders.keys()].sort();
  const key = wanted.join("\n");
  if (key === sent) return;
  sent = key;
  // The Overview took the watch over: its list is the one the core keeps.
  if (wanted.length === 0 && useUiStore.getState().overviewOpen) return;
  actions.watchDisk(wanted);
}

/** Watches `projectId`'s disk while the caller is mounted with it; `null` watches nothing. */
export function useDiskWatch(actions: Actions, projectId: string | null) {
  useEffect(() => {
    if (!projectId) return;
    holders.set(projectId, (holders.get(projectId) ?? 0) + 1);
    settle(actions);
    return () => {
      const left = (holders.get(projectId) ?? 1) - 1;
      if (left > 0) holders.set(projectId, left);
      else holders.delete(projectId);
      settle(actions);
    };
  }, [actions, projectId]);
}
