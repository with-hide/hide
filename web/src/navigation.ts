import { scopeOccurrences, scopeRows, type AgentScope } from "./agentScope";
// All projects and Project Overview (PRD S6 D-02, B1-B3, B21): every registered
// Project on every device, and one Project's Workspaces and agents, read from
// the snapshot the core already publishes - the navigator for this machine,
// each device's projected Herdr session, the registrations for a device that
// has no session to show. Nothing here is counted that the snapshot does not
// carry, and a device that cannot answer says so instead of showing zeros.

import type { TFunction } from "i18next";
import type { TreeRow } from "./agentRow";
import type { MessageKey } from "./i18n/catalogs";
import { thisMachineKey } from "./devices";
import { translate } from "./i18n/client";
import { coreErrorText, herdrStateText, remoteStateText } from "./coreError";
import type { BoardProject } from "./projectBoard";
import { projectsOf } from "./remote";
import { machineName, windowDeviceId } from "./screenMachine";
import { hideAiPlace } from "./hideAiOrigin";
import { focusedRemoteDevice, frontCheckout, localDeviceId, type AgentRow, type Device, type HideAi, type RemoteStatus, type SnapshotRest, type Workspace, type WorkspaceRegistration } from "./snapshot";
import type { Tool } from "./workspace";
import { startAnswer } from "./startAnswer";

export type AgentGroup = "needs_you" | "done" | "working" | "seen";
export const AGENT_GROUPS: readonly { group: AgentGroup; label: MessageKey }[] = [
  { group: "needs_you", label: "agents.group.needs_you" },
  { group: "done", label: "agents.group.done" },
  { group: "working", label: "agents.group.working" },
  { group: "seen", label: "agents.group.seen" },
];

/** A group's heading; a group the core names that this list does not know is shown under its own wire name. */
export function agentGroupTitle(group: string, t: TFunction<"translation">): string {
  const known = AGENT_GROUPS.find((row) => row.group === group);
  return known ? t(known.label) : group.replace(/_/g, " ");
}

export type GroupCounts = Record<AgentGroup, number>;

/**
 * What retrying an unavailable device does: reconnect its session, or
 * restart its helper, whose new connection reads the catalog again.
 */
export type AvailabilityRetry = "connect" | "helper";

/** How far a device's projects can be trusted right now. */
export type DeviceAvailability =
  | { state: "ready" }
  | { state: "loading"; text: string }
  | { state: "unavailable"; text: string; retry: AvailabilityRetry | null };

export type ProjectEntry = {
  id: string;
  label: string;
  path: string;
  deviceId: string;
  pinned: boolean;
  /** The Project's catalog row, or null when its device has no session to show it from. */
  workspace: Workspace | null;
  workspaceCount: number | null;
  counts: GroupCounts | null;
};

export type DeviceSection = {
  device: Device;
  local: boolean;
  availability: DeviceAvailability;
  projects: ProjectEntry[];
};

/** The agents running in a Project, in the core's order. */
export function projectAgents(workspace: Workspace, agents: AgentRow[]): AgentRow[] {
  return scopeOccurrences(workspace.agent_scope.rows, agents);
}

export function groupCounts(scope: AgentScope): GroupCounts { return scope.groups; }

export type AgentSection = { group: string; agents: AgentRow[] };

/**
 * The Agents explorer's sections (S6 B13): every current agent under Needs
 * You, Done, Working and Seen, in the core's order within each, with an
 * empty group left out. A group the core names that this list does not know
 * is still shown, under its own name, rather than dropping its rows.
 */
export function agentSections(scope: AgentScope, agents: AgentRow[]): AgentSection[] {
  return scope.group_rows.map((group) => ({ group: group.group, agents: scopeOccurrences(group.rows, agents) }));
}

/** An agent row and, for a row on an SSH device, the device's name. */
export type ListedAgent = { agent: AgentRow; device: string | null };

/**
 * Every current agent the snapshot carries (S6 B13, D-12): this machine's
 * and each connected device's, each device's rows in the order its core
 * gave them. A device that is not connected lists nothing, since what it
 * last reported is not current.
 */
export function allAgents(remote: RemoteStatus[] | undefined, localAgents: AgentRow[], scope: AgentScope | null | undefined, devices?: readonly Device[]): ListedAgent[] {
  return (scope?.listed ?? []).map((entry) => {
    const agent = entry.remote ? remote?.find((r) => r.target_id === entry.device_id)?.session?.agents[entry.index] : localAgents[entry.index];
    if (!agent || agent.pane_id !== entry.pane_id) throw new Error(`Missing listed agent: ${entry.pane_id}`);
    // Each machine as this window calls it (PRD core-host-node-move B2); the core's own word before devices are listed.
    const label = devices ? machineName(devices, entry.device_id, translate) : (entry.device_label ?? translate(thisMachineKey(null)));
    return { agent: { ...agent, device_id: entry.device_id, device_label: label }, device: entry.remote ? label : null };
  });
}

