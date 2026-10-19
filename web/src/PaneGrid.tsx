import { memo, useCallback, useRef, useState } from "react";
import type { Actions } from "./actions";
import { markTone } from "./agentRow";
import { useInterfaceTranslation } from "./i18n/client";
import { PaneView } from "./PaneView";
import { contextAgents, frameStyle, type RemoteView } from "./remote";
import { resizeStep } from "./resize";
import { windowDeviceId } from "./screenMachine";
import { cn } from "./lib/utils";
import {
  type AgentRow,
  dividerPaneId,
  focusedCheckout,
  layoutForTab,
  type LayoutNode,
  localDeviceId,
  type PaneLayout,
  type PaneRow,
  type Tab,
  type TerminalPane,
} from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

// The center draws the visible tab's projection tree as nested CSS grids
// (PRD S2 B4): every split is a two-track grid sized by Herdr's ratio and
// every leaf is a pane with its own xterm instance. A zoomed tab draws only
// the zoomed pane, over the whole canvas, so its fit follows the full
// geometry Herdr gave the PTY (B19); the other panes' terminals stay alive
// and fed while parked (`toggle_zoom` decides, the shell only draws).

type PaneProps = {
  panes: Map<string, PaneRow>;
  transports: Map<string, TerminalPane>;
  scales: Record<string, number>;
  focusedPaneId: string;
  agents: Map<string, AgentRow>;
  paneCount: number;
  zoomed: boolean;
  actions: Actions;
};

/** The agents of the context on screen, by pane, as the sidebar reads them: what a pane header draws as its mark and logo. */
function useAgentsByPane(): Map<string, AgentRow> {
  const agents = useShellStore((s) => contextAgents(s.rest, s.agents));
  return new Map(agents.map((agent) => [agent.pane_id, agent]));
}

/** The header's agent facts as plain values, so a pane redraws only when they change. */
function agentProps(agent: AgentRow | undefined) {
  return { agentKind: agent?.agent_kind ?? null, markSymbol: agent?.mark ?? null, markStatus: agent?.status_code ?? null, markTone: agent ? markTone(agent) : "", lettersWaiting: agent?.letters_waiting ?? 0 };
}

export function PaneCanvas({ actions, tab }: { actions: Actions; tab: Tab | null }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => focusedCheckout(s.rest));
  const layout = useShellStore((s) => layoutForTab(s.rest, tab?.id ?? null));
  const transportRows = useShellStore((s) => s.rest?.terminal?.panes);
  const scales = useShellStore((s) => s.rest?.ui_state?.pane_text_scales);
  if (!checkout || !tab || !layout) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center text-caption text-muted-foreground" data-canvas="empty">
        {checkout ? t("panes.canvas.noTab") : t("panes.canvas.noCheckout")}
      </div>
    );
  }
  return (
    <TabCanvas
      key={tab.id ?? "tab"}
      tab={tab}
      layout={layout}
      transportRows={transportRows ?? EMPTY_TRANSPORTS}
      scales={scales ?? EMPTY_SCALES}
      actions={actions}
    />
  );
}

const EMPTY_TRANSPORTS: TerminalPane[] = [];

/**
 * A selected SSH device's visible tab (PRD S5 B19). Herdr reports a remote
 * tab's geometry as each pane's rectangle rather than the split tree, so the
 * panes are placed where it says. Hairlines mark internal edges, without a
 * resize handle: a remote pane's size is its host's. A zoomed tab draws the pane Herdr
 * zoomed, its focused one, over the whole canvas.
 */
