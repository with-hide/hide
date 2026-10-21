import { PlusIcon } from "lucide-react";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Actions } from "./actions";
import type { AgentLayout } from "./agentLayout";
import { areasOf } from "./areaLayout";
import type { MenuEntry } from "./components/entry-menu";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { MOVE_EDGES, movePaneReason, movingTab, paneEdge, paneLocation, sameMoveTarget, selectedMovePane, type MoveReason, type PaneMoveDestination, type PaneMoveIntent, type PaneMoveSource } from "./paneMove";
import type { Checkout, LayoutNode, PaneRow } from "./snapshot";
import { deviceConnected } from "./devices";
import { agentEntries } from "./workspace";
import { holdShellDrag } from "./shellDrag";
import { useShellStore } from "./store";
import { mountTerminalPreview } from "./terminals";

type Target = { destination: PaneMoveDestination; rect: DOMRect; stamp: string };
type Gesture = { source: PaneMoveSource; stamp: string; pointerId: number; origin: { x: number; y: number }; dragging: boolean; target: Target | null };
type Preview = { areaId: string; tabId: string; requestId: string };
type MoveContext = {
  active: boolean;
  press: (paneId: string, event: React.PointerEvent<HTMLElement>) => void;
  mutationBlocked: (paneId: string) => boolean;
  menu: (paneId: string) => MenuEntry[];
  select: (id: string) => boolean;
  preview: Preview | null;
  checkout: Checkout;
  layout: AgentLayout;
};
const Context = createContext<MoveContext | null>(null);
export const usePaneMove = () => useContext(Context);

// A cancelled drag may release after its provider unmounts. Keep one bounded
// suppression until that release/click or the next deliberate pointer press.
let clearSuppression = () => {};
function suppressDragClick() {
  clearSuppression();
  let timer: ReturnType<typeof setTimeout>;
  const clear = () => {
    window.removeEventListener("click", swallow, true);
    window.removeEventListener("pointerdown", clear, true);
    window.removeEventListener("pointerup", released, true);
    clearTimeout(timer);
  };
  const swallow = (event: MouseEvent) => { event.preventDefault(); event.stopImmediatePropagation(); clear(); };
  const released = () => { clearTimeout(timer); timer = setTimeout(clear, 0); };
  window.addEventListener("click", swallow, true);
  window.addEventListener("pointerdown", clear, true);
  window.addEventListener("pointerup", released, true);
  timer = setTimeout(clear, 30_000);
  clearSuppression = clear;
}

