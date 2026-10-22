// The focused-area cycle. In a View area it narrows the page MRU to that one
// drawn area; everywhere else it walks every agent pane the keyboard has
// been in, across devices, projects and checkouts (issue 301).
// No committed selection lives here.
import { areaFrame } from "./areaFrames";
import { areasOf, findArea } from "./areaLayout";
import { displaySurface, paneItem, paneKey, panelItem, recentEntries, recentPanes, tabSurface, type CycleItem, type Surface } from "./recent";
import { frontCheckout, type SnapshotRest } from "./snapshot";
import { focusedPaneOf } from "./store";
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
  if (!view || view.device_id !== scope.deviceId || view.path !== scope.path || (scope.kind === "view" && !(view.open && view.documents))) return null;
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
 * The Agent pane cycle: the pane in use, then every agent pane the keyboard
 * has been in, most recent first, on any device, project or checkout. When
 * the pane in use runs no agent, or there is none, the first chord lands on
 * the most recent agent pane, so the cycle starts before it. Only View
 * ownership on the Workspace excludes this list; the sidebar, tools and
 * other screens do not need a drawn Agent area, and do not supply a pane visit.
 */
export function agentCycle(rest: SnapshotRest | null, owner: KeyboardOwner = keyboardOwner()): Cycle | null {
  if (owner.kind === "view" && useUiStore.getState().screen?.kind === "workspace" && !useUiStore.getState().overviewOpen) return null;
  const origin = agentOrigin(rest, owner) ?? { paneId: null };
  const ids = [...(origin.paneId ? [origin.paneId] : []), ...recentPanes(rest).filter((id) => id !== origin.paneId)];
  const items = ids.map((id) => paneItem(rest, id)).filter((item): item is CycleItem => item !== null);
  const originKey = origin.paneId ? paneKey(origin.paneId) : undefined;
  const atOrigin = items[0] !== undefined && items[0].key === originKey;
  if (items.length < (atOrigin ? 2 : 1)) return null;
  return { kind: "agents", originKey, items, index: atOrigin ? 0 : -1 };
}
