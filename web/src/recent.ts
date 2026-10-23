// Recent navigation (docs/UI_BEHAVIOR.md, Recent navigation), kept for the
// session.
//
// One recent-use order over every surface the shell holds, across every
// device, project and checkout: this machine's Herdr tabs and the View-area
// displays (file, diff and browser) of each checkout's Workspace, each
// connected device's Herdr tabs (PRD home-device-rail D-16), and the page's
// shared Overview page and its scope, once the
// operator has been on them.
// Recent Panels walks it; Recent Projects walks the projects in the order
// they were used and restores each one's last Workspace surface, which is
// this same order narrowed to the project. Beside it, one recent-use order
// of the terminal panes the keyboard has been in, on every device, project
// and checkout, whose agent panes the Agent area's cycle walks (issue 301). The core reports what is in
// front, not what was before, and the screens are the page's own navigation,
// so these orders are the shell's convenience: nothing here is authority, and a
// reload rebuilds them from use. The pane order is the exception: the page
// still decides what a visit is, since only it knows where the keyboard is,
// but the core keeps the order (`ui_state.recent_pane_ids`) and saves it, so
// the Agent cycle survives a reload and a restart.

import { frontDeviceId, localDeviceId } from "./devices";
import { allProjectsCount } from "./navigation";
import { translate } from "./i18n/client";
import { remoteContext, remoteView } from "./remote";
import { machineName, windowDeviceId } from "./screenMachine";
import { type AgentRow, type Checkout, type PaneRow, type SnapshotRest, type Tab, type ViewDisplaySnapshot, type Workspace } from "./snapshot";
import type { Screen } from "./ui";
import { activeDisplay, areasOf } from "./viewLayout";
import { workspaceViewOf } from "./workspace";

/** A Herdr tab, or what a View-area display shows. */
export type SurfaceKind = "herdr" | ViewDisplaySnapshot["kind"];

const AGENT_SURFACE = "herdr";

/** One surface a Recent Panels row stands for, with what committing it names. */
export type Surface = {
  key: string;
  kind: SurfaceKind;
  /** The device the checkout is on; the core's own node id for this machine's. */
  deviceId: string;
  workspaceId: string;
  checkoutId: string;
  /** The Herdr tab id, or the display id within its checkout's Workspace. */
  id: string;
  /** A display's name as its View area last drew it; a tab's comes from the snapshot. */
  label: string;
};

export function tabSurface(checkout: Checkout, tabId: string, deviceId: string): Surface {
  return { key: `tab\u0000${checkout.id}\u0000${tabId}`, kind: AGENT_SURFACE, deviceId, workspaceId: checkout.workspace_id, checkoutId: checkout.id, id: tabId, label: "" };
}

export function displaySurface(checkout: Checkout, display: ViewDisplaySnapshot, deviceId: string): Surface {
  return { key: `display\u0000${checkout.id}\u0000${display.id}`, kind: display.kind, deviceId, workspaceId: checkout.workspace_id, checkoutId: checkout.id, id: display.id, label: display.label };
}

/** A screen of the page's own: a device's Home Overview, or one Project's Overview. */
export type PageScreen = Extract<Screen, { kind: "main" }>;

/**
 * A page screen in the recent order. It joins the order when the page shows
 * it, since it is somewhere the operator goes rather than something the
 * session holds open, and an Overview leaves with its Project.
 */
export type ScreenVisit = { key: string; screen: PageScreen };

/** One entry of the recent order: a Workspace surface, or a screen of the page's own. */
export type RecentEntry = Surface | ScreenVisit;

/** A Home Overview that names no device is the front device's; the visit names it, so it stays one place after the front moves. */
function screenVisit(screen: PageScreen, frontId: string): ScreenVisit {
  {
    const deviceId = screen.deviceId ?? frontId;
    return { key: `main:${deviceId}`, screen: { ...screen, deviceId } };
  }
}

export function isScreenVisit(entry: RecentEntry): entry is ScreenVisit {
  return "screen" in entry;
}