/** One device's core-projected current rows, in their published order. */
export function deviceListedAgents(remote: RemoteStatus[] | undefined, devices: Device[] | undefined, localAgents: AgentRow[], deviceId: string): ListedAgent[] {
  return allAgents(remote, localAgents, devices?.find((d) => d.id === deviceId)?.agent_scope, devices);
}

/** A device's rows' key in `agentPlaces`: the name `allAgents` gives them, so a node's own rows in its window find their places too. */
function deviceLabel(devices: Device[] | undefined, targetId: string): string {
  return machineName(devices, targetId, translate);
}

/**
 * Where each listed agent works, as the Agents list's context line names it
 * (PRD sidebar-readability B5): `project › checkout`, or the project alone for
 * a plain folder, which is one row in Projects. A pane is placed by the
 * checkout that holds it, on its own device: this machine's projects for a
 * local row, a connected device's Herdr workspaces for its rows. A pane no
 * checkout holds has no place, and none is made up.
 */
export function agentPlaces(
  localWorkspaces: Workspace[] | undefined,
  remote: RemoteStatus[] | undefined,
  devices: Device[] | undefined,
  hideAi?: ReadonlyMap<string, HideAi>,
): (device: string | null, paneId: string) => string | null {
  const byDevice = new Map<string | null, Map<string, string>>([[null, checkoutPlaces(localWorkspaces ?? [], hideAi)]]);
  for (const status of remote ?? []) {
    byDevice.set(deviceLabel(devices, status.target_id), checkoutPlaces((status.session?.workspaces ?? []).filter((p) => p.agent_scope.places_live), hideAi));
  }
  return (device, paneId) => byDevice.get(device)?.get(paneId) ?? null;
}

/**
 * Where each pane sits, as `project › branch` (Home, or the project alone for
 * a plain folder), keyed by pane id. A pane whose agent Hide AI started names
 * its task where the branch would stand (`project › Memory 올리기 · m72`).
 */
export function checkoutPlaces(workspaces: Workspace[], hideAi?: ReadonlyMap<string, HideAi>): Map<string, string> {
  const places = new Map<string, string>();
  for (const workspace of workspaces) {
    for (const [pane, value] of Object.entries(workspace.agent_scope.places)) {
      const checkout = workspace.checkouts.find((c) => c.id === value.checkout_id);
      if (!checkout) throw new Error(`Missing place checkout: ${value.checkout_id}`);
      const started = hideAi?.get(pane);
      places.set(pane, value.kind === "home" ? translate("common.home") : value.kind === "folder" ? workspace.label : started ? hideAiPlace(translate, workspace.label, started) : `${workspace.label} › ${checkout.branch ?? checkout.label}`);
    }
  }
  return places;
}

/**
 * How many live descendants each agent has among the rows the core lists
 * (B13), keyed by pane, from one index of the rows rather than one per row.
 */
export function liveDescendantCounts(scope: AgentScope): Map<string, number> {
  return new Map(Object.entries(scope.descendants));
}

/**
 * This machine's Projects count agents only while its Herdr answers: an
 * unreachable Herdr has no agents to list, and "No agents" would be a zero
 * nobody measured (B3). The core reconnects on its own and has no event that
 * reconnects sooner, so there is no Retry.
 */
function localAvailability(rest: SnapshotRest | null): DeviceAvailability {
  const herdr = rest?.status?.herdr;
  if (!herdr?.state || herdr.state === "connected") return { state: "ready" };
  if (herdr.state === "not_connected") return { state: "loading", text: translate("overview.availability.connectingHerdr") };
  // A Herdr on another protocol says so in the operator's language; Overview draws its move in this place (PRD herdr-takeover B16).
  if (herdr.state === "protocol_mismatch") return { state: "unavailable", text: translate("herdrTakeover.overview.mismatch"), retry: null };
  // The state is worded here; Herdr's own sentence, with its socket path, is the About tab's and the log's.
  return { state: "unavailable", text: translate("coreError.herdr.countsLater", { reason: herdrStateText(herdr.state, translate) }), retry: null };
}

