import { useOffersReconnect } from "./centerReconnect";
import { ServerIcon } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { EntryContextMenu, type MenuEntry } from "./components/entry-menu";
import { Button } from "./components/ui/button";
import { escapeConsumed, Hint } from "./components/ui/tooltip";
import { RemotePaneCanvas } from "./PaneGrid";
import { remoteTabView, remoteView } from "./remote";
import { canRetryDevice, deviceAwayLine, deviceLine } from "./settings";
import { catalogWorkspaces, focusedRemoteDevice, frontCheckout, type Checkout } from "./snapshot";
import { awayDevice, machineName, windowDeviceId, wslPlace } from "./screenMachine";
import { useShellStore } from "./store";
import { focusTerminal } from "./terminals";
import { AgentAreas } from "./AgentAreas";
import { DividerGuide, LaneDivider } from "./LaneDivider";
import { Lane, laneUp, ToolRail } from "./Tools";
import { OverviewButton } from "./Overview";
import { RunningServers } from "./RunningServers";
import { useUiStore } from "./ui";
import { noteShellLayer } from "./browserViews";
import { workspaceKey } from "./viewLayout";
import { PairLayer } from "./PairLayer";
import { drawnPair } from "./pairView";
import { laneFrame, laneViewOf, pageLaneWidth, workspaceViewOf, type LaneFrame, type LaneInput, type LaneSizes, type WorkspaceView } from "./workspace";
import { keyboardOwner, noteKeyboardOwner } from "./viewFocus";
import { cn } from "./lib/utils";
import { useInterfaceTranslation } from "./i18n/client";
import "./laneCalls";
import "./missionReveal";
import { CheckoutCommit, CheckoutFileCount, CheckoutUpstream } from "./CheckoutGitHeader";

// A Workspace (PRD S6 D-01..D-05; S7 B12, B13; PRD right-lane D-01..D-07;
// PRD lane-two-columns D-01, D-02, D-09): one checkout's path in the
// window's top row (PRD sidebar-titlebar-refresh D-15, `WorkspaceTitle`),
// and in the body Agent Views (its engine tabs and their panes), the right
// lane's two columns (the front Agent tab's desk at the agents' side, and a
// tool, Memory, PRs, Explorer or Changes, beside the rail) and, at the right
// edge, the tool rail. Which columns show, the tool, each tab's desk and the
// three widths are the core's, per Workspace; this draws them and
// sends one event per operator choice.
//
// A shown desk stands at the body's right and Agent Views narrows beside it,
// every pane kept, so each terminal resizes once as it opens, closes or its
// divider lands (PRD lane-desk-pair-view D-50); a tool floats over the
// agents at the right edge with the desk, if shown, on its left, the two one
// layer, and resizes nothing; when it closes, the desk stands, so a View
// opened from a floating tool settles then (`tray.floats`). A terminal click or an agent chosen leaves the desk up, a
// press outside a floating tool closes the tool, and ⌘E or Escape closes the
// lane (PRD right-lane B4; operator decision 2026-10-23); a drag moves only a
// guide until it lands.