export function RemotePaneCanvas({
  view,
  connected,
  actions,
}: {
  view: RemoteView;
  /** False while the device's connection is down; the panes show its last state. */
  connected: boolean;
  actions: Actions;
}) {
  const transportRows = useShellStore((s) => s.rest?.terminal?.panes ?? EMPTY_TRANSPORTS);
  const scales = useShellStore((s) => s.rest?.ui_state?.pane_text_scales ?? EMPTY_SCALES);
  const agents = useAgentsByPane();
  // A device's panes wear its border, except a node's own drawn in its own window.
  const away = useShellStore((s) => view.workspace.device_id !== windowDeviceId(s.rest?.navigator?.devices));
  const { t } = useInterfaceTranslation();
  const { tab, layout } = view;
  if (!tab || !layout) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center text-caption text-muted-foreground" data-canvas="empty">
        {tab ? t("panes.canvas.remoteLayoutPending") : t("panes.canvas.noTab")}
      </div>
    );
  }
  const panes = new Map(tab.panes.map((pane) => [pane.id, pane]));
  const transports = new Map(transportRows.map((row) => [row.pane_id, row]));
  const frames = layout.zoomed ? [{ pane_id: layout.focused_pane_id, x: 0, y: 0, width: 1, height: 1 }] : layout.frames;
  return (
    <div className={cn("relative min-h-0 min-w-0 flex-1", away && "border border-device-remote")} data-canvas={tab.id ?? ""} data-remote-canvas="true" data-zoomed={layout.zoomed ? "true" : "false"}>
      {frames.map((frame) => {
        const pane = panes.get(frame.pane_id);
        return (
          <div key={frame.pane_id} className="absolute min-h-0 min-w-0" style={frameStyle(frame)} data-layout-pane={frame.pane_id}>
            {pane ? (
              <PaneView
                pane={pane}
                transport={transports.get(frame.pane_id)}
                focused={view.focusedPaneId === frame.pane_id}
                scale={scales[frame.pane_id] ?? 1}
                actions={actions}
                {...agentProps(agents.get(frame.pane_id))}
                paneCount={tab.panes.length}
                zoomed={layout.zoomed}
                local={false}
                offline={!connected}
              />
            ) : (
              <div className="h-full bg-background" data-pane-missing={frame.pane_id} />
            )}
            {frame.x > 0 ? <PaneBoundary vertical /> : null}
            {frame.y > 0 ? <PaneBoundary vertical={false} /> : null}
          </div>
        );
      })}
    </div>
  );
}
const EMPTY_SCALES: Record<string, number> = {};

const TabCanvas = memo(function TabCanvas({
  tab,
  layout,
  transportRows,
  scales,
  actions,
}: {
  tab: Tab;
  layout: PaneLayout;
  transportRows: TerminalPane[];
  scales: Record<string, number>;
  actions: Actions;
}) {
  const focusedPaneId = useShellStore((s) => s.focusedPaneId);
  const agents = useAgentsByPane();
  // The core's own panes are another machine's in a window on a node: they wear its border.
  const away = useShellStore((s) => localDeviceId(s.rest) !== windowDeviceId(s.rest?.navigator?.devices));
  const panes = new Map(tab.panes.map((pane) => [pane.id, pane]));
  const transports = new Map(transportRows.map((row) => [row.pane_id, row]));
  const props: PaneProps = {
    panes,
    transports,
    scales,
    focusedPaneId: focusedPaneId ?? "",
    agents,
    paneCount: tab.panes.length,
    zoomed: layout.zoomed,
    actions,
  };
  const root: LayoutNode = layout.zoomed ? { type: "pane", pane_id: layout.focused_pane_id } : layout.root;
  return (
    <div className={cn("relative min-h-0 min-w-0 flex-1", away && "border border-device-remote")} data-canvas={tab.id ?? ""} data-zoomed={layout.zoomed ? "true" : "false"}>
      <LayoutView node={root} {...props} />
    </div>
  );
});

function LayoutView({ node, ...props }: { node: LayoutNode } & PaneProps) {
  if (node.type === "pane") {
    const pane = props.panes.get(node.pane_id);
    return (
      <div className="relative h-full w-full min-h-0 min-w-0" data-layout-pane={node.pane_id}>
        {pane ? (
          <PaneView
            pane={pane}
            transport={props.transports.get(node.pane_id)}
            focused={props.focusedPaneId === node.pane_id}
            scale={props.scales[node.pane_id] ?? 1}
            actions={props.actions}
            {...agentProps(props.agents.get(node.pane_id))}
            paneCount={props.paneCount}
            zoomed={props.zoomed}
          />
        ) : (
          // The layout names a pane the tab rows do not carry yet; the next
          // snapshot resolves it, and nothing is drawn that could be wrong.
          <div className="h-full bg-background" data-pane-missing={node.pane_id} />
        )}
      </div>
    );
  }
  return <SplitView node={node} {...props} />;
}