/** A device's Home Overview while the device is registered; an Overview while its Project is in the catalog, on any device. */
function screenExists(rest: SnapshotRest | null, screen: PageScreen): boolean {
  return (rest?.navigator?.devices ?? []).some((device) => device.id === (screen.deviceId ?? localDeviceId(rest)));
}

/** The core's own node's checkouts, in the navigator's order. */
function localCheckouts(rest: SnapshotRest | null): Checkout[] {
  return (rest?.navigator?.workspaces ?? []).filter((workspace) => workspace.device_id === localDeviceId(rest)).flatMap((workspace) => workspace.checkouts);
}

/** A checkout with the device it is on. */
type DeviceCheckout = { deviceId: string; workspace: Workspace; checkout: Checkout };

/**
 * Every checkout the shell can bring forward: this machine's, then each
 * connected device's from the session the core last read, which a device
 * that is not connected has no current copy of.
 */
function allCheckouts(rest: SnapshotRest | null): DeviceCheckout[] {
  const local = localDeviceId(rest);
  const rows: DeviceCheckout[] = (rest?.navigator?.workspaces ?? [])
    .filter((workspace) => workspace.device_id === local)
    .flatMap((workspace) => workspace.checkouts.map((checkout) => ({ deviceId: local, workspace, checkout })));
  for (const status of rest?.status?.remote ?? []) {
    if (status.state !== "connected") continue;
    for (const workspace of status.session?.workspaces ?? []) {
      for (const checkout of workspace.checkouts) rows.push({ deviceId: status.target_id, workspace, checkout });
    }
  }
  return rows;
}

/** The Workspace in front when it is this checkout's, which is the only one whose displays the snapshot carries. */
function frontLayoutOf(rest: SnapshotRest | null, checkout: Checkout) {
  const view = workspaceViewOf(rest);
  return view && view.device_id === localDeviceId(rest) && view.path === checkout.path ? (view.layout ?? null) : null;
}

/**
 * The surface the operator is using now: the focused checkout's active
 * display while the View area is the one in use, else its visible Herdr
 * tab; over a device in front, the tab that device's Herdr shows.
 * `viewInUse` is the page's answer (`keyboardOwner`), since only the page
 * knows where the keyboard is.
 */
export function currentSurface(rest: SnapshotRest | null, viewInUse: boolean): Surface | null {
  const context = remoteContext(rest);
  if (context) {
    const view = remoteView(context.session, rest?.workspace_view);
    return view?.tab?.id ? tabSurface(view.checkout, view.tab.id, context.device.id) : null;
  }
  const id = rest?.navigator?.focused_checkout_id;
  const checkout = localCheckouts(rest).find((row) => row.id === id);
  if (!checkout) return null;
  const layout = frontLayoutOf(rest, checkout);
  const display = viewInUse && layout ? activeDisplay(layout) : null;
  if (display) return displaySurface(checkout, display.display, localDeviceId(rest));
  return checkout.active_tab_id ? tabSurface(checkout, checkout.active_tab_id, localDeviceId(rest)) : null;
}

/**
 * What the operator is on now: a Home Overview or a Project's Overview while
 * the page shows one, else the Workspace surface in use (`currentSurface`) on
 * the device in front.
 */
export function currentEntry(screen: Screen | null, rest: SnapshotRest | null, viewInUse: boolean): RecentEntry | null {
  if (screen?.kind === "main") return screenVisit(screen, frontDeviceId(rest));
  return currentSurface(rest, viewInUse);
}

/**
 * What the core has in front: the device, checkout, visible tab and the pane
 * focused in it, and the front Workspace's panel state and active display.
 * Only a change here, or the keyboard moving between a checkout's areas, is a
 * visit; an agent's status or a sidebar click that has not landed yet moves
 * none of it.
 */
