import { useCenterReconnect } from "./centerReconnect";
import { deviceScope } from "./agentScope";
import type { TFunction } from "i18next";
import { useInterfaceTranslation } from "./i18n/client";
import { BriefcaseBusinessIcon, ChevronDownIcon, ChevronRightIcon, CornerUpLeftIcon, EllipsisIcon, FolderGit2Icon, FolderIcon, Loader2Icon, PanelsTopLeftIcon, PinIcon, PlusIcon, RefreshCwIcon, ServerIcon, SettingsIcon } from "lucide-react";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { drawnInTree, hideAiCheckoutOf, originLine } from "./hideAiOrigin";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { RowButton, RowOverlayButton } from "./components/ui/row-button";
import { EntryContextMenu, EntryDropdown } from "./components/entry-menu";
import { SidebarHeader, type DeviceMenu } from "./components/sidebar-header";
import { GithubIcon } from "./components/github-icon";
import { Hint } from "./components/ui/tooltip";
import { CHECKOUT_KIND_ICON } from "./components/checkout-icon";
import { PrMark } from "./components/pr-mark";
import { PR_LOOK, rowPulls } from "./prMark";
import { CheckoutCardHint, pullRequestOpenExternal } from "./components/pr-card";
import { DeviceRail } from "./components/device-rail";
import { frontDeviceId, frontTitle, railShown, readsHere, sidebarBody } from "./devices";
import { asksOperator, badgeWords, sumMarks } from "./agentRow";
import { Badge } from "./components/ui/badge";
import { cn } from "./lib/utils";
import { agentNumber, sidebarAgentNumbers } from "./numbering";
import { checkoutAgentRows, type BoardRow } from "./projectBoard";
import { FoldLane, SidebarAgentRow, type AgentRowMenu, type SidebarTreeSlot } from "./components/sidebar-agent-row";
import { SidebarMissionRow, useEveryDeviceMissionAsks } from "./components/sidebar-mission-row";
import { useMissionTurnCount } from "./MissionScreen";
import { TreeRails } from "./components/agent-tree";
import { siblingsShown, sidebarTreeLines } from "./sidebarTree";
import { moveSidebarKey, sidebarNamedGroups, sidebarTabGroups, type SidebarCollection, type SidebarTabGroup } from "./sidebarTabs";
import { groupFoldKey, useGroupFolds, useGroupName } from "./groupTabs";
import { tabSurface } from "./recent";
import { SleepingSessionRow } from "./components/sleeping-session-row";
import { StatusBadge } from "./components/status-badge";
import { Usage } from "./components/usage";
import { agentGroupTitle, agentPlaces, allAgents, elsewhereNeedsYou, type ListedAgent } from "./navigation";
import { allAgents as everyPaneAgent } from "./paneAgents";
import {
  activeCheckouts,
  checkoutCard,
  checkoutHasSecondLine,
  checkoutNameParts,
  checkoutPresentation,
  checkoutRowExpansion,
  projectExpanded,
  projectRowExpansion,
  projectCheckout,
  folderCheckout,
  inactiveCheckouts,
  projectRows,
  shownPullRequest,
  type CheckoutPresentation,
  type ProjectRow,
} from "./projects";
import { editorHost, revealHost } from "./host";
import { commandLabel } from "./shortcutLabels";
import type { Digit } from "./shortcuts";
import { contextAgents, contextWorkspaces, deviceCatalogLine, remoteContext, remoteView } from "./remote";
import { agentReference } from "./agentReference";
import { agentMenu, checkoutMenu, checkoutRemoving, FOLDER_CHECKOUT_ITEMS, folderMenu, primaryCheckout, projectMenu, type MenuHost, type MenuItem } from "./workspaceManage";
import { projectIssueSource, projectRepository, settingsTarget } from "./projectSettings";
import { machineName, windowDeviceId, windowFirst } from "./screenMachine";
import { focusedRemoteDevice, localDeviceId, type AgentRow, type Checkout, type HideAiCheckout, type SessionPull, type SleepingSession, type SnapshotRest, type Workspace } from "./snapshot";
import { useStartPanel } from "./startDraft";
import { PracticeInvite } from "./components/practice-invite";
import { HerdrImportLine } from "./HerdrImport";
import { SidebarMenus } from "./components/sidebar-menus";
import { useShellStore } from "./store";
import { draggedSidebarWidth, sidebarWidthToSend } from "./sidebarWidth";
import { useUiStore } from "./ui";
import { useOverviewCount } from "./Overview";
import { revealSidebarFocus, sidebarFocusPane, sidebarPanes } from "./sidebarFocus";
import { SidebarLensSwitch, TaskLens } from "./TaskLens";
import { DeskSidebarEntry } from "./DeskScreen";
import { SidebarPrs } from "./SidebarPrs";

function engineRowLabel(state: string | null, t: TFunction<"translation">): string | null {
  if (state === "unconfigured" || state === "socket_missing") return t("sidebar.herdrSocketMissing");
  if (state === "not_connected" || state === "unreachable" || state === "stale") {
    return t("sidebar.herdrNotResponding");
  }
  return null;
}

export function Sidebar({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const engineState = useShellStore((s) => s.engineState);
  const visible = useShellStore((s) => s.rest?.ui_state?.left_sidebar_visible ?? true);
  // The row speaks for this machine's engine, so it is not shown over a
  // selected SSH device's lists; that device's state is on the canvas.
  const remote = useShellStore((s) => focusedRemoteDevice(s.rest) !== null);
  const status = remote ? null : engineRowLabel(engineState, t);
  // The rail is shown unless the operator hid it (quick device-rail-badges B1, B6).
  const rail = useShellStore((s) => railShown(s.rest));
  const body = useShellStore((s) => sidebarBody(s.rest));
  const lens = useShellStore((s) => s.rest?.ui_state?.sidebar_lens ?? "projects");
  // The switch has no chord of its own until the operator binds one (issue 170).
  const overviewChord = useShellStore((s) => commandLabel("overview", s.rest?.ui_state));
  const overviewCount = useOverviewCount();
  const overviewSelected = useUiStore((s) => s.overviewOpen || s.screen?.kind === "main");
  const missionsSelected = useUiStore((s) => !s.overviewOpen && s.screen?.kind === "missions");
  const missionsCount = useMissionTurnCount();
  const missionsChord = useShellStore((s) => commandLabel("open_missions", s.rest?.ui_state));
  const storedWidth = useShellStore((s) => s.rest?.ui_state?.sidebar_width ?? null);
  const columns = useSidebarColumns();
  // A drag draws the nav alone at the width under the pointer, over the
  // center, so nothing beside it (a terminal above all) reflows per move; the
  // box beside the center takes the width once the core carries it.
  const [preview, setPreview] = useState<number | null>(null);
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setPreview(null);
  }, [storedWidth]);
  const focusRequest = useUiStore((s) => s.sidebarFocus);
  useEffect(() => {
    if (!visible || !focusRequest || body === "disconnected") return;
    const frame = requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(`[data-sidebar] [data-sidebar-content] [aria-current="page"], [data-sidebar] [data-sidebar-content] button`)?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, body, focusRequest]);
  if (!visible) return null;
  const land = (current: number, landed: number) => {
    const send = sidebarWidthToSend(storedWidth ?? current, landed);
    if (send === null) setPreview(null);
    else actions.setSidebarWidth(send);
  };
  // Every device's sidebar lists its Needs You, its Done and its projects (PRD sidebar-titlebar-refresh D-06: no Home row);
  // a device that is not connected shows its name and the way to reconnect, and no stale tree (quick device-rail-badges B3).
  // Under the head, the 프로젝트 | 태스크 | PR switch picks the list (PRD sidebar-task-lens B1, D-10;
  // lane-two-columns B41). The 태스크 list keeps an unresponsive device's last rows, dimmed, under its reason (B20).
  const list = lens === "tasks" ? <TaskLens actions={actions} /> : lens === "prs" ? <SidebarPrs actions={actions} /> : body === "disconnected" ? <DisconnectedDevice actions={actions} /> : <ProjectList actions={actions} />;

  return (
    // The width is a CSS variable, set here and on the shell for the top row
    // alike (`useSidebarColumns`), never the nav's own inline width, which the
    // e2e overflow check sets to try other widths (PRD sidebar-typography D-13).
    <div className="relative h-full w-(--sidebar-total) shrink-0" style={columns} data-sidebar-box="true">
      <nav
        className={cn("absolute inset-y-0 left-0 flex w-(--sidebar-total) bg-sidebar text-foreground", preview !== null && "z-30")}
        style={
          preview === null
            ? undefined
            : ({ "--sidebar-width": `${preview}px`, "--sidebar-total": "calc(var(--sidebar-width) + var(--sidebar-rail))" } as CSSProperties)
        }
        data-sidebar="projects"
        data-sidebar-rail={rail ? "device" : "none"}
      >
        {/* The rail is the sidebar's full-height left column, the device's header and lists beside it (quick device-rail-slack). */}
        {rail ? <DeviceRail actions={actions} /> : null}
        <div className="sidebar-header-container flex min-h-0 min-w-0 flex-1 flex-col">
          <SidebarHeader
            compact={(preview ?? storedWidth ?? tokenPx("--size-sidebar-ideal")) < tokenPx("--size-sidebar-ideal")}
            overview={{ selected: overviewSelected, count: overviewCount, chord: overviewChord || null, onOpen: () => actions.openOverviewEntry() }}
            missions={{ selected: missionsSelected, count: missionsCount, chord: missionsChord || null, onOpen: () => actions.openMissions() }}
            addProject={{ chord: commandLabel("new_workspace") || null, onOpen: () => actions.openAddProject() }}
            searchChord={commandLabel("search") || null}
            onSearch={() => actions.openSearch()}
          />
          <DeskSidebarEntry actions={actions} />
          {status ? (
            <div className="border-b border-border px-md py-sm text-caption text-muted-foreground">{status}</div>
          ) : null}
          <SidebarLensSwitch actions={actions} />
          <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-sidebar-content="true">
            {list}
            <HerdrImportLine actions={actions} />
            <PracticeInvite actions={actions} />
            <div data-sidebar-footer="true" className="flex shrink-0 flex-wrap items-center gap-xs border-t border-border px-sm py-xs">
              <Usage actions={actions} />
              {/* Usage holds the left edge; with the rail hidden the menus keep the right one, also on a wrapped line. */}
              {rail ? null : (
                <div className="ml-auto flex items-center gap-xs">
                  <SidebarMenus actions={actions} place="footer" />
                </div>
              )}
            </div>
          </div>
        </div>
        <SidebarEdge
          onBegin={(start) => {
            dragging.current = true;
            setPreview(start);
          }}
          onPreview={setPreview}
          onLand={(start, landed) => {
            dragging.current = false;
            land(start, landed);
          }}
          onCancel={() => {
            dragging.current = false;
            setPreview(null);
          }}
          onReset={(current) => land(current, tokenPx("--size-sidebar-ideal"))}
        />
      </nav>
    </div>
  );
}

/**
 * The sidebar's column widths as CSS variables, set once on the shell so the
 * window's top row and the sidebar under it share one edge: the stored width
 * is the content column's, and the rail is an extra fixed column to its left
 * only while it shows (PRD home-device-rail D-09).
 */