export function WorkspaceScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  const expanded = useUiStore((s) => s.laneExpanded);
  const stackFront = useUiStore((s) => s.stackFront);
  const [body, setBody] = useState<HTMLElement | null>(null);
  const width = useWidth(body);
  const sizes = useMemo<LaneSizes>(
    () => ({
      agentMin: tokenPx("--size-agent-views-min"),
      viewMin: tokenPx("--size-lane-min"),
      viewIdeal: tokenPx("--size-lane-ideal"),
      lanePage: tokenPx("--size-lane-page"),
      explorerMin: tokenPx("--size-tree-min"),
      explorerIdeal: tokenPx("--size-tree-ideal"),
      panelMin: tokenPx("--size-lane-panel-min"),
      panelIdeal: tokenPx("--size-lane-panel-ideal"),
      divider: tokenPx("--spacing-sm"),
    }),
    [],
  );
  const opening = useShellStore((s) => (s.editor?.opening ?? []).some((row) => row.checkout_id === checkout?.id));
  const hasViews = (view?.layout?.display_count ?? 0) > 0 || opening;
  // The viewing column draws the front tab's desk (PRD lane-desk-pair-view
  // D-06); pair view draws it whatever the lane's own state, with a tool only
  // when the lane is open (D-46).
  const pair = drawnPair(view);
  const laneView = pair !== null ? "documents" : laneViewOf(view, hasViews);
  const input: LaneInput = {
    open: view?.open === true,
    view: laneView,
    tool: (pair === null || view?.open === true ? view?.tool : null) ?? null,
    documentWidth: view?.document_width ?? null,
    explorerWidth: view?.explorer_width ?? null,
    panelWidth: view?.panel_width ?? null,
    expanded,
    floats: view?.tray.floats === true,
    pair: pair !== null,
    pairRatio: view?.pair_mode?.ratio ?? null,
    front: stackFront,
    body: width,
    sizes,
  };
  const frame = laneFrame(input);
  // The documents of a folded desk are laid out at their own
  // width, so their View areas measure what they will show (an Open to the
  // side's room).
  const documents = laneView === "documents" ? frame : laneFrame({ ...input, open: true, view: "documents", front: "view" });
  const up = laneUp(frame.mode);
  const checkoutId = checkout?.id ?? null;
  // What the two columns draw now, each `none`/null while not drawn.
  const drawnView = frame.view === null ? "none" : laneView;
  const drawnTool = frame.tool === null ? null : (view?.tool ?? null);
  // A lane call that put a page in front widens the documents once, so the
  // page lays out wide enough for a site's desktop layout (PRD
  // browser-width-zoom B1-B5; lane-two-columns B18); an expanded lane, or a
  // call into another Workspace, is left as it is. The width is stored like
  // a divider landing, and a refusal leaves the page at the width it opened
  // at, with only the log saying so.
  const pageWiden = useUiStore((s) => s.pageWiden);
  const front = view ? workspaceKey(view) : null;
  const beside = frame.view && frame.tool ? frame.tool.width : null;
  useLayoutEffect(() => {
    if (pageWiden === null || width <= 0) return;
    useUiStore.getState().setPageWiden(null);
    if (pageWiden !== front || expanded || drawnView !== "documents") return;
    const documentWidth = pageLaneWidth({ width: view?.document_width ?? null, beside, body: width, sizes });
    if (documentWidth !== null && actions.setWorkspaceView({ document_width: documentWidth }) === false) {
      useShellStore.getState().noteDiagnostic(`document_width: widening for a page refused (${documentWidth})`);
    }
  }, [pageWiden, front, width, expanded, drawnView, view?.document_width, beside, sizes, actions]);
  // The rail, the chords and the close shortcut read what is drawn.
  useLayoutEffect(() => {
    useUiStore.getState().setDrawnLane({ mode: frame.mode, view: drawnView, tool: drawnTool, stacked: frame.stacked });
  }, [frame.mode, drawnView, drawnTool, frame.stacked]);
  // Off the Workspace screen nothing is drawn, so a chord there opens the lane.
  useLayoutEffect(() => () => useUiStore.getState().setDrawnLane({ mode: "closed", view: "none", tool: null, stacked: false }), []);
  // A closed lane is not expanded when it opens again.
  const closed = frame.mode === "closed";
  useLayoutEffect(() => {
    if (closed) useUiStore.getState().setLaneExpanded(false);
  }, [closed]);
  // A lane opened or closed moves no element the page observes, so the
  // native pages in it are placed again in the same frame (PRD right-lane B21).
  useLayoutEffect(() => noteShellLayer(), [frame.mode, drawnView, drawnTool]);
  // Agent Views under an expanded lane keeps its size and takes no keyboard,
  // so its keyboard target is retired before a chord can act on a pane out
  // of sight.
  const agentsCovered = frame.mode === "expanded";
  useLayoutEffect(() => {
    const owner = keyboardOwner();
    if (agentsCovered && (owner.kind === "pane" || owner.kind === "agent") && owner.workspace === checkoutId) noteKeyboardOwner({ kind: "none" });
  }, [agentsCovered, checkoutId]);
  // A lane that goes out of sight with the keyboard inside it, or a desk
  // folded with the keyboard in it, hands the
  // keyboard to the pane the core has focused, as after a closing sheet, so
  // no focus is reported for it (PRD right-lane B4, B23; lane-two-columns
  // B11). Documents that closed with their last view are not a fold: the
  // close's own follow-up hands the keyboard to the Explorer left in the
  // tool column, or to the pane (B12, `followViewClose`).
  const documentsUp = up && drawnView === "documents";
  const displays = view?.layout?.display_count ?? 0;
  const shownBefore = useRef({ up, documentsUp });
  useLayoutEffect(() => {
    const before = shownBefore.current;
    shownBefore.current = { up, documentsUp };
    const owner = keyboardOwner();
    const lost = (before.up && !up && (owner.kind === "view" || owner.kind === "tool")) || (before.documentsUp && !documentsUp && displays > 0 && owner.kind === "view");
    if (!lost || owner.workspace !== checkoutId) return;
    const paneId = useShellStore.getState().focusedPaneId;
    // A native page can report its focus after a menu command ran. The
    // shell's textarea can already be active, so focus() then emits no DOM
    // event. Retire the hidden owner with the drawing, independently of it.
    noteKeyboardOwner(paneId && checkoutId ? { kind: "pane", workspace: checkoutId, paneId } : { kind: "none" });
    if (paneId) focusTerminal(paneId);
  }, [up, documentsUp, displays, checkoutId]);
  if (!checkout || !view) return null;
  return (
    <section
      className="flex min-h-0 min-w-0 flex-1 overflow-hidden"
      aria-label={t("panes.workspace.aria", { name: checkout.branch ?? checkout.label })}
      data-workspace-screen={checkout.id}
      data-lane-mode={frame.mode}
      data-lane-view={drawnView}
      data-lane-shows={drawnTool ?? "none"}
      data-lane-stacked={frame.stacked ? "true" : undefined}
    >
      <LaneRow view={view} frame={frame} documents={documents} body={body} width={width} setBody={setBody} sizes={sizes} actions={actions} checkout={checkout} pair={pair?.pane_id ?? null} />
      <div className="contents" onKeyDown={(event) => laneEscape(event, frame, actions)}>
        <ToolRail actions={actions} />
      </div>
    </section>
  );
}

