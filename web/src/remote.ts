// The selected SSH device as the shell's context (PRD S5 B19). The core
// already connects to the device, projects its Herdr session with ids scoped
// to the target, attaches the panes of that Herdr's visible tab, and routes
// `remote_control` to the exact host after checking every id against the
// session. This module only reads that projection and builds the events, so
// the sidebar, the strip, the canvas and every command resolve one context the
// same way and a remote action can never be built from a local id.

import type { TFunction } from "i18next";
import type { AgentRow, Checkout, Device, RemotePaneLayout, RemoteSession, RemoteStatus, SnapshotRest, Tab, Workspace } from "./snapshot";
import { findArea } from "./areaLayout";
import type { WorkspaceView } from "./workspace";
import { machineName } from "./screenMachine";

export type RemoteContext = {
  device: Device;
  /** The core's connection row for the device; null before its first attempt is recorded. */
  status: RemoteStatus | null;
  /** The last session the core read from that host; kept while the connection is `stale`. */
  session: RemoteSession | null;
};

/**
 * The SSH device the operator selected, with its connection and session, or
 * null while this machine is the context.
 */
export function remoteContext(rest: SnapshotRest | null): RemoteContext | null {
  const id = rest?.navigator?.focused_device_id;
  if (!id) return null;
  const device = rest?.navigator?.devices?.find((row) => row.id === id && row.kind === "remote");
  if (!device) return null;
  const status = rest?.status?.remote?.find((row) => row.target_id === id) ?? null;
  return { device, status, session: status?.session ?? null };
}

/** Whether a command may go to the host now; the core refuses the rest with `remote.control.not_connected`. */
export function remoteConnected(context: RemoteContext): boolean {
  return context.status?.state === "connected" && context.session !== null;
}

export type RemoteView = {
  workspace: Workspace;
  checkout: Checkout;
  tab: Tab | null;
  layout: RemotePaneLayout | null;
  /** The pane that holds keyboard focus on that host, inside `tab`. */
  focusedPaneId: string | null;
};

function tabById(session: RemoteSession, tabId: string | null | undefined): { workspace: Workspace; checkout: Checkout; tab: Tab } | null {
  if (!tabId) return null;
  for (const workspace of session.workspaces) {
    for (const checkout of workspace.checkouts) {
      const tab = checkout.tabs.find((row) => row.id === tabId);
      if (tab) return { workspace, checkout, tab };
    }
  }
  return null;
}

/**
 * What the canvas draws for a remote session. The tab is the one the core
 * attaches (`remote_terminal_pane_sets`): the host's focused tab, else the
 * active tab of its focused workspace. Drawing any other tab would show panes
 * nobody streams, so the web follows the host's focus and moves it with
 * `remote_control` rather than keeping a selection of its own.
 */
export function remoteView(session: RemoteSession | null, front?: WorkspaceView | null): RemoteView | null {
  if (!session) return null;
  if (front?.agent_layout) {
    const workspace = session.workspaces.find((row) => row.device_id === front.device_id && row.checkouts.some((checkout) => checkout.path === front.path));
    const checkout = workspace?.checkouts.find((row) => row.path === front.path);
    if (workspace && checkout) {
      const area = findArea(front.agent_layout.root, front.agent_layout.active_area);
      const shown = area ? front.agent_layout.canvases[area.id] ?? area.active : null;
      const tab = checkout.tabs.find((row) => row.id === shown) ?? null;
      const layout = session.pane_layouts.find((row) => row.tab_id === tab?.id) ?? null;
      const focusedPaneId = tab?.panes.find((pane) => pane.id === session.focused_pane_id)?.id ?? layout?.focused_pane_id ?? tab?.panes[0]?.id ?? null;
      return { workspace, checkout, tab, layout, focusedPaneId };
    }
  }
  const activeTabId =
    session.focused_tab_id ?? (session.focused_checkout_id ? session.active_tab_ids[session.focused_checkout_id] : undefined);
  const found = tabById(session, activeTabId);
  let workspace = found?.workspace ?? null;
  let checkout = found?.checkout ?? null;
  if (!workspace || !checkout) {
    workspace =
      session.workspaces.find((row) => row.checkouts.some((candidate) => candidate.id === session.focused_checkout_id)) ??
      session.workspaces.find((row) => row.id === session.focused_workspace_id) ??
      session.workspaces[0] ??
      null;
    checkout = workspace?.checkouts.find((row) => row.id === session.focused_checkout_id) ?? workspace?.checkouts[0] ?? null;
  }
  if (!workspace || !checkout) return null;
  const tab = found?.tab ?? null;
  const layout = tab?.id ? (session.pane_layouts.find((row) => row.tab_id === tab.id) ?? null) : null;
  const inTab = (paneId: string | null | undefined) => (paneId && tab?.panes.some((pane) => pane.id === paneId) ? paneId : null);
  const focusedPaneId = inTab(session.focused_pane_id) ?? inTab(layout?.focused_pane_id) ?? tab?.panes[0]?.id ?? null;
  return { workspace, checkout, tab, layout, focusedPaneId };
}