export function useSidebarColumns(): CSSProperties {
  const storedWidth = useShellStore((s) => s.rest?.ui_state?.sidebar_width ?? null);
  const rail = useShellStore((s) => railShown(s.rest));
  return useMemo(
    () =>
      ({
        "--sidebar-width": storedWidth === null ? "var(--size-sidebar-ideal)" : `${storedWidth}px`,
        "--sidebar-rail": rail ? "var(--size-rail)" : "var(--spacing-none)",
        "--sidebar-total": "calc(var(--sidebar-width) + var(--sidebar-rail))",
      }) as CSSProperties,
    [storedWidth, rail],
  );
}

/** The device in front as the top row names it, and the menu its name opens while the rail is hidden. */
export function useDeviceTitle(actions: Actions): { title: { name: string; note: string | null }; deviceMenu: DeviceMenu } {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const frontId = useShellStore((s) => s.rest?.navigator?.focused_device_id);
  const remoteStatus = useShellStore((s) => s.rest?.status?.remote);
  const title = useMemo(() => frontTitle(devices, frontId, t), [devices, frontId, t]);
  const deviceMenu = useMemo<DeviceMenu>(() => {
    const list = windowFirst(devices);
    const localId = list.find((device) => device.kind !== "remote")?.id ?? "";
    const own = windowDeviceId(devices);
    return {
      devices: list.map((device) => ({
        id: device.id,
        label: machineName(devices, device.id, t),
        remote: device.id !== own,
        connected: device.id === localId || remoteStatus?.find((row) => row.target_id === device.id)?.state === "connected",
      })),
      frontId: frontId ?? localId,
      onSelect: actions.focusDevice,
      onAddDevice: actions.openAddDevice,
      onShowRail: actions.showDeviceRail,
    };
  }, [devices, frontId, remoteStatus, actions, t]);
  return { title, deviceMenu };
}

function tokenPx(name: string): number {
  const value = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name));
  if (!Number.isFinite(value)) throw new Error(`token ${name} is not a length`);
  return value;
}

/**
 * The sidebar's right edge (PRD sidebar-typography B10-B12): the pane
 * divider's part, a `--size-resize-grab` hit area over the edge and a
 * `--size-resize-handle` line in the accent under the pointer and while
 * dragging. The edge follows the pointer between `--size-sidebar-min` and
 * `--size-sidebar-max` and lands once on release; a double-click returns it
 * to `--size-sidebar-ideal`. Widths are measured on the content column (the nav less the rail), so the start is
 * what the operator sees.
 */
function SidebarEdge({
  onBegin,
  onPreview,
  onLand,
  onCancel,
  onReset,
}: {
  onBegin: (start: number) => void;
  onPreview: (width: number) => void;
  onLand: (start: number, landed: number) => void;
  onCancel: () => void;
  onReset: (current: number) => void;
}) {
  const { t } = useInterfaceTranslation();
  const drag = useRef<{ start: number; x: number; landed: number } | null>(null);
  const [active, setActive] = useState(false);
  // The width the operator sets is the content column's: the nav less the rail column while one shows.
  const navWidth = (edge: HTMLElement) => {
    const nav = edge.parentElement!;
    return Math.round(nav.getBoundingClientRect().width - (nav.querySelector("[data-device-rail]")?.getBoundingClientRect().width ?? 0));
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("sidebar.resize")}
      data-sidebar-edge="true"
      className="group/edge absolute inset-y-0 z-30 flex w-(--size-resize-grab) cursor-col-resize justify-center"
      style={{ right: "calc(var(--size-resize-grab) / -2)" }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        const start = navWidth(event.currentTarget);
        drag.current = { start, x: event.clientX, landed: start };
        setActive(true);
        onBegin(start);
      }}
      onPointerMove={(event) => {
        const current = drag.current;
        if (!current) return;
        current.landed = draggedSidebarWidth(current.start, event.clientX - current.x, {
          min: tokenPx("--size-sidebar-min"),
          max: tokenPx("--size-sidebar-max"),
        });
        onPreview(current.landed);
      }}
      onPointerUp={(event) => {
        const current = drag.current;
        if (!current) return;
        event.currentTarget.releasePointerCapture(event.pointerId);
        drag.current = null;
        setActive(false);
        onLand(current.start, current.landed);
      }}
      onPointerCancel={() => {
        drag.current = null;
        setActive(false);
        onCancel();
      }}
      onDoubleClick={(event) => onReset(navWidth(event.currentTarget))}
    >
      <span aria-hidden="true" className={cn("h-full w-(--size-resize-handle) bg-primary opacity-0 group-hover/edge:opacity-100", active && "opacity-100")} />
    </div>
  );
}

/**
 * The pane whose row stands for the focused pane while a Workspace is in front
 * (`sidebarFocusPane` over every row the list can draw), or null.
 */
function useFocusRowPane(): string | null {
  const workspaceScreen = useUiStore((s) => s.screen?.kind === "workspace" && !s.overviewOpen);
  const focusedPaneId = useShellStore((s) => s.focusedPaneId);
  const workspaces = useShellStore((s) => contextWorkspaces(s.rest));
  const agents = useShellStore((s) => contextAgents(s.rest, s.agents));
  const scope = useShellStore((s) => deviceScope(s.rest, frontDeviceId(s.rest)));
  const drawn = useMemo(() => sidebarPanes(workspaces, scope), [workspaces, scope]);
  return useMemo(() => (workspaceScreen ? sidebarFocusPane(focusedPaneId, agents, drawn) : null), [workspaceScreen, focusedPaneId, agents, drawn]);
}

/** A selected device the core cannot read: its name, why, and the way to try again unless the center already offers it, with the last tree left out (PRD home-device-rail B8, B9). */
function DisconnectedDevice({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const deviceId = useShellStore((s) => frontDeviceId(s.rest));
  const label = useShellStore((s) => machineName(s.rest?.navigator?.devices, frontDeviceId(s.rest), t));
  // The center's own reconnect for this device is the one; a second here would be the same action twice.
  const centerOffers = useCenterReconnect((s) => (s.shown[deviceId] ?? 0) > 0);
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-sm px-md text-center" data-device-disconnected={deviceId}>
      <ServerIcon aria-hidden="true" className="size-(--size-icon-lg) text-subtle-foreground" />
      <span className="max-w-full truncate text-subhead font-semibold text-foreground" data-device-disconnected-name="true">
        {label}
      </span>
      <span role="status" className="text-caption text-muted-foreground">
        {t("devices.state.unavailable")}
      </span>
      {centerOffers ? null : (
        <Button variant="secondary" size="sm" data-device-reconnect={deviceId} onClick={() => actions.retryDevice(deviceId)}>
          <RefreshCwIcon aria-hidden="true" />
          {t("sidebar.reconnect")}
        </Button>
      )}
    </div>
  );
}

const NO_SLEEPERS: SleepingSession[] = [];

/** How many named tabs of the checkout hold no agent row, each still drawn as its tab line when the list opens. */
function useEmptyNamedTabs(checkout: Checkout, deviceId: string, agentRows: BoardRow[], agents: readonly AgentRow[]): number {
  return useMemo(() => sidebarTabGroups(checkout, deviceId, agentRows, agents).filter((group) => group.rows.length === 0).length, [checkout, deviceId, agentRows, agents]);
}

function useSleepingCheckout(path: string, deviceId: string) {
  const all = useShellStore((s) => s.rest?.navigator?.sleeping_sessions ?? NO_SLEEPERS);
  return useMemo(() => all.filter((session) => session.node_id === deviceId && session.checkout_path === path), [all, path, deviceId]);
}

/** The first snapshot has not arrived: neither an empty list nor a zero is known yet. */
function ListLoading() {
  const { t } = useInterfaceTranslation();
  return (
    <div role="status" className="min-h-0 flex-1 px-md py-sm text-caption text-muted-foreground" data-sidebar-loading="true">
      {t("devices.state.connecting")}
    </div>
  );
}

function catalogLineOf(rest: SnapshotRest | null, t: TFunction<"translation">) {
  const context = remoteContext(rest);
  return context ? deviceCatalogLine(context, rest?.navigator?.devices, t) : null;
}

/**
 * The raised sections, held above the list, and the scope picker below them: the projects of the context on
 * screen. A selected SSH device lists its engine workspaces, one checkout
 * each; the inactive folds and pins are this machine's and are not drawn
 * there (docs/UI_BEHAVIOR.md: the remote context carries no pins). The row of
 * the scope the center shows is marked: a project on its Overview, or the
 * focused checkout and its agent while a Workspace is in front.
 */