export function focusSignature(rest: SnapshotRest | null): string {
  const navigator = rest?.navigator;
  const checkout = localCheckouts(rest).find((row) => row.id === navigator?.focused_checkout_id);
  const layout = checkout ? frontLayoutOf(rest, checkout) : null;
  const display = layout ? activeDisplay(layout)?.display.id : null;
  const device = remoteView(remoteContext(rest)?.session ?? null, rest?.workspace_view);
  return [navigator?.focused_device_id, checkout?.id, checkout?.active_tab_id, rest?.focused?.pane_id, device?.checkout.id, device?.tab?.id, device?.focusedPaneId, workspaceViewOf(rest)?.tool, display].join("\u0000");
}

/**
 * Every entry the session still holds, in the navigator's order: each
 * checkout's Herdr tabs, a connected device's after this machine's, the front Workspace's displays as the snapshot
 * carries them, and the displays remembered for a checkout not in front,
 * which the snapshot does not carry and which the core checks on commit;
 * then the remembered screens that still exist.
 */
export function availableEntries(rest: SnapshotRest | null, remembered: readonly RecentEntry[]): RecentEntry[] {
  const entries: RecentEntry[] = [];
  for (const { deviceId, checkout } of allCheckouts(rest)) {
    for (const tab of checkout.tabs) if (tab.id) entries.push(tabSurface(checkout, tab.id, deviceId));
    // A device's View displays are not in the snapshot, so only its Herdr tabs are entries.
    if (deviceId !== localDeviceId(rest)) continue;
    const layout = frontLayoutOf(rest, checkout);
    if (layout) {
      for (const area of areasOf(layout.root)) for (const display of area.displays) entries.push(displaySurface(checkout, display, deviceId));
    } else {
      entries.push(...remembered.filter((entry) => !isScreenVisit(entry) && entry.kind !== AGENT_SURFACE && entry.checkoutId === checkout.id));
    }
  }
  entries.push(...remembered.filter((entry) => isScreenVisit(entry) && screenExists(rest, entry.screen)));
  return entries;
}

let entries: RecentEntry[] = [];
let projects: string[] = [];
/** Where a pane visit is reported: the core, which keeps the order (see `configurePaneVisits`). */
let reportVisit: (paneId: string) => void = () => {};
/** The pane last reported, so a repeated focus in the same pane is not reported again before the core echoes it. */
let reported: string | null = null;
/**
 * The core's order when `reported` was sent, until that order moves. While it
 * has not moved the core has not echoed the visit, and the order read back
 * puts the reported pane first, so a cycle opened in that gap starts where
 * the keyboard last was; once it moves, the core's order is the order.
 */
let reportedOver: string | null = null;
const orderOf = (rest: SnapshotRest | null) => (rest?.ui_state?.recent_pane_ids ?? []).join("\n");
/** The reported pane while the core's order has not moved since; the first look that sees it move ends the wait for good. */
function unechoedVisit(rest: SnapshotRest | null): string | null {
  if (reportedOver !== null && orderOf(rest) !== reportedOver) reportedOver = null;
  return reportedOver === null ? null : reported;
}
/** The entry a Recent Panels or Recent Projects commit asked for, until the page shows it (see `expectSurface`). */
let expected: string | null = null;
/** The pane an Agent pane commit asked for, until the keyboard is in it (see `expectPane`). */
let expectedPane: string | null = null;

/**
 * Brings the order up to date with what the session holds and moves the
 * entry in use to its front. Entries that are gone leave, surfaces never
 * used join at the end in the navigator's order, and a screen joins when it
 * is first shown, so the order is bounded by what exists: every surface, one
 * Overview per Project in the catalog, and All projects.
 */
