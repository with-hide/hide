import { useEffect } from "react";
import type { Actions } from "./actions";
import type { BacklogOrigin, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";

// Where a backlog item came from carries its session's request and answer
// (PRD overview-device-tasks B31, B33), so the core keeps it only while a
// screen shows it. The item's panel and its Start dialog can both show one
// item's origin; each holds it, and the last in this window to leave tells the
// daemon, which asks the core to drop it only when no other window shows it.
const holders = new Map<string, number>();

type OriginProject = Pick<Workspace, "id" | "device_id">;

/** The selected project's origin; a same-key item on another device is separate. */
export function originOf(rest: SnapshotRest | null, project: OriginProject | null, taskKey: string): BacklogOrigin | null {
  if (!project) return null;
  return rest?.issue_work?.origins?.find((origin) => origin.workspace_id === project.id
    && origin.device_id === project.device_id && origin.task_key === taskKey) ?? null;
}

function originKey(project: OriginProject, taskKey: string): string {
  return JSON.stringify([project.device_id, project.id, taskKey]);
}

/** Takes one hold on an item's origin. */
export function holdOrigin(project: OriginProject, taskKey: string): void {
  const key = originKey(project, taskKey);
  holders.set(key, (holders.get(key) ?? 0) + 1);
}

/** Lets go of one hold; true when it was the last one on that item. */
export function releaseOrigin(project: OriginProject, taskKey: string): boolean {
  const key = originKey(project, taskKey);
  const held = holders.get(key);
  if (held === undefined) throw new Error(`no hold on the origin of ${key}`);
  if (held > 1) {
    holders.set(key, held - 1);
    return false;
  }
  holders.delete(key);
  return true;
}

/**
 * Reads an item's origin while the caller is shown, and closes it when no
 * screen shows it any more. A reconnect is a new connection, whose holds the
 * daemon starts empty and whose old ones it has let go, so the read is said
 * again each time the page is live.
 */
export function useBacklogOrigin(actions: Actions, project: OriginProject | null, taskKey: string): void {
  const live = useShellStore((state) => state.connection === "live");
  const workspaceId = project?.id;
  const deviceId = project?.device_id;
  useEffect(() => {
    if (workspaceId === undefined || deviceId === undefined) return;
    const owner = { id: workspaceId, device_id: deviceId };
    holdOrigin(owner, taskKey);
    return () => {
      if (releaseOrigin(owner, taskKey)) actions.closeBacklogOrigin(workspaceId, deviceId, taskKey);
    };
  }, [actions, workspaceId, deviceId, taskKey]);
  useEffect(() => {
    if (live && workspaceId !== undefined && deviceId !== undefined) actions.readBacklogOrigin(workspaceId, deviceId, taskKey);
  }, [actions, workspaceId, deviceId, taskKey, live]);
}