function ProjectList({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const loaded = useShellStore((s) => s.rest !== null);
  const remote = useShellStore((s) => focusedRemoteDevice(s.rest) !== null);
  const workspaces = useShellStore((s) => contextWorkspaces(s.rest));
  // The device folds are this machine's, like pins: a selected SSH device's tree has none.
  const inactiveFold = useShellStore((s) => (remote ? undefined : s.rest?.navigator?.inactive_projects));
  const focusedCheckoutId = useShellStore((s) =>
    remote ? (remoteView(remoteContext(s.rest)?.session ?? null, s.rest?.workspace_view)?.checkout.id ?? null) : (s.rest?.navigator?.focused_checkout_id ?? null),
  );
  const localWorkspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remoteStatuses = useShellStore((s) => s.rest?.status?.remote);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const localAgents = useShellStore((s) => s.agents);
  const agentScope = useShellStore((s) => s.rest?.navigator?.agent_scope);
  const listedAgents = useMemo(() => allAgents(remoteStatuses, localAgents, agentScope, devices), [remoteStatuses, localAgents, agentScope, devices]);
  const agents = useMemo(() => listedAgents.map((row) => row.agent), [listedAgents]);
  const frontId = useShellStore((s) => frontDeviceId(s.rest));
  const frontScope = useShellStore((s) => deviceScope(s.rest, frontId));
  const openCheckouts = useShellStore((s) => s.rest?.ui_state?.session_collapsed_checkout_ids ?? NO_IDS);
  const focusedPaneId = useShellStore((s) => s.focusedPaneId);
  const focusRowPane = useFocusRowPane();
  const workspaceVisible = useUiStore((s) => s.screen?.kind === "workspace" && !s.overviewOpen);
  const focusedProjectId = workspaceVisible ? (workspaces.find((workspace) => workspace.checkouts.some((checkout) => checkout.id === focusedCheckoutId))?.id ?? null) : null;
  const lists = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const revealed = useRef<string | null>(null);
  // The focus's row is brought into view when the focus moves, before the frame
  // is painted, and never while the focus stays where it was revealed, through
  // a snapshot or another screen in front, so a list the operator scrolled
  // stays where they left it.
  useLayoutEffect(() => {
    if (!workspaceVisible || !lists.current) return;
    const focus = `${focusedCheckoutId ?? ""}\u0000${focusedPaneId ?? ""}`;
    if (focus === revealed.current) return;
    revealed.current = focus;
    revealSidebarFocus(lists.current);
  }, [loaded, workspaceVisible, focusedCheckoutId, focusedPaneId]);
  const catalogState = useShellStore((s) => catalogLineOf(s.rest, t)?.state ?? null);
  const catalogText = useShellStore((s) => catalogLineOf(s.rest, t)?.text ?? null);
  const catalogLine = catalogState && catalogText ? { state: catalogState, text: catalogText } : null;
  const raisedOpen = useUiStore((s) => s.raisedOpen);
  // Needs You lists every device's rows, the front device's first; the rest of the list is the front device's.
  const asks = useEveryDeviceMissionAsks(frontId);
  // It reads the devices and their connections alone, so it is read again only when those move.
  const elsewhere = useMemo(() => elsewhereNeedsYou(useShellStore.getState().rest, localAgents, frontId), [devices, remoteStatuses, localAgents, frontId]);
  const rows = useMemo(
    () => projectRows(workspaces, { inactive_projects: inactiveFold }, listedAgents, frontScope, raisedOpen, asks, elsewhere),
    [workspaces, inactiveFold, listedAgents, frontScope, raisedOpen, asks, elsewhere],
  );
  // The device's Needs You and Done come first, then the projects in their sections; Working and Seen are the Agents tab's.
  const raised = useMemo(() => rows.filter((row) => row.kind === "raised"), [rows]);
  const listRows = useMemo(() => rows.filter((row) => row.kind !== "raised"), [rows]);
  const agentRowMenu = useAgentRowMenu(actions);
  const children = useMemo(() => new Map(agents.map((agent) => [agent.pane_id, directChildren(agent, agents)])), [agents]);
  // A row Hide AI started names its task in its place line, so the place reads the agents too.
  const hideAiOf = useMemo(() => new Map(agents.flatMap((agent) => (agent.hide_ai ? [[agent.pane_id, agent.hide_ai] as const] : []))), [agents]);
  const placeOf = useMemo(() => agentPlaces(localWorkspaces, remoteStatuses, devices, hideAiOf), [localWorkspaces, remoteStatuses, devices, hideAiOf, t]);
  // An ⌥ hold shows each agent's number once, on its raised row or else its
  // own checkout's row; ⌥n selects by the Agents list, so the number is that list's.
  const numbered = useUiStore((s) => s.hint === "agents");
  const groupClosed = useGroupFolds((state) => state.closed);
  const numberOfPane = useMemo(
    () => (numbered ? sidebarAgentNumbers(useShellStore.getState(), raisedOpen, groupClosed) : null),
    [numbered, frontScope, rows, openCheckouts, workspaces, raisedOpen, groupClosed],
  );
  // The folds are this machine's choices, like the inactive groups, so a
  // selected SSH device's tree is drawn open with every checkout's line two.
  const treeAgents = useMemo(() => everyPaneAgent(localAgents, remoteStatuses), [localAgents, remoteStatuses]);
  const context: ListContext = {
    agents,
    treeAgents,
    childrenOf: (agent) => children.get(agent.pane_id)!,
    familyOf: (agent) => (agent.state.root ? rowPulls(agent) : undefined),
    placeOf,
    numberOf: numberOfPane,
    focusedCheckoutId,
    focusedProjectId,
    focusRowPane,
    workspaceScreen: workspaceVisible,
    openCheckouts,
    disclosure: true,
    actions,
    agentRowMenu,
  };
  if (!loaded) return <ListLoading />;
  return (
    // The arrow keys move through both lists as one.
    <div ref={lists} className="flex min-h-0 flex-1 flex-col" onKeyDown={moveSidebarKey}>
      {/* One scrolling list: Needs You and Done are its first sections, each head
          sticking at the top and stacking under the heads above it while the
          rows pass under (as the 태스크 list's do), and each folds to its head. */}
      <ul ref={list} className="min-h-0 flex-1 overflow-auto px-sm" style={{ scrollPaddingTop: `calc(var(--size-sidebar-section-header) * ${raised.length})` }} data-project-list="true">
        {catalogLine ? (
          <li role="status" className="px-md py-xs text-caption text-muted-foreground" data-device-catalog={catalogLine.state}>
            {catalogLine.text}
          </li>
        ) : null}
        {raised.map((row, stack) => (
          <ProjectRowView key={rowKey(row)} row={row} stack={stack} context={context} />
        ))}
        {raised.length > 0 && listRows.length > 0 ? <li role="separator" className="my-xs border-t border-border" /> : null}
        {listRows.map((row, index) => (
          <ProjectRowView key={rowKey(row)} row={row} first={index === 0 || listRows[index - 1]?.kind === "divider"} context={context} />
        ))}
        <CleanupRows workspaces={workspaces} context={context} deviceId={frontId} />
        {!remote && workspaces.length === 0 ? (
          <li className="flex flex-col items-start gap-sm px-sm py-sm text-caption text-muted-foreground" data-projects-empty="true">
            {t("sidebar.noProjects")}
          </li>
        ) : null}
      </ul>
    </div>
  );
}

const NO_IDS: string[] = [];

/** What every row of the Projects list reads besides its own project. */
/** What an opened tree reads from its list: a checkout's tree and Home's alike. */
type TreeContext = Pick<ListContext, "agents" | "childrenOf" | "familyOf" | "numberOf" | "focusRowPane" | "actions" | "agentRowMenu">;

type ListContext = {
  agents: AgentRow[];
  /** Every device's rows: a delegation crosses machines, so the core's sidebar trees name rows of other devices (`allAgents`, `paneAgents.ts`). */
  treeAgents: AgentRow[];
  childrenOf: (agent: AgentRow) => AgentRow[];
  /** A root's own and every descendant's PRs, undefined for a child or a root without children. */
  /** A root's whole PR list, as the 태스크 row draws it (PRD project-tree-pr-family D-01, D-02); a child row keeps its own. */
  familyOf: (agent: AgentRow) => SessionPull[] | undefined;
  /** The Agents list's context line for a raised row, which no project row above names. */
  placeOf: (device: string | null, paneId: string) => string | null;
  /** The number an ⌥ hold shows on an agent's raised row (checkout null) or tree row, while one shows. */
  numberOf: ((paneId: string, checkoutId: string | null) => Digit | null) | null;
  focusedCheckoutId: string | null;
  /** The project holding the focused checkout while a Workspace is in front, or null. */
  focusedProjectId: string | null;
  /** The pane whose row stands for the focused pane (`useFocusRowPane`). */
  focusRowPane: string | null;
  /** A Workspace is in front, so the focused checkout and agent are the scope shown. */
  workspaceScreen: boolean;
  /** The project whose Overview is in front. */
  /** Checkouts whose agent rows the operator opened; every other checkout names its agents on line two. */
  openCheckouts: string[];
  /** False over a selected SSH device, whose tree is drawn with nothing folded. */
  disclosure: boolean;
  actions: Actions;
  agentRowMenu: AgentRowMenu;
};

function rowKey(row: ProjectRow): string {
  switch (row.kind) {
    case "divider":
      return "divider";
    case "raised":
      return `raised:${row.group}`;
    case "workspace":
      return `workspace:${row.workspace.id}`;
  }
}


const ProjectRowView = memo(function ProjectRowView({ row, first = false, stack = 0, context }: { row: ProjectRow; first?: boolean; stack?: number; context: ListContext }) {
  if (row.kind === "divider") return <li role="separator" className="my-sm border-t border-border" />;
  if (row.kind === "raised") return <RaisedSection row={row} stack={stack} context={context} />;
  return <WorkspaceRows workspace={row.workspace} quiet={row.quiet === true} first={first} context={context} />;
});

/**
 * A raised Needs You or Done section above the project tree
 * (docs/status-model.md): the Agents list's row for each agent, drawn
 * whatever its project, checkout or parent has folded. A raised row never
 * unfolds, so it always wears its descendant badge; opening it does what the
 * same row does in Agents. Past its cap the older rows wait behind a fold
 * row, as an inactive fold's do.
 */
function RaisedSection({ row, stack, context }: { row: Extract<ProjectRow, { kind: "raised" }>; stack: number; context: ListContext }) {
  const { t } = useInterfaceTranslation();
  const toggleRaised = useUiStore((s) => s.toggleRaised);
  const folded = useUiStore((s) => s.raisedFolded.includes(row.group));
  const toggleFolded = useUiStore((s) => s.toggleRaisedFolded);
  const Chevron = folded ? ChevronRightIcon : ChevronDownIcon;
  const name = agentGroupTitle(row.group, t);
  const count = row.missions.length + row.agents.length + row.more.length;
  // Pressed while stuck at the top it goes back to its rows; otherwise it folds or unfolds them.
  const press = (event: MouseEvent<HTMLElement>) => {
    const head = event.currentTarget.parentElement!;
    const scroller = head.closest<HTMLElement>("[data-project-list]");
    const stuck = scroller !== null && scroller.scrollTop > 0 && head.getBoundingClientRect().top - scroller.getBoundingClientRect().top <= stack * head.offsetHeight + 1;
    if (stuck) scroller.scrollTo({ top: 0 });
    else toggleFolded(row.group);
  };
  return (
    <li data-raised-group={row.group} className="contents">
      <div
        data-section={row.group}
        data-section-sticky="true"
        id={`raised-group-${row.group}`}
        className="sticky z-20 h-(--size-sidebar-section-header) bg-sidebar"
        style={{ top: `calc(var(--size-sidebar-section-header) * ${stack})` }}
      >
        <RowButton
          aria-expanded={!folded}
          aria-label={`${name} ${count}`}
          data-raised-fold={row.group}
          className={cn("h-full gap-xs rounded-xs pl-sm text-micro font-medium outline-none focus-visible:ring-1 focus-visible:ring-ring", row.group === "needs_you" ? "text-warning" : "text-subtle-foreground")}
          onClick={press}
        >
          <span>{name}</span>
          <span className="flex-1 tabular-nums">{count}</span>
          <span className="flex w-(--size-lineage-chevron) shrink-0 justify-center text-muted-foreground">
            <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
          </span>
        </RowButton>
      </div>
      {folded ? null : (
        <ul aria-labelledby={`raised-group-${row.group}`}>
          {row.missions.map((ask) => (
            <SidebarMissionRow key={`mission:${ask.mission.id}`} ask={ask} inset={PROJECT_COLUMN} actions={context.actions} />
          ))}
          {row.agents.map((listed) => (
            <RaisedAgentRow key={`${listed.device ?? ""}:${listed.agent.id}`} listed={listed} context={context} />
          ))}
          {row.more.length > 0 ? (
            <li>
              <FoldRow
                label={t("sidebar.raisedMore")}
                count={row.more.length}
                expanded={row.expanded}
                onToggle={() => toggleRaised(row.group)}
                data-raised-more={row.group}
              />
            </li>
          ) : null}
          {row.expanded
            ? row.more.map((listed) => <RaisedAgentRow key={`${listed.device ?? ""}:${listed.agent.id}`} listed={listed} context={context} />)
            : null}
        </ul>
      )}
    </li>
  );
}

function RaisedAgentRow({ listed: { agent, device, chip }, context }: { listed: ListedAgent; context: ListContext }) {
  return (
    <SidebarAgentRow
      agent={agent}
      // The device in front is the whole list, so no row of its own names it (PRD home-device-rail B2); another device's Needs You row does.
      device={chip ?? null}
      place={context.placeOf(device, agent.pane_id)}
      placeInHead
      depth={0}
      ask={agent.state.needs_you}
      selected={agent.pane_id === context.focusRowPane}
      onOpen={(id) => context.actions.openListedAgent(id)}
      onOpenPullRequest={(url, external) => context.actions.openLink(url, external)}
      inset={PROJECT_COLUMN}
      number={context.numberOf?.(agent.pane_id, null) ?? null}
      menu={context.agentRowMenu}
      pulls={context.familyOf(agent)}
      onOpenMission={(id) => context.actions.openMission(id)}
      actions={context.actions}
    />
  );
}

// The Projects columns (PRD sidebar-readability D-2, sidebar-quiet-mark-indent
// D-06): a project with checkouts draws no glyph, so its name starts at the
// row's inset, a checkout's glyph stands under it, and an opened checkout's
// agent marks on the checkout name's column. A plain folder's row keeps its
// glyph, which says the checkout's kind (D-07). Every row ends the
// same way, agent rows included: its time, then its fold slot, so the times
// stand in one column and checkout chevrons in another; an agent root's
// chevron stands at its left, on its tab's guide. A row's menu opens on a
// right-click, or the menu key or ⇧F10 on the focused row; nothing on the
// row stands for it.
/** Where a project's name, a checkout's kind glyph, a plain folder's glyph, a group header's chevron and every fold start. */
const PROJECT_COLUMN = "var(--spacing-sm)";
/** Where a plain folder's name and its agent marks start. */
const PROJECT_NAME_COLUMN = "calc(var(--spacing-sm) + var(--size-checkout-icon) + var(--spacing-sm))";
/** Where a checkout's name, its line two and its agents' marks start. */
const CHECKOUT_NAME_COLUMN = `calc(${PROJECT_COLUMN} + var(--size-checkout-icon) + var(--spacing-xs))`;
/** Where a group header's briefcase stands, one step past the chevron it shares with the checkout's glyph column; its rows' guide hangs under it. */
const GROUP_GLYPH_COLUMN = `calc(${PROJECT_COLUMN} + var(--size-icon-sm) + var(--spacing-xs))`;

/**
 * A row control that waits for the pointer, inside a row `EntryContextMenu`
 * wraps: its slot is always kept, and it shows under the pointer, with focus
 * inside the row, while the row's menu is open, and always on an input with no
 * hover (PRD sidebar-readability D-3, B2, B3). `REVEALED_CONTROL` in
 * sidebar-agent-row.tsx is the same rule for an agent row, which has no menu.
 */
const ROW_REVEALED =
  "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 group-data-[state=open]:opacity-100 focus-visible:opacity-100 hoverless:opacity-100";

/** A row's time and fold slot, a step apart as on an agent row, so every row's time ends on one column. */
function RowEnd({ children }: { children: ReactNode }) {
  return <span className="relative flex shrink-0 items-center gap-xs self-stretch">{children}</span>;
}

/**
 * One disclosure for both inactive folds, the project level's and the checkout level's; its chevron stands in the fold slot.
 * It is `current` while it hides the focused checkout, and carries the selection for it.
 */
function FoldRow({
  label,
  count,
  expanded,
  current = false,
  badge,
  onToggle,
  ...data
}: {
  label: string;
  count: number;
  expanded: boolean;
  current?: boolean;
  /** The marks of what the fold holds (a `StatusBadge`), so a demand inside a fold is seen without opening it. */
  badge?: ReactNode;
  onToggle: () => void;
} & Record<`data-${string}`, string>) {
  const Chevron = expanded ? ChevronDownIcon : ChevronRightIcon;
  return (
    <RowButton
      aria-expanded={expanded}
      aria-current={current ? "true" : undefined}
      className={cn("min-h-(--size-control-lg) gap-sm rounded-sm py-0 text-caption hover:text-foreground", current ? "bg-secondary text-foreground hover:bg-secondary" : "text-muted-foreground")}
      style={{ paddingLeft: PROJECT_COLUMN }}
      onClick={onToggle}
      {...data}
    >
      <span className="min-w-0 flex-1 truncate">
        {label} {count}
      </span>
      {badge}
      <span className="flex w-(--size-lineage-chevron) shrink-0 justify-center text-muted-foreground">
        <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
      </span>
    </RowButton>
  );
}

/** The fold on a project or checkout row: a folded one is always shown, an open one waits for the pointer (D-3). */
function FoldToggle({ open, label, onToggle, disabled, quiet = false, ...data }: { open: boolean; label: string; onToggle: () => void; disabled?: boolean; /** Waits for the pointer open or folded. */ quiet?: boolean } & Record<`data-${string}`, string>) {
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <Button
      variant="text"
      size="inline"
      disabled={disabled}
      aria-expanded={open}
      aria-label={label}
      className={cn("pointer-events-auto relative h-(--size-icon-button-toolbar) w-(--size-lineage-chevron)", (open || quiet) && ROW_REVEALED)}
      onClick={onToggle}
      {...data}
    >
      <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
    </Button>
  );
}