export function observeEntries(rest: SnapshotRest | null, current: RecentEntry | null) {
  const available = availableEntries(rest, entries);
  const byKey = new Map(available.map((entry) => [entry.key, entry]));
  const known = new Set<string>();
  const next: RecentEntry[] = [];
  for (const entry of entries) {
    const fresh = byKey.get(entry.key);
    if (fresh && !known.has(entry.key)) {
      known.add(entry.key);
      next.push(fresh);
    }
  }
  for (const entry of available) {
    if (known.has(entry.key)) continue;
    known.add(entry.key);
    next.push(entry);
  }
  entries = next;
  // A commit's own frames pass through the target's other surface (the
  // checkout arrives before the keyboard does, an Overview stays on screen
  // until the Workspace is in front); none of them is a visit.
  if (expected && current?.key !== expected) return;
  expected = null;
  if (!current) return;
  const visited = isScreenVisit(current) ? (screenExists(rest, current.screen) ? current : null) : byKey.get(current.key);
  if (!visited) return;
  entries = [visited, ...entries.filter((entry) => entry.key !== visited.key)];
}

/** Sends each pane visit to the core, which moves the pane to the front of `ui_state.recent_pane_ids`. */
export function configurePaneVisits(report: (paneId: string) => void) {
  reportVisit = report;
  reported = null;
  reportedOver = null;
}

/**
 * Reports the pane the keyboard is in as a visit. A pane joins the order only
 * once the keyboard has been in it, and only a pane a listed tab holds is
 * reported. A commit's own frames (the checkout arriving before its pane is
 * focused) are not visits, as for `observeEntries`.
 */
export function observePane(rest: SnapshotRest | null, current: string | null) {
  unechoedVisit(rest);
  if (expectedPane && current !== expectedPane) return;
  expectedPane = null;
  if (!current || !allPanes(rest).some((row) => row.pane.id === current)) return;
  // Another window's visit moves the core's head, so the same pane is reported again once it is not first.
  if (current === reported && recentPanes(rest)[0] === current) return;
  reported = current;
  reportedOver = orderOf(rest);
  reportVisit(current);
}

/** Marks the pane a commit is bringing forward: pane visits are not recorded until the keyboard is in it or the operator acts. */
export function expectPane(paneId: string | null) {
  expectedPane = paneId;
}

/** Pane ids, the one the keyboard was in last first; remote ids are scoped to their device, so one id names one pane. */
export function recentPanes(rest: SnapshotRest | null): readonly string[] {
  const held = rest?.ui_state?.recent_pane_ids ?? [];
  const pending = unechoedVisit(rest);
  return pending === null ? held : [pending, ...held.filter((id) => id !== pending)];
}

/** The project in front, first in the Recent Projects order. */
export function observeProject(workspaceId: string | null | undefined) {
  if (!workspaceId || projects[0] === workspaceId) return;
  projects = [workspaceId, ...projects.filter((id) => id !== workspaceId)];
}

/** Marks the entry a commit is bringing forward: visits are not recorded until the page shows it or the operator acts. */
export function expectSurface(key: string | null) {
  expected = key;
}

export function recentEntries(): readonly RecentEntry[] {
  return entries;
}

/** The project's Workspace surface used last, which Recent Projects restores. */
export function lastSurfaceOf(workspaceId: string): Surface | null {
  for (const entry of entries) if (!isScreenVisit(entry) && entry.workspaceId === workspaceId) return entry;
  return null;
}

/** `existing` projects in recent order, unused ones after in their own order. */
export function recentProjectOrder(existing: readonly string[]): string[] {
  const seen = projects.filter((id) => existing.includes(id));
  return [...seen, ...existing.filter((id) => !seen.includes(id))];
}

/** Back to an empty order: a test, and a practice chapter that starts over on the same page. */
export function resetRecent() {
  entries = [];
  projects = [];
  reportVisit = () => {};
  reported = null;
  reportedOver = null;
  expected = null;
  expectedPane = null;
}

// --- rows -----------------------------------------------------------------

/** What committing a switcher row brings forward. */
export type CycleTarget =
  /** A Workspace surface, in its own checkout, on its own device. */
  | { kind: "surface"; surface: Surface }
  /** One terminal pane, with the tab it is in, which comes forward with it. */
  | { kind: "pane"; paneId: string; surface: Surface }
  /** A device's Home Overview or a Project's Overview, which the page shows itself once the device is in front. */
  | { kind: "screen"; screen: PageScreen; deviceId: string }
  /** A project with no surface to restore, on its checkout and device. */
  | { kind: "checkout"; deviceId: string; workspaceId: string; checkoutId: string };