/**
 * Escape in the lane or on its rail, once nothing inside answered (a menu, a
 * filter, a document's find), returns an expanded lane; closes a floating
 * tool beside or over the desk, which then stands (D-50); or closes the lane
 * as ⌘E does. Either gives the keyboard to the focused pane, so a
 * tool or an open afterwards brings its own column alone (B4, B8, B15;
 * operator decision 2026-10-23); Escape in a terminal stays the program's.
 * Escape in a document's editor stays the editor's too: it lets the next Tab
 * leave the editor rather than indent, the keyboard's way out of a document,
 * and an Escape from there (the tab strip, the toolbar) closes the lane
 * (PRD lane-desk-pair-view B30, which took away the docked lane that kept it).
 * A hint that Escape closes (a focused tab's full name) is no answer.
 */
function laneEscape(event: React.KeyboardEvent<HTMLElement>, frame: LaneFrame, actions: Actions): void {
  if (event.key !== "Escape" || escapeConsumed(event)) return;
  if (frame.mode === "expanded") useUiStore.getState().setLaneExpanded(false);
  else if (frame.mode === "overlay" || frame.mode === "docked") {
    if (event.target instanceof Element && event.target.closest("[data-editor-body]")) return;
    if (frame.view !== null && frame.tool !== null) actions.closeTool();
    else actions.closeLane();
    const paneId = useShellStore.getState().focusedPaneId;
    if (paneId) focusTerminal(paneId);
  } else return;
  event.preventDefault();
  event.stopPropagation();
}

function tokenPx(name: string): number {
  return Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
}

/** An element's width, followed as it changes; 0 until it is measured. */
function useWidth(element: HTMLElement | null): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    if (!element) return undefined;
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return width;
}

/**
 * The body left of the rail: Agent Views, and the lane. The lane is always
 * placed at the body's right edge, so moving between standing, floating and
 * expanded never mounts it again (B8; lane-desk-pair-view B35); a standing
 * desk keeps its width free at Agent Views' right, so the panes narrow
 * beside it rather than under it (D-50).
 */
