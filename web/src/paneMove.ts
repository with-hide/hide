import type { AgentLayout } from "./agentLayout";
import { areasOf } from "./areaLayout";
import { agentClosing } from "./close";
import type { AsyncOperation, Checkout, PaneRow, Tab, TerminalPane } from "./snapshot";

export type PaneMoveSource = { pane_id: string; tab_id: string; area_id: string };
export type PaneMoveDestination =
  | ({ type: "pane"; edge: "left" | "right" | "up" | "down" } & PaneMoveSource)
  | ({ type: "tab" } & PaneMoveSource)
  | { type: "new_tab"; area_id: string };
export type PaneMoveIntent = { request_id: string; workspace: { device_id: string; checkout_path: string }; group_id: string | null; source: PaneMoveSource; destination: PaneMoveDestination };
export type MoveReason = "unavailable" | "pending" | "sleeping" | "starting" | "disconnected" | "stale" | "capacity";
export const MOVE_EDGES = ["left", "right", "up", "down"] as const;
const LIVE = new Set(["connected", "controlling", "observing", "idle"]);
export function movingTab(operations: readonly AsyncOperation[] | undefined, tabId: string): boolean {
  return (operations ?? []).some(op => op.kind === "pane.move" && op.phase !== "completed" && op.phase !== "failed" && (op.scope_id === tabId || op.related_scope_ids?.includes(tabId)));
}
export function movePaneReason(pane: PaneRow, tabId: string, transports: readonly TerminalPane[], operations: readonly AsyncOperation[] | undefined): MoveReason | null {
  if (movingTab(operations, tabId) || agentClosing(operations, pane.id)) return "pending";
  if (pane.sleep) return "sleeping";
  if (pane.failed_terminal_id) return "unavailable";
  const transport = transports.find(row => row.pane_id === pane.id);
  // Unattached background tabs have no transport; their live topology remains a valid destination.
  if (transport && !LIVE.has(transport.transport_state) && transport.transport_state !== "released") return transport.transport_state === "closing" ? "pending" : "starting";
  return null;
}
export function paneLocation(checkout: Checkout, layout: AgentLayout, paneId: string): PaneMoveSource | null {
  const tab = checkout.tabs.find(tab => tab.panes.some(pane => pane.id === paneId));
  if (!tab?.id) return null;
  const area = areasOf(layout.root).find(area => area.displays.some(item => item.id === tab.id) || layout.canvases[area.id] === tab.id);
  return area ? { pane_id: paneId, tab_id: tab.id, area_id: area.id } : null;
}
export function sameMoveTarget(a: PaneMoveDestination | null, b: PaneMoveDestination | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
export function paneEdge(rect: { left: number; top: number; width: number; height: number }, x: number, y: number): typeof MOVE_EDGES[number] | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  const px = (x - rect.left) / rect.width, py = (y - rect.top) / rect.height;
  if (px < 0 || px > 1 || py < 0 || py > 1) return null;
  const edges = [{ edge: "left", distance: px }, { edge: "right", distance: 1 - px }, { edge: "up", distance: py }, { edge: "down", distance: 1 - py }] as const;
  const nearest = [...edges].sort((a, b) => a.distance - b.distance)[0]!;
  return nearest.distance <= 0.25 ? nearest.edge : null;
}
export function selectedMovePane(tab: Tab): string | null {
  const selected = tab.sidebar?.focused_pane_id;
  return selected && tab.panes.some(pane => pane.id === selected) ? selected : tab.panes.length === 1 ? tab.panes[0]!.id : null;
}

/** Only the first header row's grip, title and unused space initiate a gesture. */
export function paneMoveStartTarget(header: HTMLElement, target: EventTarget | null): boolean {
  if (!(target instanceof Element) || !header.contains(target)) return false;
  if (target.closest("[data-pane-move-excluded], [data-pane-status-mark]")) return false;
  // The record button and title wrappers also own padding/gaps with no content.
  // Those gaps look like the rest of the unused header, so they share its gesture.
  return target === header || !!target.closest("[data-pane-move-grip], [data-pane-move-title], [data-pane-move-blank], [data-pane-record]");
}
