// The focused-area cycle. In a View area it narrows the page MRU to that one
// drawn area; everywhere else it walks every place the operator has been:
// the agent panes the keyboard has been in across devices, projects and
// checkouts, the missions and automations whose detail was in front, and
// Desk (issue 301). No committed selection lives here.
import { areaFrame } from "./areaFrames";
import { areasOf, findArea } from "./areaLayout";
import { displaySurface, panelItem, placeItem, placeKey, recentEntries, recentPlaces, tabSurface, type CycleItem, type Surface } from "./recent";
import { frontCheckout, type RecentPlace, type SnapshotRest } from "./snapshot";
import { focusedPaneOf } from "./store";
import { deskOnScreen } from "./pairView";
import { workspaceViewOf } from "./workspace";
import { useUiStore, type Cycle } from "./ui";
import { keyboardOwner, type KeyboardOwner } from "./viewFocus";

export type CycleScope = { deviceId: string; checkoutId: string; path: string; kind: "agent" | "view"; areaId: string };

/** The owner must belong to the front checkout and an area actually drawn. */
export function focusedCycleScope(rest: SnapshotRest | null, owner: KeyboardOwner = keyboardOwner()): CycleScope | null {
  const checkout = frontCheckout(rest);
  if (!checkout || owner.kind === "none" || owner.kind === "tool" || owner.workspace !== checkout.id || (useUiStore.getState().screen?.kind !== "workspace" || useUiStore.getState().overviewOpen)) return null;
  const kind = owner.kind === "view" ? "view" : "agent";
  const frame = areaFrame(kind);
  if (!frame || frame.workspace.path !== checkout.path) return null;
  let areaId = owner.kind === "view" || owner.kind === "agent" ? owner.areaId : undefined;
  if (owner.kind === "pane") {
    areaId = areasOf(frame.layout.root).find((area) => {
      // A delegated canvas is not a member of the area's normal tab strip.
      const shown = kind === "agent" ? areaFrame("agent")?.layout.canvases[area.id] ?? area.active : area.active;
      return area.displays.some((item) => item.id === shown) && checkout.tabs.find((tab) => tab.id === shown)?.panes.some((pane) => pane.id === owner.paneId);
    })?.id;
  }
  if (!areaId || !frame.geometry.areas.some((area) => area.id === areaId)) return null;
  return { deviceId: frame.workspace.device_id, checkoutId: checkout.id, path: checkout.path, kind, areaId };
}

/** Null means the originating scope disappeared, never permission to widen it. */
export function scopedSurfaces(rest: SnapshotRest | null, scope: CycleScope): { surfaces: Surface[]; active: string | null } | null {
  const checkout = frontCheckout(rest);
  const frame = areaFrame(scope.kind);
  if (!checkout || checkout.id !== scope.checkoutId || checkout.path !== scope.path || !frame || frame.workspace.device_id !== scope.deviceId || frame.workspace.path !== scope.path || (useUiStore.getState().screen?.kind !== "workspace" || useUiStore.getState().overviewOpen)) return null;
  const view = workspaceViewOf(rest);
  if (!view || view.device_id !== scope.deviceId || view.path !== scope.path || (scope.kind === "view" && !deskOnScreen(view))) return null;
  const layout = scope.kind === "view" ? view.layout : (view.agent_layout ?? frame.layout);
  if (!layout) return null;
  const area = findArea(layout.root, scope.areaId);
  if (!area || !frame.geometry.areas.some((box) => box.id === scope.areaId)) return null;
  const surfaces = scope.kind === "agent"
    ? area.displays.filter((item) => checkout.tabs.some((tab) => tab.id === item.id)).map((item) => tabSurface(checkout, item.id, scope.deviceId))
    : area.displays.map((item) => displaySurface(checkout, item as import("./snapshot").ViewDisplaySnapshot, scope.deviceId));
  return { surfaces, active: area.active };
}

export function focusedSurface(rest: SnapshotRest | null, scope: CycleScope | null): Surface | null {
  const membership = scope && scopedSurfaces(rest, scope);
  return membership ? membership.surfaces.find((surface) => surface.id === membership.active) ?? null : null;
}