function LaneRow({ view, frame, documents, body, width, setBody, sizes, actions, checkout, pair }: { view: WorkspaceView; frame: LaneFrame; documents: LaneFrame; body: HTMLElement | null; width: number; setBody: (element: HTMLElement | null) => void; sizes: LaneSizes; actions: Actions; checkout: Checkout; pair: string | null }) {
  const { t } = useInterfaceTranslation();
  const [guide, setGuide] = useState<number | null>(null);
  // A terminal click leaves the lane where it is: only the operator closes
  // it, with ⌘E or Escape (B4).
  const escape = (event: React.KeyboardEvent<HTMLElement>) => laneEscape(event, frame, actions);
  // In pair view the lane is the half beside the paired pane, under the tab
  // row and the strip (PRD lane-desk-pair-view B17, D-16); it is the same
  // element in every mode, so entering pair view mounts no desk again.
  const placed = frame.mode === "expanded" ? "inset-0 z-20" : frame.mode === "pair" ? "bottom-0 right-0 top-[calc(var(--size-tab-strip)*2)] z-20" : "inset-y-0 right-0 z-20";
  // What a standing desk takes from Agent Views, kept under an expanded lane so expanding resizes no terminal.
  const reserve = frame.agents > 0 && width > frame.agents ? width - frame.agents : 0;
  // A press outside a floating tool closes it, and a desk floating with it stands (D-50); the press still lands.
  const toolFloats = frame.tool !== null && (frame.mode === "overlay" || frame.mode === "docked");
  const pressOutside = toolFloats ? () => actions.closeTool() : undefined;

  return (
    <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1" data-lane-row="true">
      {/* Its own stacking context, so nothing the agents raise (a pane
          divider) draws over the lane; out of reach of the pointer and the
          keyboard while an expanded lane covers it. */}
      <div className="isolate flex min-h-0 min-w-0 flex-1 flex-col" style={reserve > 0 ? { marginRight: reserve } : undefined} inert={frame.mode === "expanded"} onPointerDownCapture={pressOutside} data-agent-area="true">
        <AgentArea checkout={checkout} actions={actions} />
      </div>
      {pair !== null ? <PairLayer paneId={pair} paneWidth={frame.pane ?? 0} covered={frame.mode === "expanded"} actions={actions} /> : null}
      {frame.mode !== "closed" ? (
        <div
          // Over the agents the lane's edge line is its divider's (`LaneDivider`), which stands outside the frame at its
          // outer edge, so the divider is where the landing math puts it; elsewhere the frame draws the line itself.
          // A standing desk is part of the body, so it casts no shadow; what floats does, a standing desk lifted beside a tool included.
          className={cn("absolute flex min-h-0 min-w-0 flex-col bg-card", frame.mode === "expanded" && "border-l border-border", (frame.mode === "overlay" || frame.mode === "expanded" || (frame.mode === "docked" && frame.tool !== null)) && "shadow-overlay", placed)}
          style={frame.mode === "expanded" ? undefined : { width: frame.width }}
          onKeyDown={escape}
          data-lane-frame={frame.mode}
        >
          {/* A lane over the agents is sized from its own left edge, nothing beside it moving (B6); a standing desk's divider narrows Agent Views once it lands (D-50). */}
          {frame.mode === "overlay" || frame.mode === "docked" ? (
            <LaneDivider kind="lane" edge label={t("panes.workspace.resize", { name: t("panes.lane.aria") })} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} />
          ) : frame.mode === "pair" ? (
            // Pair view's divider between the paired pane and the desk (D-47): it lands the mode's one ratio.
            <LaneDivider kind="pair" edge label={t("panes.pair.resize")} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} />
          ) : null}
          <Lane view={view} frame={frame} documents={documents} sizes={sizes} checkout={checkout} actions={actions} />
        </div>
      ) : null}
      {guide !== null ? <DividerGuide left={guide} pair={frame.mode === "pair"} /> : null}
    </div>
  );
}

/**
 * The Workspace's part of the window's top row (PRD sidebar-titlebar-refresh
 * D-15, D-16, B17, B18; PRD right-lane B1): a remote device's band, the
 * `project / checkout` path, active Git facts, and Overview then Open server.
 * The lane's tools are on the rail. The project opens its
 * Overview; a right-click on the path opens the Workspace's menu. Nothing
 * while a Workspace is not the screen in front.
 */
export function WorkspaceTitle({ actions }: { actions: Actions }) {
  const workspaceScreen = useUiStore((s) => s.screen?.kind === "workspace");
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  if (!workspaceScreen || !checkout || !view) return null;
  return <WorkspaceTitleBar checkout={checkout} view={view} actions={actions} />;
}