/** `name`: the device as this window calls it (`machineName`). */
function deviceAvailability(device: Device, status: RemoteStatus | undefined, name: string): DeviceAvailability {
  if (device.kind !== "remote") return { state: "ready" };
  if (!status || status.state === "not_connected") return { state: "loading", text: translate("devices.state.connecting") };
  if (status.state !== "connected" && status.state !== "stale") {
    return { state: "unavailable", text: remoteStateText(status.state, name, translate), retry: status.state === "disabled" ? null : "connect" };
  }
  if (status.state === "stale") {
    return { state: "unavailable", text: translate("overview.availability.deviceStale", { device: name }), retry: "connect" };
  }
  const catalog = status.catalog;
  if (catalog?.state === "resolving") return { state: "loading", text: translate("overview.availability.readingProjects") };
  if (catalog?.state === "unavailable") return { state: "unavailable", text: translate("overview.availability.projectsUnreadable"), retry: "helper" };
  return { state: "ready" };
}

function entryOf(workspace: Workspace, _agents: AgentRow[], trusted: boolean, checkoutsTrusted = trusted): ProjectEntry {
  return {
    id: workspace.id,
    label: workspace.label,
    path: workspace.path,
    deviceId: workspace.device_id,
    pinned: workspace.pinned,
    workspace,
    workspaceCount: checkoutsTrusted ? workspace.checkouts.length : null,
    counts: trusted ? workspace.agent_scope.groups : null,
  };
}

function registrationEntry(registration: WorkspaceRegistration): ProjectEntry {
  return {
    id: registration.id,
    label: registration.label,
    path: registration.path,
    deviceId: registration.device_id,
    pinned: registration.pinned,
    workspace: null,
    workspaceCount: null,
    counts: null,
  };
}

function byPinThenLabel(left: ProjectEntry, right: ProjectEntry): number {
  if (left.pinned !== right.pinned) return left.pinned ? -1 : 1;
  return left.label.localeCompare(right.label);
}

/**
 * The Overview of a device's projects: one section per device, this machine
 * first, or the one `deviceId` names. A device with a session lists its
 * projected Projects; one without lists its registrations with no counts,
 * marked loading or unavailable, so nothing is guessed (B3, B21). The
 * device's Home is no project and is not listed.
 */
export function mainSections(rest: SnapshotRest | null, localAgents: AgentRow[], deviceId?: string): DeviceSection[] {
  const devices = (rest?.navigator?.devices ?? []).filter((device) => deviceId === undefined || device.id === deviceId);
  const registrations = (rest?.ui_state?.workspace_registrations ?? []).filter((row) => !row.home);
  const sections: DeviceSection[] = [];
  const local = devices.find((device) => device.kind !== "remote");
  if (local) {
    const availability = localAvailability(rest);
    // The checkouts are this machine's own facts; only the agents need Herdr.
    const projects = projectsOf(rest?.navigator?.workspaces ?? []).map((workspace) => entryOf(workspace, localAgents, availability.state === "ready", true));
    sections.push({ device: local, local: true, availability, projects: [...projects].sort(byPinThenLabel) });
  }
  for (const device of devices.filter((row) => row.kind === "remote")) {
    const status = rest?.status?.remote?.find((row) => row.target_id === device.id);
    const availability = deviceAvailability(device, status, machineName(rest?.navigator?.devices, device.id, translate));
    const session = status?.session ?? null;
    const projects = session
      ? projectsOf(session.workspaces).map((workspace) => entryOf(workspace, session.agents, availability.state === "ready"))
      : registrations.filter((row) => row.device_id === device.id).map(registrationEntry);
    sections.push({ device, local: false, availability, projects: [...projects].sort(byPinThenLabel) });
  }
  return sections;
}

/**
 * How many Projects an Overview lists, counted the way `mainSections` lists
 * them: this machine's catalog, and each device's session or, before it
 * answers, its registrations, Home excluded. A number, so a caller reads it
 * without building the sections.
 */
export function allProjectsCount(rest: SnapshotRest | null, deviceId?: string): number {
  const devices = (rest?.navigator?.devices ?? []).filter((device) => deviceId === undefined || device.id === deviceId);
  const registrations = (rest?.ui_state?.workspace_registrations ?? []).filter((row) => !row.home);
  let count = devices.some((device) => device.kind !== "remote") ? projectsOf(rest?.navigator?.workspaces ?? []).length : 0;
  for (const device of devices.filter((row) => row.kind === "remote")) {
    const session = rest?.status?.remote?.find((row) => row.target_id === device.id)?.session ?? null;
    count += session ? projectsOf(session.workspaces).length : registrations.filter((row) => row.device_id === device.id).length;
  }
  return count;
}