/**
 * The view of `tabId` beside `view`'s: an Agent area of a node that dialed in
 * shows a tab other than the one its Herdr has in front, and the core attaches
 * that tab's panes too (`node_shown_agent_tabs`). The keyboard stays on the
 * front tab's pane, so no pane of another tab is drawn focused.
 */
export function remoteTabView(session: RemoteSession, view: RemoteView, tabId: string | null): RemoteView {
  if (!tabId || tabId === view.tab?.id) return view;
  const tab = view.checkout.tabs.find((row) => row.id === tabId) ?? null;
  const layout = tab ? (session.pane_layouts.find((row) => row.tab_id === tabId) ?? null) : null;
  return { ...view, tab, layout, focusedPaneId: null };
}

/**
 * The target a pane id belongs to, read from the core's connection rows: a
 * remote pane id is `remote:<target>:pane:<id>` for a target the core knows.
 * Null means the pane is this machine's.
 */
export function remoteTargetOfPane(rest: SnapshotRest | null, paneId: string): string | null {
  for (const status of rest?.status?.remote ?? []) {
    if (paneId.startsWith(`remote:${status.target_id}:pane:`)) return status.target_id;
  }
  return null;
}

/**
 * The id the pane's own Herdr knows it by (`w9J:p52`): what `herdr pane read`
 * and `$HERDR_PANE_ID` name on that device. The core scopes a device's pane
 * as `remote:<target>:pane:<id>`, the same split its label and Codex keys use.
 */
export function herdrPaneId(paneId: string): string {
  if (!paneId.startsWith("remote:")) return paneId;
  const at = paneId.indexOf(":pane:", "remote:".length);
  return at === -1 ? paneId : paneId.slice(at + ":pane:".length);
}

/** The agents the sidebar lists: the selected host's, or this machine's. */
export function contextAgents(rest: SnapshotRest | null, localAgents: AgentRow[]): AgentRow[] {
  const context = remoteContext(rest);
  return context ? (context.session?.agents ?? NO_AGENTS) : localAgents;
}

/** Every workspace of the context on screen, the device's Home (`~/hide`) included: what holds a pane. */
export function contextAllWorkspaces(rest: SnapshotRest | null): Workspace[] {
  const context = remoteContext(rest);
  return context ? (context.session?.workspaces ?? NO_WORKSPACES) : (rest?.navigator?.workspaces ?? NO_WORKSPACES);
}

const projectLists = new WeakMap<Workspace[], Workspace[]>();

/**
 * `workspaces` without the device's Home, which is drawn as its own row and
 * counted as no project (PRD home-device-rail D-13). The list itself when it
 * holds no Home, and one cached list per source otherwise, so a selector
 * returning it keeps its identity between snapshots.
 */
export function projectsOf(workspaces: Workspace[]): Workspace[] {
  if (!workspaces.some((row) => row.is_home)) return workspaces;
  let listed = projectLists.get(workspaces);
  if (!listed) {
    listed = workspaces.filter((row) => !row.is_home);
    projectLists.set(workspaces, listed);
  }
  return listed;
}

/** The projects the sidebar lists: the selected host's Herdr workspaces, or this machine's projects, never the Home. */
export function contextWorkspaces(rest: SnapshotRest | null): Workspace[] {
  return projectsOf(contextAllWorkspaces(rest));
}

const NO_AGENTS: AgentRow[] = [];
const NO_WORKSPACES: Workspace[] = [];

let requestSequence = 0;

/**
 * A fresh id for one `remote_control` request. The core keeps the ids it has
 * seen per target and drops a repeat, so a resent frame cannot split twice.
 */