function WorkspaceTitleBar({ checkout, view, actions }: { checkout: Checkout; view: WorkspaceView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const project = useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.checkouts.some((candidate) => candidate.id === checkout.id)) ?? null);
  // The band marks a Workspace on another machine than this window's, the core's included; the window's own has none.
  const device = useShellStore((s) => awayDevice(s.rest));
  const deviceName = useShellStore((s) => (device ? machineName(s.rest?.navigator?.devices, device.id, t) : null));
  // The menu says what the body draws now (PRD right-lane B20).
  const drawn = useUiStore((s) => s.drawnLane);
  const up = laneUp(drawn.mode);
  const name = checkout.branch ?? checkout.label;
  // What the menu acts on is this Workspace: its lane, its path and its
  // Project (B15). Opening the menu changes none of them.
  const menuItems = (): MenuEntry<ToolbarMenuId>[] => [
    { id: "lane", label: up ? t("panes.lane.hideLane") : t("panes.lane.showLane"), unavailable: null },
    { id: "copy_path", label: t("panes.workspace.copyPath"), unavailable: null, separated: true },
  ];
  const select = (id: ToolbarMenuId) => {
    if (id === "lane") return actions.toggleLane();
    if (id === "copy_path") return void actions.copyText(checkout.path, "checkout path");
  };
  const [headerElement, setHeaderElement] = useState<HTMLElement | null>(null);
  const headerWidth = useWidth(headerElement);
  const git = project?.is_git;
  return (
    <div ref={setHeaderElement} className="flex h-full min-w-0 flex-1 items-center gap-xs pr-sm text-caption" data-workspace-toolbar="true">
      {device ? (
        // The front thing is another machine's: its color and name lead the path, and the pane border wears the same color (PRD home-device-rail D-15).
        <span className="flex h-full max-w-(--size-recent-location-max) shrink-0 items-center gap-xs bg-device-remote px-sm font-medium text-primary-foreground" data-device-band={device.id}>
          <ServerIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 truncate">{deviceName}</span>
        </span>
      ) : null}
      <EntryContextMenu label={t("panes.workspace.aria", { name })} items={menuItems} onSelect={select} className={cn("flex min-w-0 shrink self-stretch items-center", !device && "pl-sm")} data-workspace-menu="true">
        <nav aria-label={t("panes.workspace.location")} className="flex min-w-0 items-center gap-xs">
          {project ? (
            <>
              <Hint label={`${project.label} · ${project.path}${deviceName ? ` · ${deviceName}` : ""}`}>
                <button
                  type="button"
                  className="min-w-0 max-w-(--size-recent-location-max) shrink truncate rounded-xs px-xs text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:bg-accent"
                  data-workspace-project={project.id}
                  onClick={() => actions.openProjectPanel(project.device_id, project.id)}
                >
                  {project.label}
                </button>
              </Hint>
              <span aria-hidden="true" className="text-muted-foreground">/</span>
            </>
          ) : null}
          {/* Not a drag handle: its hint and the path menu need the pointer (B18, B19). */}
          <Hint label={`${name} · ${checkout.path}`}>
            <span className="min-w-0 truncate px-xs text-foreground" aria-current="page" data-workspace-location="checkout" data-window-control="true">
              {name}
            </span>
          </Hint>
        </nav>
      </EntryContextMenu>
      {git ? <><CheckoutFileCount header={view.git} actions={actions} /><CheckoutUpstream header={view.git} onCatchUp={(headSha) => actions.catchUpCheckout(view.device_id, view.path, headSha)} /></> : null}
      <div className="min-w-0 flex-1" data-window-drag="true" />
      {git ? <CheckoutCommit key={`${view.device_id}:${view.path}`} header={view.git} view={view} actions={actions} title={headerWidth >= tokenPx("--size-checkout-header-commit-min")} time={headerWidth >= tokenPx("--size-checkout-header-time-min")} /> : null}
      <div className="flex shrink-0 items-center gap-xxs" data-workspace-actions="true">
        <OverviewButton actions={actions} />
        <RunningServers key={checkout.id} checkout={checkout} view={view} actions={actions} />
      </div>
    </div>
  );
}

type ToolbarMenuId = "lane" | "copy_path";

/** This machine's tabs, or the selected SSH device's own tabs and panes in their place (S5 B19). */
function AgentArea({ checkout, actions }: { checkout: Checkout; actions: Actions }) {
  const remote = useShellStore((s) => focusedRemoteDevice(s.rest) !== null);
  if (remote) return <RemoteAgentArea actions={actions} />;
  return <LocalAgentArea checkout={checkout} actions={actions} />;
}

/**
 * The strip lists only the checkout's operator tabs, but the canvas draws
 * whichever tab the core made visible: a tab holding nothing but delegated
 * children stays off the strip while the sidebar still reaches it (PRD
 * hide-orchestrator B1, B4), so a checkout whose only tab is delegated is
 * not empty. The empty state means there is no tab at all.
 */
function LocalAgentArea({ checkout, actions }: { checkout: Checkout; actions: Actions }) {
  return <AgentAreas checkout={checkout} actions={actions} />;
}