function SplitView({ node, ...props }: { node: Extract<LayoutNode, { type: "split" }> } & PaneProps) {
  const vertical = node.direction === "right";
  const first = `${node.ratio * 100}%`;
  const second = `${(1 - node.ratio) * 100}%`;
  return (
    <div
      className="grid h-full min-h-0 w-full min-w-0"
      style={vertical ? { gridTemplateColumns: `${first} ${second}` } : { gridTemplateRows: `${first} ${second}` }}
      data-split={node.direction}
    >
      <div className="relative min-h-0 min-w-0">
        <LayoutView node={node.first} {...props} />
      </div>
      <div className="relative min-h-0 min-w-0">
        <LayoutView node={node.second} {...props} />
        <PaneBoundary vertical={vertical} />
      </div>
      <Divider node={node} dispatch={props.actions.dispatch} />
    </div>
  );
}

/** One leading edge, painted over content without taking space or pointer input.
 * Focus outlines keep their higher layer; only one side owns a shared edge. */
function PaneBoundary({ vertical }: { vertical: boolean }) {
  return (
    <div
      aria-hidden="true"
      data-pane-boundary={vertical ? "vertical" : "horizontal"}
      className={cn("pointer-events-none absolute left-0 top-0 bg-border", vertical ? "h-full w-(--size-hairline)" : "h-(--size-hairline) w-full")}
    />
  );
}

/**
 * The handle on a split boundary. While dragging only the guide line moves;
 * releasing sends one `resize_pane` for the first subtree's last pane, or
 * nothing when the change falls outside the core's range (PRD S2 B6).
 */
function Divider({ node, dispatch }: { node: Extract<LayoutNode, { type: "split" }>; dispatch: DispatchFn }) {
  const { t } = useInterfaceTranslation();
  const vertical = node.direction === "right";
  const [travel, setTravel] = useState<number | null>(null);
  const origin = useRef(0);
  const spanRef = useRef(0);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const parent = event.currentTarget.parentElement;
      if (!parent) return;
      event.preventDefault();
      event.currentTarget.setPointerCapture(event.pointerId);
      const rect = parent.getBoundingClientRect();
      spanRef.current = vertical ? rect.width : rect.height;
      origin.current = vertical ? event.clientX : event.clientY;
      setTravel(0);
    },
    [vertical],
  );
  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (travel === null) return;
      setTravel((vertical ? event.clientX : event.clientY) - origin.current);
    },
    [travel, vertical],
  );
  const onPointerUp = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (travel === null) return;
      event.currentTarget.releasePointerCapture(event.pointerId);
      const step = resizeStep(travel, spanRef.current, vertical);
      setTravel(null);
      if (!step) return;
      dispatch({
        schema_version: 2,
        kind: "resize_pane",
        payload: { pane_id: dividerPaneId(node.first), direction: step.direction, amount: step.amount },
      });
    },
    [travel, vertical, dispatch, node.first],
  );
  const onPointerCancel = useCallback(() => setTravel(null), []);

  const at = `${node.ratio * 100}%`;
  const grab = vertical
    ? { left: `calc(${at} - var(--size-resize-grab) / 2)`, top: 0, bottom: 0, width: "var(--size-resize-grab)" }
    : { top: `calc(${at} - var(--size-resize-grab) / 2)`, left: 0, right: 0, height: "var(--size-resize-grab)" };
  const guide =
    travel === null
      ? null
      : vertical
        ? { left: `calc(${at} + ${travel}px - var(--size-resize-handle) / 2)`, top: 0, bottom: 0, width: "var(--size-resize-handle)" }
        : { top: `calc(${at} + ${travel}px - var(--size-resize-handle) / 2)`, left: 0, right: 0, height: "var(--size-resize-handle)" };
  return (
    <>
      <div
        role="separator"
        aria-orientation={vertical ? "vertical" : "horizontal"}
        aria-label={vertical ? t("panes.divider.width") : t("panes.divider.height")}
        data-divider={dividerPaneId(node.first)}
        className={`absolute z-20 ${vertical ? "cursor-col-resize" : "cursor-row-resize"}`}
        style={grab}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
      />
      {guide ? <div className="pointer-events-none absolute z-20 bg-primary" style={guide} data-resize-guide="true" /> : null}
    </>
  );
}