/**
 * Every Project All projects' boards read, with the agents its device last
 * reported and the device's name when it is not this machine: this
 * machine's catalog, then each device's session. A device with no session
 * has no catalog rows to draw, and its section says why on the Projects view.
 */
export function boardProjects(rest: SnapshotRest | null, localAgents: AgentRow[], deviceId?: string): BoardProject[] {
  const localId = localDeviceId(rest);
  const devices = rest?.navigator?.devices;
  // A board names the machine of a project that is not the window's own, the core's included on a node's window.
  const away = (id: string) => (id === windowDeviceId(devices) ? null : machineName(devices, id, translate));
  const projects: BoardProject[] =
    deviceId === undefined || deviceId === localId
      ? projectsOf(rest?.navigator?.workspaces ?? []).map((workspace) => ({ workspace, agents: localAgents, device: away(localId) }))
      : [];
  for (const status of rest?.status?.remote ?? []) {
    const session = status.session;
    if (!session || (deviceId !== undefined && status.target_id !== deviceId)) continue;
    const device = away(status.target_id);
    for (const workspace of projectsOf(session.workspaces)) projects.push({ workspace, agents: session.agents, device });
  }
  return projects;
}

export type OverviewProject = {
  workspace: Workspace;
  /**
   * Every agent its device last reported, current or not: the Overview board
   * keeps the last rows while a device is unreachable, and a descendant may
   * work in another project's checkout (web-project-overview D-06).
   */
  deviceAgents: AgentRow[];
  device: Device | null;
  availability: DeviceAvailability;
};

/**
 * The Project an Overview shows, on whichever device it lives, with the
 * agents that device last reported and how far it can be trusted now. The
 * board keeps those rows while the device is unreachable, and the device's
 * availability is the only thing that says so (web-project-overview D-06).
 */
export function overviewProject(rest: SnapshotRest | null, localAgents: AgentRow[], projectId: string): OverviewProject | null {
  const local = rest?.navigator?.workspaces?.find((row) => row.id === projectId);
  if (local) {
    const availability = localAvailability(rest);
    return {
      workspace: local,
      deviceAgents: localAgents,
      device: rest?.navigator?.devices?.find((row) => row.kind !== "remote") ?? null,
      availability,
    };
  }
  for (const status of rest?.status?.remote ?? []) {
    const workspace = status.session?.workspaces.find((row) => row.id === projectId);
    if (workspace) {
      const device = rest?.navigator?.devices?.find((row) => row.id === status.target_id) ?? null;
      const availability = device ? deviceAvailability(device, status, machineName(rest?.navigator?.devices, device.id, translate)) : { state: "loading" as const, text: translate("devices.state.connecting") };
      return {
        workspace,
        deviceAgents: status.session?.agents ?? [],
        device,
        availability,
      };
    }
  }
  return null;
}

/** Herdr states in which a catalog will not change on its own soon. */
const SETTLED_HERDR = new Set(["connected", "unconfigured", "socket_missing", "unreachable", "stale", "incompatible"]);
const FAILED_HERDR = new Set([...SETTLED_HERDR].filter(state => state !== "connected"));

/** Whether the local saved choice is pending, rather than failed or restored. */
export function waitingForSavedWorkspace(rest: SnapshotRest | null): boolean {
  // Herdr can connect before the asynchronous catalog and persisted View
  // arrive. The core retains a saved choice until its path facts settle;
  // absence during that restore must not choose Main permanently.
  return Boolean(rest && !focusedRemoteDevice(rest) && !FAILED_HERDR.has(rest.status?.herdr?.state ?? "") && rest.ui_state?.focused_checkout_id && !rest.workspace_view);
}

/**
 * The first screen (S6 B19, D-11): the Workspace in front when the core
 * marks it as the one the operator last chose, now or before a restart,
 * All projects when it is not (a first run, or a last Workspace that is gone) or when the device in front
 * has settled without one, and null while that device is still being
 * reached. The device in front decides: a device Workspace is known only
 * once that device's session arrives, and this machine's Herdr settling
 * first says nothing about it.
 */
export function startupScreen(rest: SnapshotRest | null, hasFront: boolean): "workspace" | "main" | null {
  if (!rest) return null;
  const device = focusedRemoteDevice(rest);
  if (waitingForSavedWorkspace(rest)) return null;
  if (hasFront) return rest.workspace_view?.resumed ? "workspace" : "main";
  if (!device) {
    const state = rest.status?.herdr?.state;
    return state && SETTLED_HERDR.has(state) ? "main" : null;
  }
  const status = rest.status?.remote?.find((row) => row.target_id === device.id);
  if (!status || status.state === "not_connected") return null;
  if (status.state === "connected" && !status.session) return null;
  return "main";
}