/**
 * A project's name: the block's quiet label at 11/600 in the muted colour,
 * brightened while one of its awake agents draws a mark, or - where a folder
 * project and its only checkout are one row (`label` is false) - that row's own
 * name at 12, as a checkout's is. `hidden` keeps it from a screen reader where
 * the row button already names the project.
 */
function ProjectName({ workspace, hidden = false, grow = true, label = false }: { workspace: Workspace; hidden?: boolean; grow?: boolean; label?: boolean }) {
  const working = workspace.agent_presence?.working === true;
  return (
    <span
      aria-hidden={hidden ? "true" : undefined}
      data-row-name="true"
      data-project-weight={working ? "working" : "quiet"}
      className={cn(
        "min-w-0 truncate",
        grow && "flex-1",
        label
          ? "text-body font-semibold text-foreground"
          : cn("text-subhead text-foreground", working ? "font-semibold" : "font-normal"),
      )}
    >
      {workspace.label}
    </span>
  );
}

/**
 * A project or folder row's `+` and `⋯` (PRD sidebar-titlebar-refresh D-04,
 * B6-B8): `+` opens the start panel aimed at the project's main checkout, `⋯`
 * the row's own menu under the button. They wait for the pointer or the
 * keyboard, as the row's other revealed controls do, and stay while their
 * menu is open; they take no width at rest, and the badge of a folded project
 * stands aside while they show, so the row's end never holds both. They wait
 * at opacity 0, never `display: none`: a hidden button cannot take focus, so
 * the keyboard could not reach them from outside the row and a closing
 * Project settings could not hand focus back to the gear that opened it.
 */