/** One gesture and hover timer, no persistent layout or selection. */
export function PaneMoveProvider({ checkout, layout, deviceId, actions, children }: { checkout: Checkout; layout: AgentLayout; deviceId: string; actions: Actions; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const root = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(false);
  const [target, setTarget] = useState<Target | null>(null);
  const [reason, setReason] = useState<MoveReason | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [request, setRequest] = useState<PaneMoveIntent | null>(null);
  const [unconfirmed, setUnconfirmed] = useState(false);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const gesture = useRef<Gesture | null>(null);
  const hover = useRef<{ key: string; timer: ReturnType<typeof setTimeout> } | null>(null);
  const cancelGesture = useRef<() => void>(() => {});
  const latest = useRef({ checkout, layout, deviceId });
  latest.current = { checkout, layout, deviceId };
  const connection = useShellStore(s => s.connection);
  const operations = useShellStore(s => s.rest?.status?.async_operations);
  const trackedOperation = operations?.find(op => op.kind === "pane.move" && op.id === request?.request_id);
  const unresolvedOperation = operations?.find(op => op.kind === "pane.move" && op.phase !== "completed" && op.phase !== "failed" && checkout.tabs.some(tab => tab.id !== null && (op.scope_id === tab.id || op.related_scope_ids?.includes(tab.id))));
  // Published reservations survive a scope change or renderer reconnect.
  const operation = !request || trackedOperation?.phase === "completed" || trackedOperation?.phase === "failed" && unresolvedOperation?.phase === "unknown" ? unresolvedOperation : trackedOperation;
  const held = request !== null && (!trackedOperation || (trackedOperation.phase !== "completed" && trackedOperation.phase !== "failed"));
  useEffect(() => {
    if (!request || trackedOperation) { setUnconfirmed(false); return; }
    const timer = setTimeout(() => setUnconfirmed(true), 15_000);
    return () => clearTimeout(timer);
  }, [request, trackedOperation]);
  const selectedOperation = operation?.id === dismissed ? undefined : operation;
  const clearHover = () => { if (hover.current) clearTimeout(hover.current.timer); hover.current = null; };
  const stamp = (tabId: string) => {
    const state = useShellStore.getState();
    return JSON.stringify([latest.current.checkout.tabs.find(tab => tab.id === tabId)?.panes.map(pane => pane.id), state.rest?.pane_layouts?.find(row => row.tab_id === tabId) ?? state.rest?.status?.remote?.find(row => row.target_id === latest.current.deviceId)?.session?.pane_layouts.find(row => row.tab_id === tabId)]);
  };
  const unavailable = (source: PaneMoveSource): MoveReason | null => {
    const state = useShellStore.getState();
    if (state.connection !== "live") return "disconnected";
    if (!deviceConnected(state.rest, latest.current.deviceId)) return "disconnected";
    if (latest.current.layout.drafts?.includes(source.tab_id) || latest.current.layout.start_receipts?.some(receipt => receipt.live_tab_id === source.tab_id && receipt.state !== "ready")) return "starting";
    const pane = latest.current.checkout.tabs.find(tab => tab.id === source.tab_id)?.panes.find(pane => pane.id === source.pane_id);
    const location = pane && paneLocation(latest.current.checkout, latest.current.layout, pane.id);
    if (!location || location.tab_id !== source.tab_id || location.area_id !== source.area_id) return "stale";
    if (held && request && [request.source.tab_id, "tab_id" in request.destination ? request.destination.tab_id : ""].includes(source.tab_id)) return "pending";
    return movePaneReason(pane, source.tab_id, state.rest?.terminal?.panes ?? [], state.rest?.status?.async_operations);
  };
  const destinationReason = (destination: PaneMoveDestination, source = gesture.current?.source): MoveReason | null => {
    if (!areasOf(latest.current.layout.root).some(area => area.id === destination.area_id)) return "stale";
    if (destination.type !== "new_tab") return unavailable(destination);
    const sole = latest.current.checkout.tabs.find(tab => tab.id === source?.tab_id)?.panes.length === 1;
    if (!sole && latest.current.layout.display_count >= latest.current.layout.limits.displays) return "capacity";
    return null;
  };
  const run = (source: PaneMoveSource, destination: PaneMoveDestination) => {
    const refused = unavailable(source) ?? destinationReason(destination, source);
    if (refused) { setReason(refused); return; }
    if (destination.type !== "new_tab" && destination.pane_id === source.pane_id) return;
    const intent: PaneMoveIntent = { request_id: crypto.randomUUID(), workspace: { device_id: latest.current.deviceId, checkout_path: latest.current.checkout.path }, group_id: latest.current.layout.group_id ?? null, source, destination };
    setRequest(intent); setUnconfirmed(false); setDismissed(null); setReason(null);
    actions.movePane(intent);
  };
  const resolve = (x: number, y: number): Target | null => {
    const source = gesture.current?.source;
    if (!source || !root.current) return null;
    const element = document.elementFromPoint(x, y);
    if (!(element instanceof Element) || !root.current.contains(element)) { clearHover(); setPreview(null); setReason(null); return null; }
    const area = element.closest<HTMLElement>("[data-agent-area-id]");
    if (preview && area?.dataset.agentAreaId !== preview.areaId) setPreview(null);
    const plus = element.closest<HTMLElement>("[data-agent-new-tab]");
    if (plus) {
      const destination: PaneMoveDestination = { type: "new_tab", area_id: plus.dataset.agentNewTab! };
      const why = destinationReason(destination); setReason(why);
      return why ? null : { destination, rect: plus.getBoundingClientRect(), stamp: "" };
    }
    const bar = element.closest<HTMLElement>("[data-agent-tab-bar]");
    const tabElement = bar && element.closest<HTMLElement>("[data-area-item]");
    if (bar && tabElement) {
      const tabId = tabElement.dataset.areaItem!;
      if (tabId === source.tab_id) { clearHover(); setPreview(null); setReason(null); return null; }
      const tab = latest.current.checkout.tabs.find(tab => tab.id === tabId);
      const paneId = tab && selectedMovePane(tab);
      const areaId = bar.dataset.agentTabBar!;
      if (!paneId) { setReason("starting"); clearHover(); return null; }
      const destination: PaneMoveDestination = { type: "tab", tab_id: tabId, pane_id: paneId, area_id: areaId };
      const why = destinationReason(destination); setReason(why);
      if (why) { clearHover(); return null; }
      const key = `${areaId}\0${tabId}`;
      if (hover.current?.key !== key) {
        clearHover();
        hover.current = { key, timer: setTimeout(() => { setPreview({ areaId, tabId, requestId: crypto.randomUUID() }); }, 450) };
      }
      return { destination, rect: tabElement.getBoundingClientRect(), stamp: stamp(tabId) };
    }
    clearHover();
    const paneElement = element.closest<HTMLElement>("[data-pane-move-preview-pane], [data-pane-view]");
    const paneId = paneElement?.dataset.paneMovePreviewPane ?? paneElement?.dataset.paneView;
    if (!paneElement || !paneId || paneId === source.pane_id) { setReason(null); return null; }
    const located = paneLocation(latest.current.checkout, latest.current.layout, paneId);
    if (!located) return null;
    const rect = paneElement.getBoundingClientRect();
    const edge = paneEdge(rect, x, y);
    if (!edge) { setReason(null); return null; }
    const destination: PaneMoveDestination = { type: "pane", ...located, edge };
    const why = destinationReason(destination); setReason(why);
    return why ? null : { destination, rect, stamp: stamp(located.tab_id) };
  };
  const runtime = useRef({ resolve, run, unavailable, stamp });
  runtime.current = { resolve, run, unavailable, stamp };
  useEffect(() => {
    let releaseDrag: (() => void) | null = null;
    const finish = (swallow: boolean) => {
      gesture.current = null; clearHover(); setActive(false); setTarget(null); setPreview(null);
      releaseDrag?.(); releaseDrag = null;
      if (swallow) suppressDragClick();
    };
    const move = (event: PointerEvent) => {
      const current = gesture.current;
      if (!current || event.pointerId !== current.pointerId) return;
      if (!current.dragging) {
        const threshold = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--size-tab-drag-activation"));
        if (Math.hypot(event.clientX - current.origin.x, event.clientY - current.origin.y) < threshold) return;
        current.dragging = true; setActive(true);
        releaseDrag = holdShellDrag("move", "data-agent-drag");
      }
      event.preventDefault();
      const refused = runtime.current.unavailable(current.source);
      if (refused || runtime.current.stamp(current.source.tab_id) !== current.stamp) { setReason(refused ?? "stale"); finish(true); return; }
      const next = runtime.current.resolve(event.clientX, event.clientY);
      const previous = current.target;
      current.target = next;
      if (!sameMoveTarget(previous?.destination ?? null, next?.destination ?? null) || previous?.stamp !== next?.stamp || previous?.rect.left !== next?.rect.left || previous?.rect.top !== next?.rect.top || previous?.rect.width !== next?.rect.width || previous?.rect.height !== next?.rect.height) setTarget(next);
    };
    const up = (event: PointerEvent) => {
      const current = gesture.current;
      if (!current || event.pointerId !== current.pointerId) return;
      const now = current.dragging ? runtime.current.resolve(event.clientX, event.clientY) : null;
      const lands = current.dragging && now && current.target && sameMoveTarget(now.destination, current.target.destination) && now.stamp === current.target.stamp && runtime.current.stamp(current.source.tab_id) === current.stamp;
      finish(current.dragging);
      if (lands) runtime.current.run(current.source, now.destination);
    };
    const cancel = () => { if (gesture.current) finish(gesture.current.dragging); };
    cancelGesture.current = cancel;
    const key = (event: KeyboardEvent) => { if (event.key === "Escape" && gesture.current) { event.preventDefault(); event.stopImmediatePropagation(); cancel(); } };
    window.addEventListener("pointermove", move, { passive: false }); window.addEventListener("pointerup", up, true); window.addEventListener("pointercancel", cancel); window.addEventListener("blur", cancel); window.addEventListener("keydown", key, true);
    return () => {
      cancelGesture.current = () => {};
      if (gesture.current?.dragging) suppressDragClick();
      clearHover(); releaseDrag?.();
      window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up, true); window.removeEventListener("pointercancel", cancel); window.removeEventListener("blur", cancel); window.removeEventListener("keydown", key, true);
    };
  }, []);
  useEffect(() => { if (connection !== "live") cancelGesture.current(); }, [connection]);
  const previewActions = useRef(actions);
  previewActions.current = actions;
  useEffect(() => {
    if (!preview) return;
    previewActions.current.previewPaneMove({ request_id: preview.requestId, workspace: { device_id: latest.current.deviceId, checkout_path: latest.current.checkout.path }, group_id: latest.current.layout.group_id ?? null, tab_id: preview.tabId });
    return () => previewActions.current.cancelPaneMovePreview(preview.requestId);
  }, [preview]);
  const menuTargets = new Map<string, { source: PaneMoveSource; destination: PaneMoveDestination }>();
  const menu = (paneId: string): MenuEntry[] => {
    const source = paneLocation(checkout, layout, paneId);
    if (!source) return [];
    const why = unavailable(source);
    const reasonText = (reason: MoveReason | null) => reason ? t(`menus.paneMove.${reason === "pending" ? "busy" : reason}`) : null;
    const entry = (destination: PaneMoveDestination, label: string): MenuEntry => {
      const id = `pane_move:${JSON.stringify(destination)}`;
      menuTargets.set(id, { source, destination });
      return { id, label, unavailable: reasonText(destinationReason(destination, source)) };
    };
    return [{ id: "pane_move", label: t("menus.paneMove.label"), icon: "right", unavailable: reasonText(why), separated: true, children: areasOf(layout.root).map((area, index) => ({
      id: `pane_move_area:${area.id}`, label: t("menus.paneMove.area", { index: index + 1 }), unavailable: null,
      children: [entry({ type: "new_tab", area_id: area.id }, t("menus.paneMove.newTab")), ...[...area.displays, ...(layout.canvases[area.id] && !area.displays.some(item => item.id === layout.canvases[area.id]) ? [{ id: layout.canvases[area.id]! }] : [])].flatMap(item => {
        const tab = checkout.tabs.find(tab => tab.id === item.id);
        if (!tab) return [];
        return [{ id: `pane_move_tab:${item.id}`, label: agentEntries(checkout).find(entry => entry.source_id === item.id)?.label ?? tab.label ?? item.id, unavailable: movingTab(operations, item.id) ? t("menus.paneMove.busy") : null, children: tab.panes.filter(pane => pane.id !== paneId).map(pane => ({ id: `pane_move_pane:${pane.id}`, label: pane.identity_label ?? pane.terminal_title ?? pane.herdr_label ?? pane.id, unavailable: reasonText(movePaneReason(pane, item.id, useShellStore.getState().rest?.terminal?.panes ?? [], operations)), children: MOVE_EDGES.map(edge => entry({ type: "pane", tab_id: item.id, pane_id: pane.id, area_id: area.id, edge }, t(`menus.${edge}`))) })) }];
      })],
    })) }];
  };
  const context: MoveContext = {
    active, checkout, layout, preview, menu,
    mutationBlocked: (paneId) => {
      const tabId = checkout.tabs.find(tab => tab.panes.some(pane => pane.id === paneId))?.id;
      return !!tabId && (movingTab(operations, tabId) || !!(held && request && [request.source.tab_id, "tab_id" in request.destination ? request.destination.tab_id : ""].includes(tabId)));
    },
    select: (id) => { const choice = menuTargets.get(id); if (!choice) return false; run(choice.source, choice.destination); return true; },
    press: (paneId, event) => {
      if (event.button !== 0 || !event.isPrimary || gesture.current) return;
      const source = paneLocation(checkout, layout, paneId);
      if (!source) return;
      const why = unavailable(source); setReason(why); if (why) return;
      // Keep terminal keys and selection while a header press becomes a drag.
      // Click still reaches the existing record button below the threshold.
      event.preventDefault();
      gesture.current = { source, stamp: stamp(source.tab_id), pointerId: event.pointerId, origin: { x: event.clientX, y: event.clientY }, dragging: false, target: null };
    },
  };
  const overlayStyle = target ? (() => {
    const { rect, destination } = target;
    const style = { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
    if (destination.type === "pane") {
      if (destination.edge === "left" || destination.edge === "right") { style.width /= 2; if (destination.edge === "right") style.left += style.width; }
      else { style.height /= 2; if (destination.edge === "down") style.top += style.height; }
    }
    return style;
  })() : null;
  return <Context.Provider value={context}><div ref={root} className="relative flex min-h-0 min-w-0 flex-1 flex-col" data-pane-move-active={active ? "true" : undefined}>
    {reason && !active ? <div className="absolute inset-x-0 top-0 z-40 flex items-center gap-sm bg-card px-sm py-xxs text-caption text-muted-foreground shadow-sm" role="status"><span className="min-w-0 flex-1 truncate">{t(`menus.paneMove.${reason === "pending" ? "busy" : reason}`)}</span><Button size="sm" variant="ghost" onClick={() => setReason(null)}>{t("workspace.dismiss")}</Button></div> : null}
    {(held || selectedOperation && selectedOperation.phase !== "completed") ? <div className="absolute inset-x-0 top-0 z-40 flex items-center gap-sm bg-card px-sm py-xxs text-caption text-muted-foreground shadow-sm" role="status" data-pane-move-status={operation?.phase ?? "pending"}>
      <span className="min-w-0 flex-1 truncate">{operation?.phase === "failed" ? (operation.message ?? t("menus.paneMove.failed")) : operation?.phase === "unknown" || unconfirmed ? t("menus.paneMove.unknown") : t("menus.paneMove.pending")}</span>
      {operation?.phase === "unknown" || unconfirmed || held && connection !== "live" ? <Button size="sm" variant="ghost" disabled={connection !== "live"} onClick={() => actions.checkPaneMove(operation?.id ?? request!.request_id)}>{t("workspace.checkStatus")}</Button> : null}
      {operation?.phase === "failed" && operation.retryable && request ? <Button size="sm" variant="ghost" onClick={() => run(request.source, request.destination)}>{t("common.retry")}</Button> : null}
      {operation?.phase === "failed" ? <Button size="sm" variant="ghost" onClick={() => setDismissed(operation.id)}>{t("workspace.dismiss")}</Button> : null}
    </div> : null}
    {children}
    {active ? createPortal(<div className="pointer-events-none fixed inset-0 z-50" aria-hidden="true">{overlayStyle ? <div className="absolute flex items-center justify-center overflow-hidden border-2 border-primary bg-primary/20 px-sm text-center text-caption text-foreground" style={overlayStyle} data-pane-move-target={target!.destination.type}>{target!.destination.type === "new_tab" ? t("menus.paneMove.newTab") : t(`menus.${target!.destination.type === "pane" ? target!.destination.edge : "right"}`)}</div> : null}{reason ? <div className="absolute bottom-lg left-1/2 max-w-full -translate-x-1/2 rounded-sm bg-popover px-sm py-xs text-caption text-popover-foreground shadow-sm">{t(`menus.paneMove.${reason === "pending" ? "busy" : reason}`)}</div> : null}</div>, document.body) : null}
  </div></Context.Provider>;
}

function PreviewTerminal({ pane }: { pane: PaneRow }) {
  const move = usePaneMove();
  const answer = useShellStore(state => state.rest?.status?.pane_move_preview);
  const text = answer?.phase === "ready" && answer.request_id === move?.preview?.requestId && answer.tab_id === move.preview.tabId ? answer.panes.find(row => row.pane_id === pane.id)?.text : undefined;
  const host = useRef<HTMLDivElement>(null);
  const { t } = useInterfaceTranslation();
  const [available, setAvailable] = useState(true);
  useEffect(() => {
    const node = host.current;
    if (!node) return;
    setAvailable(mountTerminalPreview(pane.id, node));
    return () => { node.replaceChildren(); };
  }, [pane.id]);
  return <section className="relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-background" data-pane-move-preview-pane={pane.id}>
    <div className="flex h-(--size-pane-header) shrink-0 items-center truncate bg-card px-sm text-caption text-muted-foreground">{pane.identity_label ?? pane.terminal_title ?? pane.herdr_label}</div>
    <div ref={host} className="pointer-events-none relative min-h-0 flex-1 overflow-hidden" aria-hidden="true" />
    {!available ? text !== undefined ? <pre className="pointer-events-none absolute inset-x-0 bottom-0 top-(--size-pane-header) overflow-hidden whitespace-pre px-xs py-xs font-mono text-terminal text-foreground" aria-hidden="true">{text}</pre> : <div className="pointer-events-none absolute inset-0 flex items-center justify-center px-sm text-caption text-muted-foreground">{t("menus.paneMove.previewUnavailable")}</div> : null}
  </section>;
}
export function PaneMovePreview({ areaId }: { areaId: string }) {
  const move = usePaneMove();
  const geometry = useShellStore(s => s.rest?.pane_layouts?.find(row => row.tab_id === move?.preview?.tabId));
  const remoteGeometry = useShellStore(s => s.rest?.status?.remote?.flatMap(row => row.session?.pane_layouts ?? []).find(row => row.tab_id === move?.preview?.tabId));
  if (!move?.preview || move.preview.areaId !== areaId) return null;
  const tab = move.checkout.tabs.find(tab => tab.id === move.preview!.tabId);
  if (!tab) return null;
  const render = (node: LayoutNode): ReactNode => node.type === "pane" ? (() => { const pane = tab.panes.find(pane => pane.id === node.pane_id); return pane ? <PreviewTerminal key={pane.id} pane={pane} /> : null; })() : <div className="grid h-full min-h-0 min-w-0" style={node.direction === "right" ? { gridTemplateColumns: `${node.ratio}fr ${1 - node.ratio}fr` } : { gridTemplateRows: `${node.ratio}fr ${1 - node.ratio}fr` }}><div className="min-h-0 min-w-0">{render(node.first)}</div><div className="min-h-0 min-w-0">{render(node.second)}</div></div>;
  return <div className="absolute inset-0 z-30 overflow-hidden bg-background" data-pane-move-preview-tab={tab.id}>{geometry ? render(geometry.zoomed ? { type: "pane", pane_id: geometry.focused_pane_id } : geometry.root) : remoteGeometry ? (remoteGeometry.zoomed ? [{ pane_id: remoteGeometry.focused_pane_id, x: 0, y: 0, width: 1, height: 1 }] : remoteGeometry.frames).map(frame => { const pane = tab.panes.find(pane => pane.id === frame.pane_id); return pane ? <div key={pane.id} className="absolute overflow-hidden" style={{ left: `${frame.x * 100}%`, top: `${frame.y * 100}%`, width: `${frame.width * 100}%`, height: `${frame.height * 100}%` }}><PreviewTerminal pane={pane} /></div> : null; }) : <div className="grid h-full min-h-0" style={{ gridTemplateColumns: `repeat(${tab.panes.length}, minmax(0, 1fr))` }}>{tab.panes.map(pane => <PreviewTerminal key={pane.id} pane={pane} />)}</div>}</div>;
}

export function PaneMovePlus() {
  const move = usePaneMove();
  const { t } = useInterfaceTranslation();
  return <><PlusIcon className="size-(--size-icon) shrink-0" />{move?.active ? <span className="min-w-0 max-w-(--size-tab-preferred) truncate px-xs text-caption" data-pane-move-new-tab="true">{t("menus.paneMove.newTab")}</span> : null}</>;
}