/** What All projects, an Overview or the Agents list asked to bring forward (B2, B12, B21). */
export type OpenTarget = { checkoutId: string; deviceId: string; path: string | null; workspaceId?: string; expanded?: boolean; tool?: Tool } | { paneId: string } | { startRequestId: string };

/**
 * A Workspace or agent asked for and not yet in front. The screen changes
 * only once the core has moved there, so a refusal leaves the operator where
 * they were, with the reason. `errorBefore` is the core's last error when the
 * request went out; a newer one is this request's refusal.
 */
export type Opening = { target: OpenTarget; errorBefore: number | null; failure: string | null };

/** How long an open may take before it is reported as not having happened. */
export const OPEN_ANSWER_TIMEOUT_MS = 15_000;

/** Whether the Workspace in front is the one an open asked for. */
export function openingLanded(rest: SnapshotRest | null, target: OpenTarget): boolean {
  const front = frontCheckout(rest);
  if (!front) return false;
  if ("startRequestId" in target) {
    const answer = startAnswer(rest, target.startRequestId);
    return answer.phase === "ready" && answer.paneId !== null && rest?.focused?.pane_id === answer.paneId
      && front.tabs.some((tab) => tab.panes.some((pane) => pane.id === answer.paneId));
  }
  if ("paneId" in target) return front.tabs.some((tab) => tab.panes.some((pane) => pane.id === target.paneId));
  if (target.tool !== undefined) {
    const view = rest?.workspace_view;
    if (!view || view.tool !== target.tool || view.device_id !== target.deviceId || view.path !== front.path) return false;
  }
  if (target.expanded !== undefined && !(rest?.ui_state?.session_collapsed_checkout_ids ?? []).includes(target.checkoutId) !== target.expanded) return false;
  if (front.id === target.checkoutId) return true;
  // A device project with no Herdr workspace yet gets one at its folder, under a new id.
  return target.path !== null && front.path === target.path && (rest?.navigator?.focused_device_id ?? localDeviceId(rest)) === target.deviceId;
}

/** A vanished local target is cancelled silently; other refusals keep their existing message. */
export function openingProgress(rest: SnapshotRest | null, opening: Opening): "landed" | string | null {
  const target = opening.target;
  if ("startRequestId" in target) {
    const answer = startAnswer(rest, target.startRequestId);
    // The start panel owns its refusal and draft. This intent only presents
    // the core's accepted selection; it never sends another focus request.
    if (answer.phase === "refused" || answer.phase === "failed") return "cancelled";
    return openingLanded(rest, target) ? "landed" : null;
  }
  if (rest?.navigator?.workspaces && "checkoutId" in target && target.deviceId === localDeviceId(rest) && target.workspaceId !== undefined
    && !rest.navigator.workspaces.some((workspace) => workspace.id === target.workspaceId && workspace.checkouts.some((checkout) => checkout.id === target.checkoutId))) return "cancelled";
  if (openingLanded(rest, target)) return "landed";
  const error = rest?.status?.last_error;
  if (error && error.occurred_at !== opening.errorBefore) return coreErrorText(error, translate);
  return null;
}

/**
 * The sections and the rows each draws, from one index per device: pane ids
 * are scoped to the device that reported them, so a lineage never crosses
 * devices.
 */
export type AgentTree = {
  sections: { group: string; rows: TreeRow[]; count: number }[];
  children: (device: string | null, agent: AgentRow) => AgentRow[];
};

export function agentTree(listed: ListedAgent[], scope: AgentScope | null): AgentTree {
  const index = new Map(listed.map((row) => [row.agent, row]));
  const agents = listed.map((row) => row.agent);
  return {
    sections: scope?.sections.map((section) => ({ ...section, rows: scopeOccurrences(section.rows, agents).map((agent, i) => {
      const row = section.rows[i]!;
      const listed = index.get(agent);
      if (!listed) throw new Error(`Agent section references a missing row: ${row.pane_id}`);
      return { ...listed, depth: row.depth, descendants: row.descendants };
    }) })) ?? [],
    children: (_device, agent) => scopeRows(scope?.children[agent.pane_id] ?? [], listed.map((r) => r.agent)),
  };
}

/**
 * Every row the Agents list draws, top to bottom across its sections: the
 * order ⌥1-9 numbers (PRD electron-digit-shortcuts-hints D-02). A folded
 * parent's descendants are not rows, so they take no number.
 */
export function agentListRows(tree: AgentTree): TreeRow[] {
  return tree.sections.flatMap((section) => section.rows);
}
