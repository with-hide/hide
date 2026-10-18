import { ServerIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { EntryContextMenu, type MenuEntry } from "./components/entry-menu";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { RemotePaneCanvas } from "./PaneGrid";
import { remoteTabView, remoteView } from "./remote";
import { canRetryDevice, deviceAwayLine, deviceLine } from "./settings";
import { catalogWorkspaces, focusedRemoteDevice, frontCheckout, type Checkout } from "./snapshot";
import { awayDevice, machineName, windowDeviceId } from "./screenMachine";
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
import { laneFrame, laneToolOf, workspaceViewOf, type LaneFrame, type LaneSizes, type LaneTool, type WorkspaceView } from "./workspace";
import { keyboardOwner, noteKeyboardOwner } from "./viewFocus";
import { cn } from "./lib/utils";
import { useInterfaceTranslation } from "./i18n/client";
import "./laneCalls";

// A Workspace (PRD S6 D-01..D-05; S7 B12, B13; PRD right-lane D-01..D-07):
// one checkout's path in the window's top row (PRD sidebar-titlebar-refresh
// D-15, `WorkspaceTitle`), and in the body Agent Views (its Herdr tabs and
// their panes), the right lane (one tool: Memory, PRs, Files or Changes,
// or the window's 보기 slot) and, at the right edge, the tool rail. The lane's tool, pin, tree
// and widths are the core's, per Workspace; this draws them and sends one
// event per operator choice.
//
// An unpinned lane is drawn over Agent Views, which keeps its width, so
// opening and closing it resizes no terminal (B3, B24); a terminal click,
// Escape or an agent chosen puts it away, mounted out of sight so its
// documents, scroll and pages are as they were when it comes back (B4). A
// pinned lane docks and Agent Views gives up its width, so the pin and a
// divider landing resize the agents' terminals once (B5, B6); a drag moves
// only a guide until it lands. The rail is always there, so nothing the lane
// does changes Agent Views' width but the pin.

/** Where every device keeps its Home (PRD home-device-rail D-03). */
const HOME_FOLDER = "~/hide";

export function WorkspaceScreen({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const view = useShellStore((s) => workspaceViewOf(s.rest));
  const shown = useUiStore((s) => s.laneShown);
  const expanded = useUiStore((s) => s.laneExpanded);
  const [body, setBody] = useState<HTMLElement | null>(null);
  const width = useWidth(body);
  const sizes = useMemo<LaneSizes>(
    () => ({
      agentMin: tokenPx("--size-agent-views-min"),
      laneMin: tokenPx("--size-lane-min"),
      laneIdeal: tokenPx("--size-lane-ideal"),
      treeMin: tokenPx("--size-tree-min"),
      treeIdeal: tokenPx("--size-tree-ideal"),
      divider: tokenPx("--spacing-sm"),
    }),
    [],
  );
  const opening = useShellStore((s) => (s.editor?.opening ?? []).some((row) => row.checkout_id === checkout?.id));
  const hasViews = (view?.layout?.display_count ?? 0) > 0 || opening;
  // The lane draws the window's 보기 slot while it is shown, else the Workspace's tool (PRD sidebar-task-lens D-50).
  const detail = useShellStore((s) => s.rest?.ui_state?.detail);
  const tool = laneToolOf(view, detail);
  const input = {
    tool,
    pinned: view?.pinned === true,
    tree: view?.tree === true,
    laneWidth: view?.lane_width ?? null,
    panelWidth: view?.panel_width ?? null,
    treeWidth: view?.tree_width ?? null,
    hasViews,
    shown,
    expanded,
    body: width,
    sizes,
  };
  const frame = laneFrame(input);
  // The files tool kept under another tool is laid out at its own width, so
  // its View areas measure what Files will show (an Open to the side's room).
  const files = tool === "files" ? frame : laneFrame({ ...input, tool: "files" });
  const up = laneUp(frame.mode);
  const checkoutId = checkout?.id ?? null;
  // The rail, the chords and the close shortcut read what is drawn.
  useLayoutEffect(() => {
    useUiStore.getState().setDrawnLane({ mode: frame.mode, tool });
  }, [frame.mode, tool]);
  // Off the Workspace screen nothing is drawn, so a chord there opens the lane.
  useLayoutEffect(() => () => useUiStore.getState().setDrawnLane({ mode: "closed", tool: null }), []);
  // A closed lane is not expanded when it opens again.
  useLayoutEffect(() => {
    if (tool === null) useUiStore.getState().setLaneExpanded(false);
  }, [tool]);
  // A lane put away or shown moves no element the page observes, so the
  // native pages in it are placed again in the same frame (B21).
  useLayoutEffect(() => noteShellLayer(), [frame.mode, tool]);
  // Agent Views under an expanded lane keeps its size and takes no keyboard,
  // so its keyboard target is retired before a chord can act on a pane out
  // of sight.
  const agentsCovered = frame.mode === "expanded";
  useLayoutEffect(() => {
    const owner = keyboardOwner();
    if (agentsCovered && (owner.kind === "pane" || owner.kind === "agent") && owner.workspace === checkoutId) noteKeyboardOwner({ kind: "none" });
  }, [agentsCovered, checkoutId]);
  // A lane that goes out of sight with the keyboard inside it, or a files
  // tool another tool replaces with the keyboard in its documents, hands the
  // keyboard to the pane the core has focused, as after a closing sheet, so
  // no focus is reported for it (B4, B23).
  const filesUp = up && tool === "files";
  const shownBefore = useRef({ up, filesUp });
  useEffect(() => {
    const before = shownBefore.current;
    shownBefore.current = { up, filesUp };
    const owner = keyboardOwner();
    const lost = (before.up && !up && (owner.kind === "view" || owner.kind === "tool")) || (before.filesUp && !filesUp && owner.kind === "view");
    if (!lost) return;
    const paneId = useShellStore.getState().focusedPaneId;
    if (paneId) focusTerminal(paneId);
  }, [up, filesUp]);
  if (!checkout || !view) return null;
  return (
    <section
      className="flex min-h-0 min-w-0 flex-1 overflow-hidden"
      aria-label={t("panes.workspace.aria", { name: checkout.branch ?? checkout.label })}
      data-workspace-screen={checkout.id}
      data-lane-mode={frame.mode}
      data-lane-shows={tool ?? "none"}
    >
      <LaneRow view={view} tool={tool} frame={frame} files={files} width={width} body={body} setBody={setBody} sizes={sizes} actions={actions} checkout={checkout} />
      <div className="contents" onKeyDown={(event) => laneEscape(event, frame, view.pinned)}>
        <ToolRail actions={actions} />
      </div>
    </section>
  );
}

/**
 * Escape in the lane or on its rail, once nothing inside answered (a menu, a
 * filter, a document's find), returns an expanded lane, or puts an unpinned
 * one away and gives the keyboard to the focused pane; Escape in a terminal
 * stays the program's (B4, B8).
 */
function laneEscape(event: React.KeyboardEvent<HTMLElement>, frame: LaneFrame, pinned: boolean): void {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  const ui = useUiStore.getState();
  if (frame.mode === "expanded") ui.setLaneExpanded(false);
  else if (frame.mode === "overlay" && !pinned) {
    ui.putLaneAway();
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
 * placed over the body's right edge, so moving between over, beside,
 * expanded and put away never mounts it again; a docked lane, and an
 * expanded one that was docked, keep their width beside Agent Views with a
 * divider and an empty slot under the lane, so Agent Views keeps the width
 * it has (B8).
 */
function LaneRow({ view, tool, frame, files, width, body, setBody, sizes, actions, checkout }: { view: WorkspaceView; tool: LaneTool | null; frame: LaneFrame; files: LaneFrame; width: number; body: HTMLElement | null; setBody: (element: HTMLElement | null) => void; sizes: LaneSizes; actions: Actions; checkout: Checkout }) {
  const { t } = useInterfaceTranslation();
  const [guide, setGuide] = useState<number | null>(null);
  // The width Agent Views gives up: a docked lane's, kept while it is expanded.
  const slot = frame.agents > 0 && frame.agents < width ? width - frame.agents - sizes.divider : 0;
  const up = laneUp(frame.mode);
  // A terminal click leaves the lane where it is, pinned or not: only the
  // operator closes it, or puts an unpinned one away with Escape (B4).
  const escape = (event: React.KeyboardEvent<HTMLElement>) => laneEscape(event, frame, view.pinned);
  const placed = frame.mode === "expanded" ? "inset-0 z-20" : "inset-y-0 right-0 z-20";
  const raised = frame.mode === "overlay" || frame.mode === "expanded" ? "border-l border-border shadow-lg" : "";
  return (
    <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1" data-lane-row="true">
      {/* Its own stacking context, so nothing the agents raise (a pane
          divider) draws over the lane; out of reach of the pointer and the
          keyboard while an expanded lane covers it. */}
      <div className="isolate flex min-h-0 min-w-0 flex-1 flex-col" inert={frame.mode === "expanded"} data-agent-area="true">
        <AgentArea checkout={checkout} actions={actions} />
      </div>
      {slot > 0 ? (
        <>
          <LaneDivider kind="lane" label={t("panes.workspace.resize", { name: t("panes.lane.aria") })} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} />
          <div className="shrink-0" style={{ width: slot }} aria-hidden="true" data-lane-slot="true" />
        </>
      ) : null}
      {tool !== null ? (
        <div
          className={cn("absolute flex min-h-0 min-w-0 flex-col bg-card", placed, raised)}
          style={frame.mode === "expanded" ? undefined : { width: frame.width }}
          inert={!up}
          onKeyDown={escape}
          data-lane-frame={frame.mode}
          data-lane-hidden={up ? undefined : "true"}
        >
          {/* A lane over the agents is sized from its own left edge; nothing beside it moves (B6). */}
          {frame.mode === "overlay" ? (
            <LaneDivider kind="lane" edge label={t("panes.workspace.resize", { name: t("panes.lane.aria") })} workspace={workspaceKey(view)} acknowledged={view.width_request_id ?? null} frame={frame} origin={body} sizes={sizes} setGuide={setGuide} actions={actions} />
          ) : null}
          <Lane view={view} tool={tool} frame={frame} files={files} sizes={sizes} checkout={checkout} actions={actions} />
        </div>
      ) : null}
      {guide !== null ? <DividerGuide left={guide} /> : null}
    </div>
  );
}

/**
 * The Workspace's part of the window's top row (PRD sidebar-titlebar-refresh
 * D-15, D-16, B17, B18; PRD right-lane B1): a remote device's band, the
 * `project / checkout` path, and at the right end, icons only, Overview and
 * Open server; the lane's tools are on the rail. The project opens its
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
    if (id === "copy_path") return void navigator.clipboard?.writeText(checkout.path).catch(() => undefined);
  };
  const home = project?.is_home === true;
  return (
    <div className="flex h-full min-w-0 flex-1 items-center gap-sm pr-sm text-caption" data-workspace-toolbar="true">
      {device ? (
        // The front thing is another machine's: its color and name lead the path, and the pane border wears the same color (PRD home-device-rail D-15).
        <span className="flex h-full max-w-(--size-recent-location-max) shrink-0 items-center gap-xs bg-device-remote px-sm font-medium text-primary-foreground" data-device-band={device.id}>
          <ServerIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 truncate">{deviceName}</span>
        </span>
      ) : null}
      <EntryContextMenu label={t("panes.workspace.aria", { name })} items={menuItems} onSelect={select} className={cn("flex min-w-0 flex-1 self-stretch items-center", !device && "pl-sm")} data-workspace-menu="true">
        <nav aria-label={t("panes.workspace.location")} className="flex min-w-0 items-center gap-xs">
          {project && !home ? (
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
          {/* Home is no project (B16): its path is the folder every device keeps it in (D-03).
              Not a drag handle: its hint and the path menu need the pointer (B18, B19). */}
          <Hint label={`${home ? HOME_FOLDER : name} · ${checkout.path}`}>
            <span className="min-w-0 truncate px-xs text-foreground" aria-current="page" data-workspace-location={home ? "home" : "checkout"} data-window-control="true">
              {home ? HOME_FOLDER : name}
            </span>
          </Hint>
        </nav>
      </EntryContextMenu>
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
  // The machine this window runs on, which "Show this machine" brings forward; on a node it is not the core's.
  const own = useShellStore((s) => windowDeviceId(s.rest?.navigator?.devices));
  const session = status?.session ?? null;
  const view = useMemo(() => remoteView(session), [session]);
  if (!device) return null;
  const connected = status?.state === "connected";
  const away = deviceAwayLine(device, name, t);
  if (!view) {
    const line = deviceLine(device, status ?? undefined, t);
    return (
      <div className="flex min-h-0 flex-1 flex-col items-start justify-center gap-sm p-xl text-body text-subtle-foreground" data-remote-device-surface={device.id} data-remote-state={status?.state ?? "none"}>
        <h2 className="text-title font-semibold text-foreground">
          {name} <span className="font-mono text-caption text-muted-foreground">{device.ssh_alias}</span>
        </h2>
        <p>{connected ? t("panes.workspace.connectedEmpty", { device: name }) : t("panes.workspace.remoteState", { device: name, state: line.text })}</p>
        {away.hint ? <p className="text-caption text-muted-foreground">{away.hint}</p> : null}
        <div className="flex gap-sm">
          {canRetryDevice(device, status ?? undefined) ? (
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
          {canRetryDevice(device, status ?? undefined) ? (
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