/** What a switcher row says under its title; the overlay words it in the interface language. */
export type CycleDetail =
  /** Where a surface is and what it is; a null place is the device's Home. */
  | { kind: "surface"; place: string | null; surface: SurfaceKind }
  | { kind: "projects"; count: number }
  | { kind: "text"; text: string };

/** What a switcher row draws and what committing it names. */
export type CycleItem = {
  key: string;
  /** The row's name; empty on a device's Home page (kind `main`), which the overlay names in the interface language. */
  title: string;
  detail: CycleDetail;
  kind: SurfaceKind | "project" | PageScreen["kind"];
  /** The device's name when the row is not on the device in front, drawn as a chip (PRD home-device-rail D-16); null otherwise. */
  chip: DeviceChipView | null;
  /** The one agent a tab holds, drawn with its status mark and its own mark. */
  agent: import("./snapshot").TabAgent | null;
  target: CycleTarget;
};

/** "project · checkout", collapsed to the checkout when both share a name. */
export function placeLabel(workspace: Pick<Workspace, "label">, checkout: Pick<Checkout, "label">): string {
  return workspace.label === checkout.label ? checkout.label : `${workspace.label} · ${checkout.label}`;
}

/** A device's chip: its name, and whether it is this machine (which wears the laptop glyph, not the server's). */
export type DeviceChipView = { label: string; local: boolean };

/** The chip for a row on `deviceId`, or null while that is the device in front. */
export function deviceChip(rest: SnapshotRest | null, deviceId: string): DeviceChipView | null {
  if (deviceId === frontDeviceId(rest)) return null;
  const devices = rest?.navigator?.devices;
  return { label: machineName(devices, deviceId, translate), local: deviceId === windowDeviceId(devices) };
}

function findCheckout(rest: SnapshotRest | null, checkoutId: string) {
  const row = allCheckouts(rest).find((candidate) => candidate.checkout.id === checkoutId);
  return row ?? null;
}

/** The agents `deviceId` reports, which a tab's one agent is looked up in. */
function agentsOf(rest: SnapshotRest | null, deviceId: string): AgentRow[] {
  if (deviceId === localDeviceId(rest)) return rest?.navigator?.agents ?? [];
  return rest?.status?.remote?.find((status) => status.target_id === deviceId)?.session?.agents ?? [];
}

/** A terminal pane with where it is. */
type DevicePane = DeviceCheckout & { tab: Tab; pane: PaneRow };

/** Every terminal pane of every checkout `allCheckouts` holds. */
function allPanes(rest: SnapshotRest | null): DevicePane[] {
  return allCheckouts(rest).flatMap((row) => row.checkout.tabs.flatMap((tab) => tab.panes.map((pane) => ({ ...row, tab, pane }))));
}

export function paneKey(paneId: string): string {
  return `pane\u0000${paneId}`;
}

/**
 * An Agent pane row, for a pane that runs an agent: called by that agent,
 * with the place and the device chip as Recent Panels reads them. A pane with
 * no agent is no row, though its visits stay recorded for when one starts.
 */
export function paneItem(rest: SnapshotRest | null, paneId: string): CycleItem | null {
  const place = allPanes(rest).find((row) => row.pane.id === paneId);
  const agent = place ? agentsOf(rest, place.deviceId).find((row) => row.pane_id === paneId) : undefined;
  if (!place?.tab.id || !agent) return null;
  return {
    key: paneKey(paneId),
    title: agent.identity_label,
    detail: { kind: "surface", place: place.workspace.is_home ? null : placeLabel(place.workspace, place.checkout), surface: AGENT_SURFACE },
    kind: AGENT_SURFACE,
    chip: deviceChip(rest, place.deviceId),
    agent,
    target: { kind: "pane", paneId, surface: tabSurface(place.checkout, place.tab.id, place.deviceId) },
  };
}

