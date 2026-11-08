// Where the start panel starts an agent (PRD home-device-rail D-19, D-22):
// the default is what is in front. The panel asks two things, which project
// and which branch: the project menu lists the front device's Home and
// projects, then each other device's, and a device that is not connected is
// listed but cannot be chosen; the branch menu lists that project's
// checkouts and a new branch of its own. A target is never remembered: the
// default is read from the page when the panel opens, so nothing here is state.

import type { TFunction } from "i18next";
import { thisMachineKey } from "./devices";
import { machineName } from "./screenMachine";
import { remoteContext } from "./remote";
import type { Checkout, Device, SnapshotRest, Workspace } from "./snapshot";
import { frontCheckout, localDeviceId } from "./snapshot";
import type { Screen } from "./ui";
import { projectSettings, settingsTarget } from "./projectSettings";

type TargetBase = {
  /** Unique across devices; the menu's value. */
  key: string;
  deviceId: string;
  label: string;
  /** Why the target cannot be chosen now, or null. */
  disabled: "not_connected" | null;
};

export type StartTarget =
  | (TargetBase & { kind: "home" })
  | (TargetBase & { kind: "checkout"; checkoutPath: string; workspaceId: string; checkoutId: string; branch: string; primary: boolean })
  | (TargetBase & { kind: "worktree"; repositoryRoot: string; workspaceId: string });

/**
 * One row of the project menu: a device's Home, or a project with the
 * checkouts its branch menu lists (its main one first) and, for a Git
 * project, the new branch a start can make.
 */
export type StartProject = {
  key: string;
  deviceId: string;
  label: string;
  disabled: "not_connected" | null;
  home: boolean;
  branches: StartTarget[];
  worktree: StartTarget | null;
};

export type TargetGroup = { deviceId: string; deviceLabel: string; connected: boolean; projects: StartProject[]; items: StartTarget[] };

export type StartTargets = {
  /** The front device's group first, then each other device's, in the order the catalog lists devices. */
  groups: TargetGroup[];
  /** The key the panel opens on; null only while nothing can be chosen. */
  defaultKey: string | null;
  /** The checkout in front, which the branch menu marks; null when no checkout is. */
  currentKey: string | null;
};

export const homeKey = (deviceId: string) => `home:${deviceId}`;
export const checkoutKey = (deviceId: string, path: string) => `checkout:${deviceId}:${path}`;
export const worktreeKey = (deviceId: string, root: string) => `worktree:${deviceId}:${root}`;

/** The core's own node before the snapshot names it: the empty id, which an event carries as no `device_id`. */
const localDevice: Pick<Device, "id" | "kind"> = { id: "", kind: "local" };

/** A device's workspaces as the catalog lists them, and whether the device answers commands now. */
function deviceWorkspaces(rest: SnapshotRest, device: Pick<Device, "id" | "kind">): { workspaces: Workspace[]; connected: boolean } {
  if (device.kind !== "remote") return { workspaces: rest.navigator?.workspaces ?? [], connected: true };
  const status = rest.status?.remote?.find((row) => row.target_id === device.id) ?? null;
  const session = status?.session ?? null;
  return { workspaces: session?.workspaces ?? [], connected: status?.state === "connected" && session !== null };
}

export const projectKey = (deviceId: string, workspaceId: string) => `project:${deviceId}:${workspaceId}`;

function groupOf(rest: SnapshotRest, device: Pick<Device, "id" | "kind">, name: string, front: boolean, current: string | null, t: TFunction<"translation">): TargetGroup {
  const { workspaces, connected } = deviceWorkspaces(rest, device);
  const disabled = connected ? null : "not_connected";
  // Only the front device's items go unprefixed: the trigger already sits where that device is.
  const prefix = front ? "" : `${name} · `;
  const home: StartTarget = { kind: "home", key: homeKey(device.id), deviceId: device.id, label: `${prefix}${t("common.home")}`, disabled };
  const projects: StartProject[] = [{ key: home.key, deviceId: device.id, label: t("common.home"), disabled, home: true, branches: [home], worktree: null }];
  for (const workspace of workspaces) {
    if (workspace.is_home || workspace.temporary) continue;
    const branches: StartTarget[] = [];
    for (const checkout of workspace.checkouts) {
      if (!checkout.exists) continue;
      branches.push({
        kind: "checkout",
        key: checkoutKey(device.id, checkout.path),
        deviceId: device.id,
        label: `${prefix}${workspace.label} · ${checkout.label}`,
        disabled,
        checkoutPath: checkout.path,
        workspaceId: workspace.id,
        checkoutId: checkout.id,
        branch: checkout.branch ?? checkout.label,
        primary: checkout.id === mainCheckout(workspace)?.id,
      });
    }
    if (branches.length === 0) continue;
    // The main checkout, then the one in front, then the rest as the catalog lists them.
    const rank = (target: StartTarget) => (target.kind === "checkout" && target.primary ? 0 : target.key === current ? 1 : 2);
    branches.sort((a, b) => rank(a) - rank(b));
    const worktree: StartTarget | null = workspace.is_git
      ? { kind: "worktree", key: worktreeKey(device.id, workspace.path), deviceId: device.id, label: `${prefix}${workspace.label} · ${t("projectSettings.worktree")}`, disabled, repositoryRoot: workspace.path, workspaceId: workspace.id }
      : null;
    projects.push({ key: projectKey(device.id, workspace.id), deviceId: device.id, label: workspace.label, disabled, home: false, branches, worktree });
  }
  const items = projects.flatMap((project) => (project.worktree ? [...project.branches, project.worktree] : project.branches));
  return { deviceId: device.id, deviceLabel: name, connected, projects, items };
}