export function remoteRequestId(): string {
  requestSequence += 1;
  const random = globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `web-${random}-${requestSequence}`;
}

export type RemoteAction =
  | { action: "focus_pane"; pane_id: string }
  | { action: "split_pane"; pane_id: string; direction: "right" | "down"; cwd: string | null }
  | { action: "toggle_pane_zoom"; pane_id: string }
  | { action: "close_pane"; pane_id: string; confirmed: boolean }
  | { action: "focus_workspace"; workspace_id: string; checkout_id: string }
  | { action: "focus_tab"; tab_id: string }
  | { action: "create_tab"; workspace_id: string; checkout_id: string; cwd: string; label: string }
  | { action: "close_tab"; tab_id: string; confirmed: boolean };

/** One `remote_control` event for `targetId` (core `RemoteControlPayload`). */
export function remoteControl(targetId: string, request: RemoteAction, requestId: string = remoteRequestId()) {
  return { schema_version: 2, kind: "remote_control", payload: { target_id: targetId, request_id: requestId, ...request } };
}

/** The same request, also bringing its device forward once it is sent: one event for both (S6 B21). */
export function withDeviceForward(event: ReturnType<typeof remoteControl>) {
  return { ...event, payload: { ...event.payload, focus_device: true } };
}

/** A remote pane's rectangle as CSS percentages of the canvas. */
export function frameStyle(frame: { x: number; y: number; width: number; height: number }): Record<string, string> {
  const percent = (value: number) => `${Math.max(0, Math.min(1, value)) * 100}%`;
  return { left: percent(frame.x), top: percent(frame.y), width: percent(frame.width), height: percent(frame.height) };
}

/** Whether Herdr on the host is new enough to store a purpose (the native `supportsRemotePurpose`). */
export function supportsRemotePurpose(version: string | null | undefined): boolean {
  if (!version) return false;
  const numeric = (version.startsWith("v") ? version.slice(1) : version).split(/[-+]/)[0] ?? "";
  const parts = numeric.split(".").map((part) => Number.parseInt(part, 10));
  if (parts.length !== 3 || parts.some((part) => !Number.isFinite(part))) return false;
  const [major, minor, patch] = parts as [number, number, number];
  if (major !== 0) return major > 0;
  if (minor !== 9) return minor > 9;
  return patch >= 1;
}

export type DeviceCatalogLine = {
  state: "loading" | "error" | "stale" | "empty" | "resolving" | "unavailable" | "partial";
  text: string;
};

/**
 * What the project list of a selected device says above its rows (PRD S5.5
 * B4): still loading, failed, disconnected with the last list kept, empty,
 * or listed but not yet confirmed by the device's helper. Null when the list
 * is the device's confirmed projects. A failure is never shown as an empty
 * list, and an unconfirmed list is never shown as a confirmed one.
 */
export function deviceCatalogLine(context: RemoteContext, devices: readonly Device[] | undefined, t: TFunction<"translation">): DeviceCatalogLine | null {
  const device = machineName(devices, context.device.id, t);
  const status = context.status;
  const session = context.session;
  const connected = status?.state === "connected";
  if (!session) {
    if (!status || status.state === "not_connected" || status.state === "connecting") {
      return { state: "loading", text: t("devices.catalog.loading", { device }) };
    }
    const state = status.state.replace(/_/g, " ");
    return { state: "error", text: status.message ? t("devices.catalog.errorMessage", { device, state, message: status.message }) : t("devices.catalog.error", { device, state }) };
  }
  if (!connected) {
    return { state: "stale", text: t("devices.catalog.stale", { device }) };
  }
  if (session.workspaces.length === 0) {
    return { state: "empty", text: t("devices.catalog.empty", { device }) };
  }
  const catalog = status?.catalog;
  if (catalog?.state === "resolving") {
    return { state: "resolving", text: t("devices.catalog.resolving", { device }) };
  }
  if (catalog?.state === "unavailable") {
    return {
      state: "unavailable",
      text: catalog.message ? t("devices.catalog.unavailableMessage", { device, message: catalog.message }) : t("devices.catalog.unavailable", { device }),
    };
  }
  if (catalog && catalog.refused.length > 0) {
    return { state: "partial", text: t("devices.catalog.partial", { device, count: catalog.refused.length }) };
  }
  return null;
}