/** The View area cycle: that area's tabs, most recent first, then the ones never visited. */
export function areaCycle(rest: SnapshotRest | null, owner?: KeyboardOwner): Cycle | null {
  const scope = focusedCycleScope(rest, owner);
  const membership = scope?.kind === "view" ? scopedSurfaces(rest, scope) : null;
  if (!scope || !membership || membership.surfaces.length < 2) return null;
  const current = membership.surfaces.find((surface) => surface.id === membership.active);
  if (!current) return null;
  const known = new Map(membership.surfaces.map((surface) => [surface.key, surface]));
  const order = [current, ...recentEntries().flatMap((entry) => {
    const surface = known.get(entry.key);
    return surface && surface.key !== current.key ? [surface] : [];
  })];
  const ordered = new Set(order.map((surface) => surface.key));
  order.push(...membership.surfaces.filter((surface) => !ordered.has(surface.key)));
  const items = order.map((surface) => panelItem(rest, surface)).filter((item): item is CycleItem => item !== null);
  return items.length > 1 ? { kind: "area", scope, originKey: current.key, items, index: 0 } : null;
}

/**
 * Where the keyboard is while the front Workspace's Agent area holds it: the
 * pane it is in, or for a tab bar the pane the core focused in the tab that
 * area shows. Null when the keyboard is anywhere else. Only a tab an Agent
 * area draws counts, its normal one or a delegated child's canvas, so a pane
 * whose terminal went away with its tab is not where the keyboard is.
 */
export function agentOrigin(rest: SnapshotRest | null, owner: KeyboardOwner = keyboardOwner()): { paneId: string | null } | null {
  const checkout = frontCheckout(rest);
  const frame = areaFrame("agent");
  if (!rest || !checkout || !frame || frame.workspace.path !== checkout.path || (owner.kind !== "pane" && owner.kind !== "agent") || owner.workspace !== checkout.id || (useUiStore.getState().screen?.kind !== "workspace" || useUiStore.getState().overviewOpen)) return null;
  const drawn = new Set(areasOf(frame.layout.root).map((area) => frame.layout.canvases[area.id] ?? area.active));
  const holds = (paneId: string | null | undefined, tabId?: string | null) =>
    paneId && checkout.tabs.some((tab) => tab.id !== null && drawn.has(tab.id) && (tabId === undefined || tab.id === tabId) && tab.panes.some((pane) => pane.id === paneId)) ? paneId : null;
  if (owner.kind === "pane") return holds(owner.paneId) ? { paneId: owner.paneId } : null;
  const tabId = focusedSurface(rest, focusedCycleScope(rest, owner))?.id ?? checkout.active_tab_id;
  return { paneId: holds(focusedPaneOf(rest), tabId) };
}

/**
 * The place the operator is on now, as the recent place order names it: the
 * pane the keyboard is in while the front Workspace's Agent area holds it,
 * the mission or automation whose detail the screen shows, or Desk's room.
 * Null anywhere else: a list (the Overview, the mission or automation table,
 * an automation's form), a View area, the Overview over any screen.
 */
export function placeOrigin(rest: SnapshotRest | null, owner: KeyboardOwner = keyboardOwner()): RecentPlace | null {
  const ui = useUiStore.getState();
  if (ui.overviewOpen) return null;
  switch (ui.screen?.kind) {
    case "workspace": {
      const paneId = agentOrigin(rest, owner)?.paneId;
      return paneId ? { kind: "pane", pane_id: paneId } : null;
    }
    case "missions":
      return ui.missions.selected !== null ? { kind: "mission", mission_id: ui.missions.selected } : null;
    case "automations":
      return ui.automations.selected !== null && ui.automations.form === null ? { kind: "automation", automation_id: ui.automations.selected } : null;
    case "desk":
      return { kind: "desk" };
    default:
      return null;
  }
}

/**
 * The recent place cycle: the place in use, then every place the operator
 * has been, most recent first: an agent pane on any device, project or
 * checkout, a mission, an automation, Desk. When the place in use is no row
 * (a pane running no agent) or the operator is on no place (a list, the
 * sidebar, Settings), the first chord lands on the most recent place, so the
 * cycle starts before it. Only View ownership on the Workspace excludes this
 * list; there the View area cycle walks that area's tabs.
 */
export function placeCycle(rest: SnapshotRest | null, owner: KeyboardOwner = keyboardOwner()): Cycle | null {
  if (owner.kind === "view" && useUiStore.getState().screen?.kind === "workspace" && !useUiStore.getState().overviewOpen) return null;
  const origin = placeOrigin(rest, owner);
  const originKey = origin ? placeKey(origin) : undefined;
  const places = [...(origin ? [origin] : []), ...recentPlaces(rest).filter((place) => placeKey(place) !== originKey)];
  const items = places.map((place) => placeItem(rest, place)).filter((item): item is CycleItem => item !== null);
  const atOrigin = items[0] !== undefined && items[0].key === originKey;
  if (items.length < (atOrigin ? 2 : 1)) return null;
  return { kind: "places", originKey, items, index: atOrigin ? 0 : -1 };
}
