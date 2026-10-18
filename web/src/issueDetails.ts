import { useEffect } from "react";
import type { Actions } from "./actions";
import type { IssueDetail, Workspace } from "./snapshot";
import { useShellStore } from "./store";

type DetailProject = Pick<Workspace, "id" | "device_id">;

/** A path-qualified memo key can still name different work on two devices. */
export function issueDetailOf(detail: IssueDetail | null | undefined, project: DetailProject | null, taskKey: string): IssueDetail | null {
  if (!project?.id || !project.device_id || !detail) return null;
  return detail.workspace_id === project.id && detail.device_id === project.device_id && detail.task_key === taskKey ? detail : null;
}

/** Reuse the active detail when opening Start over an already loaded item. */
export function useIssueDetail(actions: Actions, project: DetailProject | null, taskKey: string) {
  const workspaceId = project?.id;
  const deviceId = project?.device_id;
  const detail = useShellStore((state) => issueDetailOf(state.rest?.issue_work?.detail, project, taskKey));
  useEffect(() => {
    if (!workspaceId || !deviceId) return;
    const known = useShellStore.getState().rest?.issue_work?.detail;
    if (!issueDetailOf(known, { id: workspaceId, device_id: deviceId }, taskKey)) actions.requestIssueDetail(workspaceId, taskKey);
  }, [actions, workspaceId, deviceId, taskKey]);
  return detail;
}