/** The checkout a project's start lands on: its main one, else the first that exists. */
function mainCheckout(workspace: Workspace): Checkout | null {
  return workspace.checkouts.find((row) => row.exists && row.is_primary) ?? workspace.checkouts.find((row) => row.exists) ?? null;
}

/** The key of the thing in front on the front device, before it is checked against the menu. */
function frontKey(rest: SnapshotRest, screen: Screen | null, noProject: boolean, deviceId: string, workspaces: Workspace[], projectId: string | null): string {
  const home = homeKey(deviceId);
  // Settings sits in front of a center that stays where it was, so what the center shows is not what is in front.
  if (noProject) return home;
  if (projectId) {
    const project = workspaces.find((row) => row.id === projectId);
    const checkout = project && !project.is_home ? mainCheckout(project) : null;
    return checkout ? checkoutKey(deviceId, checkout.path) : home;
  }
  if (screen?.kind === "workspace") {
    const checkout = frontCheckout(rest);
    if (!checkout) return home;
    const workspace = workspaces.find((row) => row.checkouts.some((candidate) => candidate.id === checkout.id));
    return workspace && !workspace.is_home ? checkoutKey(deviceId, checkout.path) : home;
  }
  // All projects, a device's Home and Settings are all "no project in front".
  return home;
}

/**
 * `settings` says the panel was opened over Settings, so the front device's
 * Home is the default. A device's own Home screen names its device, which is
 * then the front device.
 */
export function startTargets(rest: SnapshotRest | null, screen: Screen | null, t: TFunction<"translation">, settings = false, projectId: string | null = null): StartTargets {
  if (!rest) return { groups: [], defaultKey: null, currentKey: null };
  const listed = rest.navigator?.devices?.length ? rest.navigator.devices : null;
  const devices = listed ?? [localDevice];
  const named = screen?.kind === "main" ? screen.deviceId : undefined;
  const frontId = named && devices.some((device) => device.id === named) ? named : (remoteContext(rest)?.device.id ?? localDeviceId(rest));
  const ordered = [...devices.filter((device) => device.id === frontId), ...devices.filter((device) => device.id !== frontId)];
  // Each device as this window names it; before the snapshot lists any, the one machine is this window's.
  const name = (id: string) => (listed ? machineName(listed, id, t) : t(thisMachineKey(null)));
  const frontDevice = ordered[0];
  const frontWorkspaces = frontDevice ? deviceWorkspaces(rest, frontDevice).workspaces : [];
  const inFront = frontDevice ? frontKey(rest, screen, settings, frontDevice.id, frontWorkspaces, projectId) : null;
  // Only the checkout a Workspace shows is "the one in front" the branch menu marks.
  const currentKey = screen?.kind === "workspace" && !settings && inFront?.startsWith("checkout:") ? inFront : null;
  const groups = ordered.map((device) => groupOf(rest, device, name(device.id), device.id === frontId, currentKey, t));
  // Only a project-scoped start offers this initial default. Explicit checkout,
  // Home, issue, PR and resume targets keep their existing meaning.
  let projectDefault: string | null = null;
  if (projectId && !settings) {
    for (const group of groups) {
      const project = group.projects.find((row) => row.key === projectKey(group.deviceId, projectId));
      if (!project?.worktree) continue;
      const workspace = ordered.find((row) => row.id === group.deviceId);
      const record = workspace && deviceWorkspaces(rest, workspace).workspaces.find((row) => row.id === projectId);
      const location = record ? projectSettings(rest, settingsTarget(record)).record?.new_work : undefined;
      projectDefault = location === "worktree" ? project.worktree.key : (project.branches[0]?.key ?? null);
      break;
    }
  }
  const items = groups.flatMap((group) => group.items);
  const usable = (key: string) => items.some((item) => item.key === key && item.disabled === null);
  const front = groups[0];
  const wanted = projectDefault ?? inFront;
  const defaultKey =
    (wanted && usable(wanted) ? wanted : null) ?? (front && usable(homeKey(front.deviceId)) ? homeKey(front.deviceId) : null) ?? items.find((item) => item.disabled === null)?.key ?? null;
  return { groups, defaultKey, currentKey };
}

/**
 * The target a start goes to: the operator's pick while it is listed, else the
 * default. A pick whose device stopped answering stays chosen and disabled, so
 * the panel locks its start instead of quietly moving it somewhere else.
 */
export function resolveTarget(targets: StartTargets, chosen: string | null): StartTarget | null {
  const items = targets.groups.flatMap((group) => group.items);
  const pick = chosen ? items.find((item) => item.key === chosen) : undefined;
  return pick ?? items.find((item) => item.key === targets.defaultKey) ?? null;
}

/** The project row a target belongs to, for the project chip and its branch menu. */
export function projectOf(targets: StartTargets, target: StartTarget | null): StartProject | null {
  if (!target) return null;
  for (const group of targets.groups) {
    const project = group.projects.find((row) => row.worktree?.key === target.key || row.branches.some((branch) => branch.key === target.key));
    if (project) return project;
  }
  return null;
}