/**
 * A selected SSH device's Agent area: that host's visible tab, drawn from the
 * session the core projected over SSH, or the connection's own state when
 * there is no session. A lost connection keeps the last session on screen
 * and names the device (B21); nothing here reads this machine's tabs.
 */
function RemoteAgentArea({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const device = useShellStore((s) => focusedRemoteDevice(s.rest));
  const status = useShellStore((s) => (device ? (s.rest?.status?.remote?.find((row) => row.target_id === device.id) ?? null) : null));
  const name = useShellStore((s) => (device ? machineName(s.rest?.navigator?.devices, device.id, t) : ""));
  // Beside the name, the SSH alias it is reached by, or where a WSL distribution is (PRD wsl-device B5).
  const where = useShellStore((s) => (device ? (wslPlace(s.rest?.navigator?.devices, device.id, t) ?? device.ssh_alias) : null));
  // The machine this window runs on, which "Show this device" brings forward; on a node it is not the core's.
  const own = useShellStore((s) => windowDeviceId(s.rest?.navigator?.devices));
  const session = status?.session ?? null;
  const front = useShellStore((s) => s.rest?.workspace_view);
  const view = useMemo(() => remoteView(session, front), [session, front]);
  // The reconnect below, in the empty surface or the stale band, is the one the sidebar then leaves out.
  useOffersReconnect(device && !deviceAwayLine(device, name, t).toRow && canRetryDevice(device, status ?? undefined) && (!view || status?.state !== "connected") ? device.id : null);
  if (!device) return null;
  const connected = status?.state === "connected";
  const away = deviceAwayLine(device, name, t);
  if (!view) {
    const line = deviceLine(device, status ?? undefined, t);
    return (
      <div className="flex min-h-0 flex-1 flex-col items-start justify-center gap-sm p-xl text-body text-subtle-foreground" data-remote-device-surface={device.id} data-remote-state={status?.state ?? "none"}>
        <h2 className="text-title font-semibold text-foreground">
          {name} <span className="font-mono text-caption text-muted-foreground">{where}</span>
        </h2>
        <p>{connected ? t("panes.workspace.connectedEmpty", { device: name }) : t("panes.workspace.remoteState", { device: name, state: line.text })}</p>
        {away.hint ? <p className="text-caption text-muted-foreground">{away.hint}</p> : null}
        <div className="flex gap-sm">
          {away.toRow ? (
            <Button variant="secondary" onClick={() => useUiStore.getState().openSettings("devices")} data-remote-sign-in={device.id}>
              {t("devices.signIn.openRow")}
            </Button>
          ) : canRetryDevice(device, status ?? undefined) ? (
            <Button variant="secondary" onClick={() => actions.retryDevice(device.id)} data-remote-retry={device.id}>
              {t("sidebar.reconnect")}
            </Button>
          ) : null}
          {device.id === own ? null : (
            <Button variant="ghost" onClick={() => actions.focusDevice(own)} data-use-local-device="true">
              {t("panes.workspace.showThisMachine")}
            </Button>
          )}
        </div>
      </div>
    );
  }
  return (
    <>
      {connected ? null : (
        // A lost connection keeps the last session on screen, but the host
        // takes no command until it is back (`remote.control.not_connected`).
        <div role="status" className="flex items-center gap-md border-b border-border bg-card px-md py-xs text-caption text-warning" data-remote-stale={device.id}>
          {away.hint ? (
            <Hint label={away.hint} reveals>
              <span className="min-w-0 flex-1 truncate">{away.text}</span>
            </Hint>
          ) : (
            <span className="min-w-0 flex-1 truncate">{away.text}</span>
          )}
          {away.toRow ? (
            <Button variant="secondary" onClick={() => useUiStore.getState().openSettings("devices")} data-remote-sign-in={device.id}>
              {t("devices.signIn.openRow")}
            </Button>
          ) : canRetryDevice(device, status ?? undefined) ? (
            <Button variant="secondary" onClick={() => actions.retryDevice(device.id)} data-remote-retry={device.id}>
              {t("sidebar.reconnect")}
            </Button>
          ) : null}
        </div>
      )}
      <AgentAreas checkout={view.checkout} deviceId={device.id} actions={actions} remoteBody={(tabId) => <RemotePaneCanvas view={session ? remoteTabView(session, view, tabId) : view} connected={connected} actions={actions} />} />
    </>
  );
}