/**
 * A Recent Panels row: a tab holding exactly one agent pane is called by
 * that agent and carries its status mark; any other tab keeps its Herdr
 * label, and a display its own name. A Project's Overview is called by its
 * Project and is gone with it; a device's Home Overview reads as the sidebar's
 * Home row does, `Home` over the project count.
 */
export function panelItem(rest: SnapshotRest | null, entry: RecentEntry): CycleItem | null {
  if (isScreenVisit(entry)) return screenItem(rest, entry);
  const surface = entry;
  const place = findCheckout(rest, surface.checkoutId);
  if (!place) return null;
  const detail: CycleDetail = { kind: "surface", place: place.workspace.is_home ? null : placeLabel(place.workspace, place.checkout), surface: surface.kind };
  const base = { key: surface.key, kind: surface.kind, chip: deviceChip(rest, place.deviceId), target: { kind: "surface", surface } as const, detail };
  if (surface.kind !== AGENT_SURFACE) return { ...base, title: surface.label, agent: null };
  const tab = place.checkout.tabs.find((row) => row.id === surface.id);
  if (!tab) return null;
  const paneIds = new Set(tab.panes.map((pane) => pane.id));
  const agents = agentsOf(rest, place.deviceId).filter((agent) => paneIds.has(agent.pane_id));
  const agent = agents.length === 1 ? agents[0]! : null;
  return { ...base, title: agent?.identity_label ?? tab.label ?? surface.id, agent };
}

function screenItem(rest: SnapshotRest | null, visit: ScreenVisit): CycleItem | null {
  const { screen } = visit;
  {
    const deviceId = screen.deviceId ?? frontDeviceId(rest);
    const count = allProjectsCount(rest, deviceId);
    return {
      key: visit.key,
      title: "",
      detail: { kind: "projects", count },
      kind: screen.kind,
      chip: deviceChip(rest, deviceId),
      agent: null,
      target: { kind: "screen", screen, deviceId },
    };
  }

}

/** A Recent Projects row: the project, with the surface and checkout it would come back on. */
export function projectItem(workspace: Workspace, rest: SnapshotRest | null): CycleItem | null {
  const last = lastSurfaceOf(workspace.id);
  const lastItem = last ? panelItem(rest, last) : null;
  const restored = lastItem ? last : null;
  const checkout = (restored && workspace.checkouts.find((row) => row.id === restored.checkoutId)) ?? workspace.checkouts[0];
  if (!checkout) return null;
  const detail: CycleDetail = { kind: "text", text: lastItem ? `${lastItem.title} · ${checkout.label}` : checkout.label };
  const target: CycleTarget = restored ? { kind: "surface", surface: restored } : { kind: "checkout", deviceId: workspace.device_id, workspaceId: workspace.id, checkoutId: checkout.id };
  return { key: workspace.id, title: workspace.label, detail, kind: "project", chip: deviceChip(rest, workspace.device_id), agent: null, target };
}

// --- the held cycle ---------------------------------------------------------

/** How many rows either switcher shows around its highlight. */
export const CYCLE_WINDOW = 9;

/** The rows around the highlight, at most `CYCLE_WINDOW`, the highlight among them. */
export function visibleWindow<T>(items: readonly T[], index: number): { start: number; rows: T[] } {
  const count = Math.min(CYCLE_WINDOW, items.length);
  const start = Math.min(Math.max(0, index - Math.floor(count / 2)), items.length - count);
  return { start, rows: items.slice(start, start + count) };
}

/**
 * The held cycle after the session changed under it: rows that are gone
 * leave without reordering the rest, a highlight that went moves to the next
 * surviving row, and with none surviving the cycle ends.
 */
export function reconcileCycle<T extends { key: string }>(items: readonly T[], index: number, alive: (item: T) => boolean): { items: T[]; index: number } | null {
  if (items.length === 0) return null;
  const successor = Array.from({ length: items.length }, (_, step) => items[(index + step) % items.length]!).find(alive);
  if (!successor) return null;
  const kept = items.filter(alive);
  return { items: kept, index: kept.indexOf(successor) };
}
