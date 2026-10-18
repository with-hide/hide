import { useEffect } from "react";
import type { Actions } from "./actions";
import { useShellStore } from "./store";

/** Reuse the active detail when opening Start over an already loaded item. */
export function useIssueDetail(actions: Actions, workspaceId: string, taskKey: string) {
  const detail = useShellStore((state) => state.rest?.issue_work?.detail?.task_key === taskKey ? state.rest.issue_work.detail : null);
  useEffect(() => {
    const known = useShellStore.getState().rest?.issue_work?.detail;
    if (!known || known.task_key !== taskKey) actions.requestIssueDetail(workspaceId, taskKey);
  }, [actions, workspaceId, taskKey]);
  return detail;
}