function RowActions({ workspace, items, onSelect, fill, children }: { workspace: Workspace; items: () => MenuItem[]; onSelect: (id: MenuItem["id"]) => void; /** The `from-*` classes of the row's own fill, which the fade starts from; the row's own fill is what the buttons cover the name's end with. */ fill: string; children?: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [menuOpen, setMenuOpen] = useState(false);
  const more = useRef<HTMLButtonElement>(null);
  return (
    <span className="pointer-events-none absolute inset-y-0 right-full flex items-center justify-end">
      <span className={cn("pointer-events-none relative flex h-full items-center gap-xxs bg-linear-to-l from-70% to-transparent pl-md [&_button]:pointer-events-auto", fill, menuOpen ? "opacity-100" : ROW_REVEALED)} data-row-actions={workspace.id}>
        <Hint label={t(workspace.pinned ? "workspace.menu.unpin" : "workspace.menu.pin")}>
          <Button variant="row-action" size="icon-sm" aria-label={t(workspace.pinned ? "workspace.menu.unpin" : "workspace.menu.pin")} aria-pressed={workspace.pinned} data-row-pin={workspace.id} onClick={() => onSelect(workspace.pinned ? "unpin" : "pin")}>
            <PinIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", workspace.pinned && "text-foreground")} />
          </Button>
        </Hint>
        <Hint label={t("sidebar.startAgentIn", { name: workspace.label })}>
          <Button
            variant="row-action"
            size="icon-sm"
            aria-label={t("sidebar.startAgentIn", { name: workspace.label })}
            data-row-start={workspace.id}
            onClick={() => useStartPanel.getState().open(false, { projectId: workspace.id })}
          >
            <PlusIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </Button>
        </Hint>
        <Hint label={t("projectSettings.open", { project: workspace.label })}>
          <Button variant="row-action" size="icon-sm" aria-label={t("projectSettings.open", { project: workspace.label })} data-row-settings={workspace.id} onClick={() => useUiStore.getState().openProjectSettings(settingsTarget(workspace))}>
            <SettingsIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </Button>
        </Hint>
        <EntryDropdown
          label={t("common.entryActions", { name: workspace.label })}
          items={items}
          onSelect={(id) => id === "project_settings" || id === "rename_project" ? useUiStore.getState().openProjectSettings(settingsTarget(workspace), more.current, id === "rename_project" ? "name" : undefined) : onSelect(id)}
          align="start"
          hint={t("sidebar.moreActionsFor", { name: workspace.label })}
          onOpenChange={setMenuOpen}
          data-row-more-menu={workspace.id}
          trigger={
            <Button ref={more} variant="row-action" size="icon-sm" aria-label={t("sidebar.moreActionsFor", { name: workspace.label })} data-row-more={workspace.id}>
              <EllipsisIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </Button>
          }
        />
        {children}
      </span>
    </span>
  );
}

/**
 * A GitHub project's mark at its row's end: the repository's name in a tooltip,
 * and a press opens it on GitHub (⌘ asks for the default browser, as on a
 * pull request's mark) without selecting the project. It sits outside the
 * hover-swapped `+` and `⋯` so it stays pressable while they show. A folder,
 * or a project no GitHub repository is known for, draws nothing.
 */
function RepositoryMark({ workspace, actions }: { workspace: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const repository = useShellStore((s) => projectRepository(s.rest, workspace));
  if (!repository) return null;
  const label = t("sidebar.openRepository", { repository });
  return (
    <Hint label={repository}>
      <Button
        variant="row-action"
        size="icon-sm"
        aria-label={label}
        data-project-github={workspace.id}
        onClick={(event) => actions.openRepository(repository, workspace.device_id, pullRequestOpenExternal(event))}
      >
        <GithubIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      </Button>
    </Hint>
  );
}

function WorkspaceRows({ workspace, quiet = false, first = false, context }: { workspace: Workspace; quiet?: boolean; first?: boolean; context: ListContext }) {
  const { t } = useInterfaceTranslation();
  const { treeAgents, actions, disclosure } = context;
  const active = activeCheckouts(workspace);
  const inactive = inactiveCheckouts(workspace);
  const rowsByCheckout = useMemo(() => checkoutAgentRows(workspace, treeAgents, "sidebar"), [workspace, treeAgents]);
  const hideAiListed = useShellStore((s) => s.rest?.navigator?.hide_ai_checkouts);
  const issueSource = useShellStore((s) => projectIssueSource(s.rest, workspace));
  const expanded = !disclosure || projectExpanded(workspace);
  const folder = folderCheckout(workspace);
  if (folder) {
    if (workspace.session_folds?.cleanup.includes(folder.id)) return null;
    return (
      <FolderRowView
        workspace={workspace}
        checkout={folder}
        agentRows={rowsByCheckout.get(folder.id) ?? NO_BOARD_ROWS}
        focused={context.workspaceScreen && folder.id === context.focusedCheckoutId}
        first={first}
        context={context}
      />
    );
  }
  if (workspace.checkouts.length > 0 && workspace.checkouts.every((checkout) => workspace.session_folds?.cleanup.includes(checkout.id))) return null;
  const target = projectCheckout(workspace, useShellStore.getState().rest?.ui_state?.recent_checkouts);
  const hiding = focusFold(workspace, context.workspaceScreen ? context.focusedCheckoutId : null, expanded);
  // A checkout Hide AI made is drawn only while it is in front or an operator's agent works in it (PRD hide-ai-work-home D-17, B3).
  const drawn = (checkout: Checkout) => drawnInTree(hideAiListed, workspace, checkout, checkout.id === context.focusedCheckoutId, (rowsByCheckout.get(checkout.id) ?? NO_BOARD_ROWS).length > 0);
  const emptyFold = workspace.session_folds ? workspace.checkouts.filter((checkout) => workspace.session_folds!.empty.includes(checkout.id) && drawn(checkout)) : [];
  const inactiveDrawn = inactive.filter(drawn);
  // The project's main checkout is its row; every other checkout, with or without an agent, waits in one fold whose badge carries what runs or asks inside it.
  const activeDrawn = active.filter(drawn);
  // The project's own row stands for a main checkout nothing runs in: its row is drawn only while it holds an agent or a conversation, a purpose, or the focus.
  const main = activeDrawn.filter((checkout) => checkout.is_primary && ((rowsByCheckout.get(checkout.id) ?? NO_BOARD_ROWS).length > 0 || checkout.purpose !== null || checkout.id === context.focusedCheckoutId));
  const others = [...activeDrawn, ...emptyFold, ...inactiveDrawn].filter((checkout) => !checkout.is_primary);
  const othersOpen = workspace.inactive_checkouts.expanded;
  const othersMarks = sumMarks(others.map((checkout) => checkout.agent_summary?.marks));
  const checkoutRow = (checkout: Checkout) => (
    <CheckoutRowView
      key={checkout.id}
      workspace={workspace}
      checkout={checkout}
      hideAi={hideAiCheckoutOf(hideAiListed, workspace, checkout)}
      agentRows={rowsByCheckout.get(checkout.id) ?? NO_BOARD_ROWS}
      focused={context.workspaceScreen && checkout.id === context.focusedCheckoutId}
      context={context}
    />
  );
  return (
    <li data-project={workspace.id} data-project-pinned={workspace.pinned ? "true" : undefined} data-project-quiet={quiet ? "true" : undefined} className={cn(!first && "mt-xs", quiet && "opacity-(--opacity-dimmed)")}>
      <EntryContextMenu
        label={t("common.entryActions", { name: workspace.label })}
        items={() => projectMenu(workspace, menuHost(), t, issueSource)}
        onSelect={(item) => runProjectItem(actions, workspace, item)}
        className="group flex items-stretch"
        data-project-menu={workspace.id}
      >
        <div
          className={cn("flex min-h-(--size-project-row) w-full items-center gap-xs rounded-sm pr-xs", hiding === "project" ? "bg-secondary" : "hover:bg-accent")}
          style={{ paddingLeft: PROJECT_COLUMN }}
        >
          <button
            type="button"
            data-project-row={workspace.id}
            aria-current={hiding === "project" ? "true" : undefined}
            aria-label={workspace.label}
            className="flex min-w-0 flex-1 self-stretch items-center gap-sm rounded-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
            onClick={() => actions.openProject(workspace, projectRowExpansion(workspace, target, context.workspaceScreen ? context.focusedCheckoutId : null, disclosure))}
          >
            {/* A project's own mark, so its checkouts read as standing under it; a pinned one wears the pin in its place, and a Git project a folder with the branch mark. */}
            <ProjectMark workspace={workspace} />
            <ProjectName workspace={workspace} label />
            {/* A folded project still says what runs and asks inside it; open, its own rows say it. */}
            {expanded ? null : <StatusBadge counts={sumMarks(workspace.checkouts.map((checkout) => checkout.agent_summary?.marks))} className="group-hover:hidden group-focus-within:hidden" data-project-status={workspace.id} />}
          </button>
          <RowEnd>
            <RowActions
              workspace={workspace}
              items={() => projectMenu(workspace, menuHost(), t, issueSource)}
              onSelect={(item) => runProjectItem(actions, workspace, item)}
              fill={hiding === "project" ? "from-secondary" : "from-sidebar group-hover:from-accent group-focus-within:from-accent"}
            >
              <RepositoryMark workspace={workspace} actions={actions} />
            </RowActions>
            {disclosure ? (
              <FoldToggle
                open={expanded}
                quiet
                label={expanded ? t("sidebar.foldProject", { name: workspace.label }) : t("sidebar.unfoldProject", { name: workspace.label })}
                onToggle={() => actions.toggleProjectCheckouts(workspace)}
                data-project-toggle={workspace.id}
              />
            ) : (
              <FoldLane />
            )}
          </RowEnd>
        </div>
      </EntryContextMenu>
      {expanded ? (
        <ul className="pl-sm">
          {main.map(checkoutRow)}
          {others.length > 0 ? (
            <li>
              <FoldRow
                label={t("sidebar.otherCheckouts")}
                count={others.length}
                expanded={othersOpen}
                current={!othersOpen && context.workspaceScreen && others.some((checkout) => checkout.id === context.focusedCheckoutId)}
                badge={<StatusBadge counts={othersMarks} data-other-checkouts-status={workspace.id} />}
                onToggle={() => actions.toggleInactiveCheckouts(workspace.path)}
                data-other-checkouts={workspace.id}
              />
              {othersOpen ? <ul>{others.map(checkoutRow)}</ul> : null}
            </li>
          ) : null}
        </ul>
      ) : null}
    </li>
  );
}

const NO_BOARD_ROWS: BoardRow[] = [];

/**
 * The row of this project that carries the focused checkout's selection while
 * a fold hides that checkout's own row: the folded project's row, or the
 * empty-worktree or Inactive fold that holds it. Null while the checkout's row
 * is drawn, or the checkout is another project's or under the cleanup fold.
 */
function focusFold(workspace: Workspace, checkoutId: string | null, expanded: boolean): "project" | "empty" | "inactive" | null {
  if (checkoutId === null || !workspace.checkouts.some((checkout) => checkout.id === checkoutId)) return null;
  const folds = workspace.session_folds;
  if (folds?.cleanup.includes(checkoutId)) return null;
  if (!expanded) return "project";
  const inactive = workspace.inactive_checkouts.checkout_ids.includes(checkoutId);
  const empty = folds?.empty.includes(checkoutId) ?? false;
  if ((!inactive && !empty) || (inactive && workspace.inactive_checkouts.expanded) || (empty && folds!.empty_open)) return null;
  return empty ? "empty" : "inactive";
}

function CleanupRows({ workspaces, context, deviceId }: { workspaces: Workspace[]; context: ListContext; deviceId: string }) {
  const { t } = useInterfaceTranslation();
  const hideAiListed = useShellStore((s) => s.rest?.navigator?.hide_ai_checkouts);
  // A cleanup candidate is what the operator reviews, so a checkout Hide AI made and kept after its pull request settled is listed here too (PRD hide-ai-work-home B18).
  const rows = workspaces.flatMap((workspace) =>
    workspace.checkouts
      .filter((checkout) => workspace.session_folds?.cleanup.includes(checkout.id))
      .map((checkout) => ({ workspace, checkout })),
  );
  if (rows.length === 0) return null;
  const expanded = rows[0]!.workspace.session_folds!.cleanup_open;
  const current = !expanded && context.workspaceScreen && rows.some(({ checkout }) => checkout.id === context.focusedCheckoutId);
  return <li data-session-cleanup={deviceId}>
    <FoldRow label={t("agentSessions.cleanup")} count={rows.length} expanded={expanded} current={current} onToggle={() => context.actions.toggleSessionFold(`cleanup/${deviceId}`)} />
    {expanded ? <ul>{rows.map(({ workspace, checkout }) => <CheckoutRowView key={checkout.id} workspace={workspace} checkout={checkout} hideAi={hideAiCheckoutOf(hideAiListed, workspace, checkout)}
      agentRows={checkoutAgentRows(workspace, context.treeAgents, "sidebar").get(checkout.id) ?? NO_BOARD_ROWS}
      focused={context.workspaceScreen && context.focusedCheckoutId === checkout.id} context={context} />)}</ul> : null}
  </li>;
}


/**
 * What a checkout's row draws besides line one, shared by the checkout row and
 * a folder's one row: its agent rows open only where agents run and the
 * operator opened them, or on a selected SSH device, whose tree has no folds.
 * Line two is the purpose and exists only while there
 * is one (PRD sidebar-typography D-04); opening the agents never grows or
 * shrinks the row.
 */
/** `otherLines` counts what the opened list draws besides agent rows: sleeping conversations and named tabs with no agent. */
function checkoutDisclosure(checkout: Checkout, agentRows: BoardRow[], view: CheckoutPresentation, context: ListContext, t: TFunction<"translation">, otherLines: number, hideAi: HideAiCheckout | null) {
  const hasRows = agentRows.length + otherLines > 0 || (checkout.agent_groups?.groups.length ?? 0) > 0;
  const foldable = context.disclosure && hasRows;
  const open = hasRows && !context.openCheckouts.includes(checkout.id);
  // A checkout Hide AI made names its task where the purpose stands, ahead of the purpose (D-03).
  const purpose = [hideAi ? originLine(t, hideAi) : null, checkout.purpose?.text ?? null].filter(Boolean).join(" · ") || null;
  const parents = [...new Set(agentRows
    .filter((row) => row.depth === 0)
    .map((row) => context.agents.find((candidate) => candidate.pane_id === row.agent.lineage_parent_pane_id))
    .filter((parent): parent is AgentRow => parent !== undefined)
    .map((parent) => parent.identity_label))];
  const raisedFrom = parents.length > 0 ? t("sidebar.raisedFrom", { name: `${parents[0]}${parents.length > 1 ? ` +${parents.length - 1}` : ""}` }) : null;
  // Opened, the agent rows say what the checkout is doing, so the row is one line (B35).
  return { foldable, open, purpose, secondLine: !open && checkoutHasSecondLine(view, purpose, raisedFrom), raisedFrom };
}

/**
 * A checkout: the row opens it, and a right-click opens its menu. Line one
 * ends in its agents' status badge while their rows are folded, then the fold
 * slot, whose chevron opens those rows while agents run in it. Line two is
 * the purpose, and the last-commit age ends the row's last line on the time
 * column: line two's when the row has a purpose, line one's otherwise.
 * Opened, its rows and it share a small group fill, and the rows' own marks
 * stand for the badge. While its worktree is being deleted the row and its
 * agent rows are dimmed, a spinner stands where the badge was, and neither
 * the row nor its menu opens anything.
 */
const CheckoutRowView = memo(function CheckoutRowView({
  workspace,
  checkout,
  hideAi,
  agentRows,
  focused,
  context,
}: {
  workspace: Workspace;
  checkout: Checkout;
  /** Set for a checkout Hide AI made: its task is named on the row (D-03). */
  hideAi: HideAiCheckout | null;
  agentRows: BoardRow[];
  focused: boolean;
  context: ListContext;
}) {
  const { t } = useInterfaceTranslation();
  const { actions } = context;
  const removing = useShellStore((s) => checkoutRemoving(s.rest?.worktree_removal, workspace.device_id, checkout.path, localDeviceId(s.rest)));
  const view = checkoutPresentation(workspace, checkout, Date.now(), t);
  const name = checkout.branch ?? checkout.label;
  const sleeping = useSleepingCheckout(checkout.path, workspace.device_id);
  const emptyTabs = useEmptyNamedTabs(checkout, workspace.device_id, agentRows, context.agents);
  const { foldable, open, purpose, secondLine, raisedFrom } = checkoutDisclosure(checkout, agentRows, view, context, t, sleeping.length + emptyTabs, hideAi);
  const visibleRows = open ? agentRows : agentRows.filter((row) => row.depth === 0 && asksOperator(row.agent));
  const marks = checkout.agent_summary?.marks;
  return (
    <li
      data-checkout-row={checkout.id}
      data-checkout-open={open ? "true" : undefined}
      data-checkout-removing={removing ? "true" : undefined}
      className={cn(removing && "opacity-(--opacity-dimmed)")}
    >
      <EntryContextMenu
        label={t("common.entryActions", { name })}
        items={() => checkoutMenu(workspace, checkout, menuHost(), t)}
        onSelect={(item) => runCheckoutItem(actions, workspace, checkout, item)}
        disabled={removing}
        className="group flex items-stretch"
        data-checkout-menu={checkout.id}
      >
        <div
          className={cn(
            "relative flex w-full flex-col justify-center gap-xxs rounded-sm pr-xs",
            secondLine ? "min-h-(--size-checkout-row-detailed)" : "min-h-(--size-checkout-row)",
            focused ? "bg-secondary" : !removing && "hover:bg-accent",
            view.settled && !removing && "opacity-(--opacity-dimmed)",
          )}
          style={{ paddingLeft: PROJECT_COLUMN }}
        >
          <CheckoutOpenButton
            workspace={workspace}
            checkout={checkout}
            view={view}
            focused={focused}
            label={[name, removing ? t("sidebar.deleting") : open ? null : badgeWords(marks, t), view.age, purpose].filter(Boolean).join(", ")}
            actions={actions}
            expanded={checkoutRowExpansion(foldable, context.workspaceScreen && focused, open)}
            disabled={removing}
          />
          {/* The row button covers the whole row; a control drawn over it is positioned, so it stacks above.
              Line one is 22 whatever it carries: the 24-high fold toggle overhangs it rather than moving line two. */}
          <span className="pointer-events-none flex h-(--size-sidebar-line) min-w-0 items-center gap-xs">
            <KindGlyph view={view} deviceId={workspace.device_id} actions={actions} />
            <CheckoutName name={name} focused={focused} />
            <CheckoutBadge checkout={checkout} />
            <span className="flex-1" />
            <RowEnd>
              {removing ? (
                <Loader2Icon aria-hidden="true" data-checkout-removing-mark="true" className="size-(--size-icon-sm) shrink-0 animate-spin text-muted-foreground" />
              ) : open ? null : (
                <StatusBadge counts={marks} data-checkout-status={checkout.id} />
              )}
              {!secondLine && view.age ? <CheckoutAge age={view.age} /> : null}
              {foldable ? (
                <FoldToggle
                  open={open}
                  label={open ? t("sidebar.hideAgentsIn", { name }) : t("sidebar.showAgentsIn", { name })}
                  onToggle={() => actions.toggleCheckoutAgents(checkout.id)}
                  data-checkout-toggle={checkout.id}
                />
              ) : (
                <FoldLane />
              )}
            </RowEnd>
          </span>
          {secondLine ? <PurposeLine purpose={purpose} origin={checkout.purpose?.origin} age={view.age} raisedFrom={raisedFrom} /> : null}
        </div>
      </EntryContextMenu>
      {visibleRows.length > 0 || (open && (sleeping.length + emptyTabs > 0 || !!checkout.agent_groups)) ? <OpenAgentRows checkout={checkout} groupTabs={open} deviceId={workspace.device_id} agentRows={visibleRows} sleeping={open ? sleeping : NO_SLEEPERS} inset={CHECKOUT_NAME_COLUMN} glyphColumn={PROJECT_COLUMN} context={context} /> : null}
    </li>
  );
});

/**
 * A plain folder (`folderCheckout`): the project and its only checkout are
 * one row. Line one is the project's name, with no agent counts; line
 * two is the purpose. The row opens the checkout and is marked while that
 * checkout is the Workspace in front, and its
 * menu is the project's followed by the checkout's. It ends as a checkout row
 * does, and its project scope is reached from the shared Overview.
 */
const FolderRowView = memo(function FolderRowView({
  workspace,
  checkout,
  agentRows,
  focused,
  first,
  context,
}: {
  workspace: Workspace;
  checkout: Checkout;
  agentRows: BoardRow[];
  focused: boolean;
  /** The first row of its list: no project-block hairline above it. */
  first: boolean;
  context: ListContext;
}) {
  const { t } = useInterfaceTranslation();
  const { actions } = context;
  const issueSource = useShellStore((s) => projectIssueSource(s.rest, workspace));
  const view = checkoutPresentation(workspace, checkout, Date.now(), t);
  const sleeping = useSleepingCheckout(checkout.path, workspace.device_id);
  const emptyTabs = useEmptyNamedTabs(checkout, workspace.device_id, agentRows, context.agents);
  // A folder whose every agent sleeps follows its own fold, which starts folded (D-03, D-19).
  const asleep = context.disclosure && workspace.agent_presence?.asleep !== undefined;
  const disclosure = checkoutDisclosure(checkout, agentRows, view, context, t, sleeping.length + emptyTabs, null);
  const { foldable, purpose, raisedFrom } = disclosure;
  const open = asleep ? projectExpanded(workspace) : disclosure.open;
  const secondLine = asleep ? !open && checkoutHasSecondLine(view, purpose, raisedFrom) : disclosure.secondLine;
  const visibleRows = open ? agentRows : agentRows.filter((row) => row.depth === 0 && asksOperator(row.agent));
  return (
    <li data-project={workspace.id} data-project-pinned={workspace.pinned ? "true" : undefined} data-checkout-open={open ? "true" : undefined} className={cn(!first && "mt-xs")}>
      <EntryContextMenu
        label={t("common.entryActions", { name: workspace.label })}
        items={() => folderMenu(workspace, checkout, menuHost(), t, issueSource)}
        onSelect={(item) => (FOLDER_CHECKOUT_ITEMS.has(item) ? runCheckoutItem(actions, workspace, checkout, item) : runProjectItem(actions, workspace, item))}
        className="group flex items-stretch"
        data-project-menu={workspace.id}
      >
        <div
          className={cn(
            "relative flex w-full flex-col justify-center gap-xxs rounded-sm pr-xs",
            secondLine ? "min-h-(--size-checkout-row-detailed)" : "min-h-(--size-project-row)",
            focused ? "bg-secondary" : "hover:bg-accent",
          )}
          style={{ paddingLeft: PROJECT_COLUMN }}
        >
          <CheckoutOpenButton
            workspace={workspace}
            checkout={checkout}
            view={view}
            focused={focused}
            label={[workspace.label, purpose].filter(Boolean).join(", ")}
            actions={actions}
            expanded={asleep ? undefined : checkoutRowExpansion(foldable, context.workspaceScreen && focused, open)}
            projectExpanded={asleep ? checkoutRowExpansion(true, context.workspaceScreen && focused, open) : undefined}
          />
          <span className="pointer-events-none flex h-(--size-sidebar-line) min-w-0 items-center gap-sm">
            <ProjectMark workspace={workspace} />
            <ProjectName workspace={workspace} hidden grow={false} label />
            <CheckoutBadge checkout={checkout} />
            <span className="flex-1" />
            <RowEnd>
              <RowActions
                workspace={workspace}
                items={() => folderMenu(workspace, checkout, menuHost(), t, issueSource)}
                onSelect={(item) => (FOLDER_CHECKOUT_ITEMS.has(item) ? runCheckoutItem(actions, workspace, checkout, item) : runProjectItem(actions, workspace, item))}
                fill={focused ? "from-secondary" : "from-sidebar group-hover:from-accent group-focus-within:from-accent"}
              />
              {foldable ? (
                <FoldToggle
                  open={open}
                  label={open ? t("sidebar.hideAgentsIn", { name: workspace.label }) : t("sidebar.showAgentsIn", { name: workspace.label })}
                  onToggle={() => (asleep ? actions.toggleProjectCheckouts(workspace) : actions.toggleCheckoutAgents(checkout.id))}
                  data-checkout-toggle={checkout.id}
                />
              ) : (
                <FoldLane />
              )}
            </RowEnd>
          </span>
          {secondLine ? <PurposeLine purpose={purpose} origin={checkout.purpose?.origin} age={null} raisedFrom={raisedFrom} /> : null}
        </div>
      </EntryContextMenu>
      {visibleRows.length > 0 || (open && (sleeping.length + emptyTabs > 0 || !!checkout.agent_groups)) ? <OpenAgentRows checkout={checkout} groupTabs={open} deviceId={workspace.device_id} agentRows={visibleRows} sleeping={open ? sleeping : NO_SLEEPERS} inset={PROJECT_NAME_COLUMN} glyphColumn={PROJECT_COLUMN} context={context} /> : null}
    </li>
  );
});

/**
 * The checkout's kind glyph. A row with a pull request draws that pull
 * request's mark without its number (the name already names the branch) as a
 * button that opens it (PRD checkout-pr-glyph-card D-02): a ring in the
 * state's colour under the pointer says so, ⌘ asks for the default browser,
 * and the press never reaches the row button under it, so the checkout
 * neither opens nor unfolds. Every other kind is the plain icon the row
 * button covers.
 */
/** The project's mark in a box the checkout glyph's size, so a tab's guide hangs at the center of either; a pinned project is told by where the list puts it, and its pin waits for the pointer in the row's buttons. */
function ProjectMark({ workspace }: { workspace: Workspace }) {
  const Icon = workspace.is_git ? FolderGit2Icon : FolderIcon;
  return (
    <span className="flex size-(--size-checkout-icon) shrink-0 items-center justify-center text-muted-foreground">
      <Icon aria-hidden="true" data-project-mark={workspace.id} className="size-(--size-icon-sm)" />
    </span>
  );
}

function KindGlyph({ view, deviceId, actions }: { view: CheckoutPresentation; deviceId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pr = view.kind === "pull_request" ? view.pullRequest : null;
  if (view.kind === "pull_request") {
    if (!pr) throw new Error("A pull request glyph without its pull request");
    return (
      <button
        type="button"
        data-checkout-pr-glyph={pr.number}
        data-checkout-glyph="true"
        aria-label={t("sidebar.openPullRequest", { number: pr.number })}
        className={cn(
          "pointer-events-auto relative flex size-(--size-checkout-icon) shrink-0 cursor-pointer items-center justify-center rounded-xs outline-none hover:ring-1 hover:ring-current focus-visible:ring-1 focus-visible:ring-ring",
          PR_LOOK[pr.state].tone,
        )}
        onClick={(event) => {
          event.stopPropagation();
          actions.openPullRequest(pr.url, deviceId, pullRequestOpenExternal(event));
        }}
      >
        <PrMark state={pr.state} stale={view.stale} compact />
      </button>
    );
  }
  const KindIcon = CHECKOUT_KIND_ICON[view.kind];
  return <KindIcon aria-hidden="true" data-checkout-glyph="true" className={cn("m-px size-(--size-icon-sm) shrink-0", view.kindTone)} />;
}

/**
 * The button under a checkout's whole row: it opens the checkout, and the
 * card it opens on hover and focus carries the pull request, agents, branch
 * and path (`checkoutCard`); a screen reader reads the same facts as the
 * sentence `view.detail`.
 */
function CheckoutOpenButton({
  workspace,
  checkout,
  view,
  focused,
  label,
  actions,
  expanded,
  projectExpanded,
  disabled = false,
}: {
  workspace: Workspace;
  checkout: Checkout;
  view: CheckoutPresentation;
  focused: boolean;
  label: string;
  actions: Actions;
  expanded?: boolean;
  /** The project's own fold to set in the same event: a sleeping folder's (D-19). */
  projectExpanded?: boolean;
  disabled?: boolean;
}) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  // Read on every render, like `view.detail`: a memo keyed on the checkout would keep the Commit age from the last change to this row.
  const card = checkoutCard(workspace, checkout, Date.now(), t);
  return (
    <CheckoutCardHint
      card={card}
      description={view.detail}
      onOpenPullRequest={(url, external) => actions.openPullRequest(url, workspace.device_id, external)}
      onShowPullRequestRow={workspace.is_git && readsHere(workspace, devices) ? (number) => actions.openPrCard(workspace.id, number) : undefined}
    >
      <RowOverlayButton
        hover={false}
        data-checkout={checkout.id}
        data-checkout-kind={view.kind}
        aria-current={focused ? "true" : undefined}
        aria-label={label}
        disabled={disabled}
        className="rounded-sm disabled:pointer-events-auto disabled:cursor-default"
        onClick={() => actions.openWorkspace(workspace.device_id, checkout.workspace_id, checkout.id, expanded, projectExpanded)}
      />
    </CheckoutCardHint>
  );
}

function CheckoutBadge({ checkout }: { checkout: Checkout }) {
  const { t } = useInterfaceTranslation();
  if (!checkout.exists) {
    return (
      <Badge variant="secondary" className="shrink-0 text-destructive" data-checkout-missing="true">
        {t("sidebar.checkoutMissing")}
      </Badge>
    );
  }
  if (checkout.temporary) {
    return (
      <Badge variant="secondary" className="shrink-0 text-warning">
        {t("sidebar.checkoutTemporary")}
      </Badge>
    );
  }
  return null;
}

/**
 * A checkout's name at 12/400, 500 while its Workspace is in front, with its
 * path prefix (`prd/`) muted so the part that tells checkouts apart reads
 * first (PRD sidebar-typography D-02, D-06).
 */
function CheckoutName({ name, focused }: { name: string; focused: boolean }) {
  const { prefix, rest } = checkoutNameParts(name);
  return (
    <span aria-hidden="true" data-row-name="true" className={cn("min-w-0 truncate text-body text-foreground", focused && "font-medium")}>
      {prefix ? (
        <span className="text-muted-foreground" data-name-prefix="true">
          {prefix}
        </span>
      ) : null}
      {rest}
    </span>
  );
}

/** A checkout's last-commit age, drawn on the time column of whichever line ends the row. */
function CheckoutAge({ age }: { age: string }) {
  return (
    <span aria-hidden="true" data-checkout-age={age} className="font-mono text-caption text-muted-foreground">
      {age}
    </span>
  );
}

/**
 * A checkout's line two: its purpose, and the last-commit age ending on the
 * time column over an empty fold slot, as line one ends. A folder's line two
 * has no age: a plain folder has no commit.
 */
function PurposeLine({ purpose, origin, age, raisedFrom }: { purpose: string | null; origin: string | undefined; age: string | null; raisedFrom: string | null }) {
  return (
    // The row is already inset to its glyph, so line two starts one glyph and gap further, under the name.
    <span aria-hidden="true" className="pointer-events-none flex min-h-(--size-sidebar-line-detail) min-w-0 items-center gap-sm pl-(--size-checkout-metadata-inset) text-caption leading-(--size-sidebar-line-detail)">
      <span className="flex min-w-0 flex-1 items-center gap-xs truncate text-muted-foreground" data-purpose={purpose === null ? undefined : origin}>
        {raisedFrom ? (
          <span className="inline-flex min-w-0 shrink items-center gap-xxs text-subtle-foreground" data-checkout-parent={raisedFrom}>
            <CornerUpLeftIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
            <span className="truncate">{raisedFrom}</span>
          </span>
        ) : null}
        {purpose ? <span className="min-w-0 truncate">{purpose}</span> : null}
      </span>
      {age ? (
        <RowEnd>
          <CheckoutAge age={age} />
          <FoldLane />
        </RowEnd>
      ) : null}
    </span>
  );
}

/**
 * An opened checkout's tree (PRD agent-hierarchy-screens D-12, D-38; B10 to
 * B16): its operator roots, and under a root the operator opened its
 * children and grandchildren wherever they work, five siblings at a time,
 * joined by elbow rails. A grandchild's chevron opens the tree popover
 * rooted on it. A dormant conversation has no pane or numbered shortcut.
 */
function OpenAgentRows({ checkout, groupTabs, deviceId, agentRows, sleeping, inset, glyphColumn, context }: { checkout: Checkout; groupTabs: boolean; deviceId: string; agentRows: BoardRow[]; sleeping: SleepingSession[]; inset: string; glyphColumn: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const closed = useGroupFolds((state) => state.closed);
  const groups = useMemo(() => sidebarTabGroups(checkout, deviceId, agentRows, context.agents), [checkout, deviceId, agentRows, context.agents]);
  const named = useMemo(() => sidebarNamedGroups(checkout, groups), [checkout, groups]);
  // The core publishes the ungrouped collection first, and a header names a
  // collection against the others, so a checkout holding no named group draws
  // its ungrouped rows alone under the checkout, with no header at all
  // (docs/UI_BEHAVIOR.md, Sidebar hierarchy).
  const ungrouped = named.find((collection) => collection.group.id === null);
  const hasNamed = named.some((collection) => collection.group.id !== null);
  // An empty, unselected ungrouped collection draws no header: the row would
  // name a collection with nothing under it (docs/UI_BEHAVIOR.md).
  const shown = named.filter((collection) => collection.group.id !== null || collection.group.id === checkout.agent_groups?.selected || collection.segments.length > 0 || collection.drafts.length > 0);
  if (groupTabs && checkout.agent_groups && ungrouped && hasNamed) return <ul className="isolate mt-xxs mb-sm" data-checkout-agents-open={checkout.id}>
    {shown.map((collection, index) => {
      const { group } = collection;
      const foldKey = groupFoldKey(deviceId, checkout.id, group.id);
      const open = !closed.has(foldKey);
      const name = group.name ?? t("groups.ungrouped");
      const selected = group.id === checkout.agent_groups!.selected;
      const childInset = `calc(${inset} + var(--spacing-md))`;
      return <li key={group.id ?? "ungrouped"} data-sidebar-named-group={group.id ?? "ungrouped"} className={cn(index > 0 && "mt-sm")}>
        {/* The header stands one step in from the checkout's own row: its chevron on the checkout's glyph column
            and its briefcase where its rows' own column starts, so a collection never reads deeper than its rows. */}
        <div className="flex h-(--size-control-compact) min-w-0 items-center gap-xs rounded-sm pr-xs text-caption hover:bg-accent" style={{ paddingLeft: glyphColumn }}>
          <Button variant="text" size="inline" aria-label={t(open ? "groups.fold" : "groups.unfold", { name })} aria-expanded={open} onClick={() => useGroupFolds.getState().toggle(foldKey)}>{open ? <ChevronDownIcon className="size-(--size-icon-sm)" /> : <ChevronRightIcon className="size-(--size-icon-sm)" />}</Button>
          <Hint label={name} reveals><Button variant="text" size="inline" data-sidebar-group-select={group.id ?? "ungrouped"} aria-current={selected ? "true" : undefined} className={cn("min-w-0 flex-1 justify-start", selected && "font-medium text-foreground")} onClick={() => context.actions.agentGroup({ device_id: deviceId, path: checkout.path }, group.id, { action: "select" })}><BriefcaseBusinessIcon className={cn("size-(--size-icon-sm) shrink-0", selected ? "text-foreground" : "text-muted-foreground")} /><span className="truncate">{name}</span></Button></Hint>
        </div>
        {open ? <ul className="mt-xxs"><CollectionRows checkout={checkout} deviceId={deviceId} collection={collection} inset={childInset} guideColumn={GROUP_GLYPH_COLUMN} guideSize="var(--size-icon-sm)" context={context} /></ul> : null}
      </li>;
    })}
    {sleeping.map((session) => <SleepingSessionRow key={session.sleep_id} session={session} actions={context.actions} inset={inset} />)}
  </ul>;
  // A checkout with no named group keeps the core's own order for its ungrouped
  // collection, at the column a checkout's own tabs have always stood on.
  if (groupTabs && ungrouped && !hasNamed && checkout.agent_groups) return <ul className="isolate mt-xxs mb-sm" data-checkout-agents-open={checkout.id}>
    <CollectionRows checkout={checkout} deviceId={deviceId} collection={ungrouped} inset={inset} guideColumn={glyphColumn} guideSize="var(--size-checkout-icon)" context={context} />
    {sleeping.map((session) => <SleepingSessionRow key={session.sleep_id} session={session} actions={context.actions} inset={inset} />)}
  </ul>;
  // The list before any catalog answered for its groups: one segment per tab, in
  // the order the checkout itself publishes them.
  return <ul className={cn("isolate", groupTabs && "mt-xxs mb-sm")} data-checkout-agents-open={checkout.id}>
    {groups.map((group, index) => (
      <li key={group.tab?.id ?? "unplaced"} className={cn("relative", groupTabs && index > 0 && "mt-sm")} data-sidebar-tab-group={group.tab?.id ?? undefined}>
        {groupTabs ? <TabSegmentHead group={group} checkout={checkout} deviceId={deviceId} inset={inset} glyphColumn={glyphColumn} context={context} /> : null}
        <AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={group.rows} inset={inset} fold={guideCenter(glyphColumn, "var(--size-checkout-icon)")} context={context} />
      </li>
    ))}
    {sleeping.map((session) => <SleepingSessionRow key={session.sleep_id} session={session} actions={context.actions} inset={inset} />)}
  </ul>;
}

/**
 * One collection's rows: its drafts and tab segments in the core's own order,
 * then any segment no collection claims. Each row draws at the inset its case
 * gives, so the same collection hangs under a group header or, when no named
 * group exists, directly under its checkout.
 */
function CollectionRows({ checkout, deviceId, collection, inset, guideColumn, guideSize, context }: { checkout: Checkout; deviceId: string; collection: SidebarCollection; inset: string; guideColumn: string; guideSize: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const { group, segments, drafts } = collection;
  const placed = new Set(group.tabs);
  return <>
    {group.tabs.map((id) => {
      if (drafts.includes(id)) return <li key={id}><RowButton data-sidebar-tab={id} className="h-(--size-sidebar-line) rounded-sm py-0 text-caption text-subtle-foreground" style={{ paddingLeft: inset }} onClick={() => context.actions.agentGroup({ device_id: deviceId, path: checkout.path }, group.id, { action: "select", tab_id: id })}><PanelsTopLeftIcon className="size-(--size-icon-sm) shrink-0" /><span className="truncate">{t("groups.newTab")}</span></RowButton></li>;
      const segment = segments.find((candidate) => candidate.tab?.id === id);
      return segment ? <li key={id} className="relative" data-sidebar-tab-group={id}><TabSegmentHead group={segment} checkout={checkout} deviceId={deviceId} inset={inset} glyphColumn={guideColumn} glyphSize={guideSize} context={context} /><AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={segment.rows} inset={inset} fold={guideCenter(guideColumn, guideSize)} context={context} /></li> : null;
    })}
    {segments.filter((segment) => segment.tab === null || !placed.has(segment.tab.id!)).map((segment) => <li key={segment.tab?.id ?? "unplaced"}><AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={segment.rows} inset={inset} fold={guideCenter(guideColumn, guideSize)} context={context} /></li>)}
  </>;
}

/** The center of a guide column: a tab's guide hangs there, and a root's chevron stands on it. */
function guideCenter(column: string, size: string): string {
  return `calc(${column} + ${size} / 2)`;
}

/** A tab's quiet line: a sidebar line and a hairline of air. */
const TAB_LINE = "calc(var(--size-sidebar-line) + var(--spacing-xxs))";

/**
 * A tab is a segment of its checkout's agent list, not a level (docs/UI_BEHAVIOR.md, Sidebar hierarchy): every tab, whatever it holds or is named,
 * draws a hairline guide centered under the glyph of the row it hangs from (its group's briefcase, or the checkout's kind glyph when no group header is drawn) spanning the segment's rows, and, unless it is an unnamed tab
 * holding one agent, opens with one quiet line inside the guide, the tab glyph on the status-mark column and its name (the manual one, else `Tab N`) on the title column, so it reads as part of
 * the checkout rather than beside it. The line opens the real tab, and the hint says its name and agent count, or only its name with no agent.
 */
function TabSegmentHead({ group, checkout, deviceId, inset, glyphColumn, glyphSize = "var(--size-checkout-icon)", context }: { group: SidebarTabGroup; checkout: Checkout; deviceId: string; inset: string; glyphColumn: string; glyphSize?: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const removing = useShellStore((s) => checkoutRemoving(s.rest?.worktree_removal, deviceId, checkout.path, localDeviceId(s.rest)));
  const tab = group.tab;
  const naming = tab?.sidebar;
  const name = naming ? naming.manual_name ?? t("sidebar.tabNumber", { number: naming.number }) : checkout.strip.find((entry) => entry.source_id === tab?.id)?.label ?? t("groups.newTab");
  // A tab the operator kept with no agent left says its name alone, never a count of none.
  const label = group.count === 0 ? name : t("sidebar.tabAgents", { name, count: group.count });
  const center = guideCenter(glyphColumn, glyphSize);
  const open = () => context.actions.openSurface(tabSurface(checkout, tab!.id!, deviceId));
  if (!group.glyph) return null;
  return (
    <>
      {/* It stands above a selected or hovered row's fill, so only a tab boundary breaks it; the list isolates its stacking so it never rises over the sticky section header. */}
      <span aria-hidden="true" data-sidebar-tab-guide={tab!.id!} className="pointer-events-none absolute top-xs bottom-xs z-10 w-(--size-hairline) bg-lineage-rail" style={{ left: `calc(${center} - var(--size-hairline) / 2)` }} />
      {group.line ? <div data-sidebar-tab-line={tab!.id!} className="group/row relative rounded-sm hover:bg-accent" style={{ height: TAB_LINE }}>
          <Hint label={label} reveals>
            <RowOverlayButton hover={false} data-sidebar-tab={tab!.id!} disabled={removing} aria-label={label} className="rounded-sm" onClick={open} />
          </Hint>
          <span aria-hidden="true" className="pointer-events-none absolute top-1/2 -translate-x-1/2 -translate-y-1/2 text-subtle-foreground" style={{ left: `calc(${inset} + var(--size-agent-mark) / 2)` }}><PanelsTopLeftIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span>
          <span aria-hidden="true" data-sidebar-tab-name="true" className="pointer-events-none absolute inset-y-0 right-xs flex min-w-0 items-center text-caption text-subtle-foreground" style={{ left: `calc(${inset} + var(--size-agent-mark) + var(--spacing-xs))` }}><span className="truncate">{name}</span></span>
        </div> : null}
    </>
  );
}

/** `fold` is the center of the column a root's chevron stands on: its tab's guide. */
function AgentTreeRows({ checkoutId, deviceId, agentRows, inset, fold, context }: { checkoutId: string; deviceId: string; agentRows: BoardRow[]; inset: string; fold: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const [shownAll, setShownAll] = useState<ReadonlySet<string>>(NO_PANES);
  const siblings = useShellStore((s) => siblingsShown(s.rest));
  const lines = useMemo(() => sidebarTreeLines(agentRows, shownAll, siblings), [agentRows, shownAll, siblings]);
  // Which devices are reachable now, as one value, so a reconnect redraws the rows that name one.
  const reachable = useShellStore((s) => [localDeviceId(s.rest), ...(s.rest?.status?.remote ?? []).filter((row) => row.state === "connected").map((row) => row.target_id)].join("\n"));
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  return (
    <ul>
      {lines.map((line) => {
        if (line.kind === "more") {
          return (
            <li key={`more:${line.parent}`} className="flex items-stretch rounded-sm pr-xs hover:bg-accent" style={{ paddingLeft: inset }}>
              <TreeRails place={line.place} />
              <Button variant="text" size="inline" data-tree-more={line.count} className="min-w-0 flex-1 justify-start py-xs pl-xxs text-caption" onClick={() => setShownAll((before) => new Set(before).add(line.parent))}>
                <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                <span className="min-w-0 truncate">{t("agentSessions.tree.more", { count: line.count })}</span>
              </Button>
            </li>
          );
        }
        const { agent, depth } = line.row;
        const parent = depth > 0 ? context.agents.find((row) => row.pane_id === agent.lineage_parent_pane_id) : undefined;
        const hasChildren = context.childrenOf(agent).length > 0;
        const parentDevice = parent?.device_id ?? deviceId;
        const remote = depth > 0 && agent.device_id !== undefined && agent.device_id !== parentDevice
          ? { label: machineName(devices, agent.device_id, t), reachable: reachable.split("\n").includes(agent.device_id) }
          : null;
        const tree: SidebarTreeSlot = {
          place: line.place,
          fold,
          open: hasChildren ? depth < 2 && !agent.lineage_collapsed : null,
          onToggle: () => context.actions.toggleAgentTree(agent.pane_id),
          popover: hasChildren && depth >= 2 ? {
            agents: context.agents,
            onOpenChild: (id, label) => context.actions.followRelation(agent.pane_id, id, label),
            onTask: () => context.actions.revealTask(agent.pane_id),
            actions: context.actions,
          } : null,
        };
        return (
          <SidebarAgentRow
            key={`${agent.pane_id}:${depth}`}
            agent={agent}
            device={depth === 0 && agent.device_id !== undefined && agent.device_id !== deviceId ? machineName(devices, agent.device_id, t) : null}
            place={null}
            depth={depth}
            tree={tree}
            remote={remote}
            selected={agent.pane_id === context.focusRowPane}
            onOpen={depth === 0 || !parent ? (id) => context.actions.openListedAgent(id) : (id) => context.actions.followRelation(parent.pane_id, id, agent.identity_label)}
            onOpenPullRequest={(url, external) => context.actions.openLink(url, external)}
            inset={inset}
            // A tree row is one line with no branch (B14); the popover names a differing branch.
            branchShown={false}
            number={depth === 0 ? context.numberOf?.(agent.pane_id, checkoutId) ?? null : null}
            menu={context.agentRowMenu}
            pulls={context.familyOf(agent)}
            onOpenMission={depth === 0 ? (id) => context.actions.openMission(id) : undefined}
            actions={context.actions}
          />
        );
      })}
    </ul>
  );
}

const NO_PANES: ReadonlySet<string> = new Set();

/** What a row's menu reads from the host when it opens: the OS file manager, and the new-tab chord the registry binds here. */
function menuHost(): MenuHost {
  const rest = useShellStore.getState().rest;
  return { reveal: revealHost(), newTabChord: commandLabel("new_tab"), node: localDeviceId(rest), screen: windowDeviceId(rest?.navigator?.devices), editor: editorHost() };
}

function runProjectItem(actions: Actions, workspace: Workspace, item: MenuItem["id"]) {
  switch (item) {
    case "project_settings":
      return useUiStore.getState().openProjectSettings(settingsTarget(workspace));
    case "rename_project":
      return useUiStore.getState().openProjectSettings(settingsTarget(workspace), null, "name");
    case "new_worktree":
      return useUiStore.getState().setWorkspaceDialog({ kind: "new_worktree", workspaceId: workspace.id });
    case "new_tab_primary": {
      const primary = primaryCheckout(workspace);
      return primary ? actions.newTabIn(workspace.device_id, primary) : undefined;
    }
    case "reveal_external":
      return actions.revealExternal(workspace.path);
    case "open_in_editor":
      return actions.openInEditor(workspace.path);
    case "copy_path":
      return actions.copyText(workspace.path, "path");
    case "pin":
    case "unpin":
      return actions.setPinned(workspace.id, item === "pin");
    case "issue_source_auto":
    case "issue_source_github":
    case "issue_source_local":
      return actions.setProjectSettings(settingsTarget(workspace), { issue_source: item.slice("issue_source_".length) as "auto" | "github" | "local" }, crypto.randomUUID());
    case "remove_project":
      return useUiStore.getState().setWorkspaceDialog({ kind: "remove_project", workspaceId: workspace.id });
  }
}

function runCheckoutItem(actions: Actions, workspace: Workspace, checkout: Checkout, item: MenuItem["id"]) {
  switch (item) {
    case "open_checkout":
      return actions.openWorkspace(workspace.device_id, checkout.workspace_id, checkout.id);
    case "new_tab_here":
      return actions.newTabIn(workspace.device_id, checkout);
    case "new_group":
      return useGroupName.getState().open({ workspace: { device_id: workspace.device_id, path: checkout.path }, groupId: null, name: "" });
    case "open_pull_request": {
      const pr = shownPullRequest(checkout);
      return pr ? actions.openPullRequest(pr.url, workspace.device_id, false) : undefined;
    }
    case "set_purpose":
      return useUiStore.getState().setWorkspaceDialog({ kind: "purpose", workspaceId: workspace.id, checkoutId: checkout.id });
    case "set_primary":
      return actions.setPrimaryCheckout(workspace.id, checkout.id);
    case "copy_branch":
      return checkout.branch ? actions.copyText(checkout.branch, "branch name") : undefined;
    case "copy_path":
      return actions.copyText(checkout.path, "path");
    case "reveal_external":
      return actions.revealExternal(checkout.path);
    case "open_in_editor":
      return actions.openInEditor(checkout.path);
    case "delete_worktree":
      return useUiStore.getState().setWorkspaceDialog({ kind: "delete_worktree", workspaceId: workspace.id, checkoutId: checkout.id });
    case "remove_project":
      return useUiStore.getState().setWorkspaceDialog({ kind: "remove_project", workspaceId: workspace.id });
  }
}

/**
 * One agent row menu for a whole list: Show carries the ⌥n that selects the
 * same row in the Agents list, read when the menu opens.
 */
function useAgentRowMenu(actions: Actions): AgentRowMenu {
  const { t } = useInterfaceTranslation();
  return useMemo(
    () => ({
      items: (agent) => {
        const state = useShellStore.getState();
        const number = agentNumber(state, agent.pane_id, useUiStore.getState());
        const chord = number === null ? "" : commandLabel(`select_agent_${number}`, state.rest?.ui_state);
        return agentMenu(agent, chord, t);
      },
      onSelect: (agent, item) => {
        switch (item) {
          case "show_agent":
            return actions.openAgent(agent.pane_id);
          case "copy_reference":
            return actions.copyText(agentReference(agent), "agent reference");
          case "copy_session_id":
            return agent.session_id ? actions.copyText(agent.session_id, "session id") : undefined;
          case "close_tab":
            return actions.closeTabOfPane(agent.pane_id);
        }
      },
    }),
    [actions, t],
  );
}

function directChildren(parent: AgentRow, agents: AgentRow[]): AgentRow[] {
  return (parent.lineage_child_pane_ids ?? []).map((id) => agents.find((agent) => agent.pane_id === id)).filter((agent): agent is AgentRow => agent !== undefined);
}
