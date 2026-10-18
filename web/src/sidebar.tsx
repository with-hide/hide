import { deviceScope } from "./agentScope";
import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { useInterfaceTranslation } from "./i18n/client";
import { BriefcaseBusinessIcon, ChevronDownIcon, ChevronRightIcon, CornerUpLeftIcon, EllipsisIcon, FolderIcon, Loader2Icon, PanelsTopLeftIcon, PinIcon, PlusIcon, RefreshCwIcon, ServerIcon, SettingsIcon } from "lucide-react";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { StatusMark } from "./components/status-mark";
import { markTone } from "./agentRow";
import { drawnInTree, hideAiCheckoutOf, originLine } from "./hideAiOrigin";
import { useFactoryTurnCount } from "./factory/hooks";
import { FACTORY_OPEN } from "./factory/launch";
import { REQUEST_ANSWER_TIMEOUT_MS } from "./factory/request";
import { paneListed } from "./factory/secretary";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { EntryContextMenu, EntryDropdown } from "./components/entry-menu";
import { SidebarHeader, type DeviceMenu, type FactoryProjectRow } from "./components/sidebar-header";
import { GithubIcon } from "./components/github-icon";
import { Hint } from "./components/ui/tooltip";
import { CHECKOUT_KIND_ICON } from "./components/checkout-icon";
import { PrMark } from "./components/pr-mark";
import { agentsByPane, allAgents as allPaneAgents } from "./paneAgents";
import { familyPulls, PR_LOOK, type SessionPull } from "./prMark";
import { CheckoutCardHint, pullRequestOpenExternal } from "./components/pr-card";
import { DeviceRail } from "./components/device-rail";
import { frontDeviceId, frontTitle, railShown, readsHere, sidebarBody } from "./devices";
import { badgeWords } from "./agentRow";
import { Badge } from "./components/ui/badge";
import { cn } from "./lib/utils";
import { agentNumber, sidebarAgentNumbers } from "./numbering";
import { checkoutAgentRows, type BoardRow } from "./projectBoard";
import { FoldLane, SidebarAgentRow, type AgentRowMenu, type SidebarTreeSlot } from "./components/sidebar-agent-row";
import { TreeRails } from "./components/agent-tree";
import { sidebarTreeLines } from "./sidebarTree";
import { moveSidebarKey, sidebarNamedGroups, sidebarTabGroups, type SidebarTabGroup } from "./sidebarTabs";
import { groupFoldKey, useGroupFolds, useGroupName } from "./groupTabs";
import { tabSurface } from "./recent";
import { SleepingSessionRow } from "./components/sleeping-session-row";
import { StatusBadge } from "./components/status-badge";
import { Usage } from "./components/usage";
import { agentGroupTitle, agentPlaces, allAgents, type ListedAgent } from "./navigation";
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
import { editorHost, hostBridge, revealHost } from "./host";
import { commandLabel } from "./shortcutLabels";
import type { Digit } from "./shortcuts";
import { contextAgents, contextAllWorkspaces, contextWorkspaces, deviceCatalogLine, herdrPaneId, remoteContext, remoteView } from "./remote";
import { agentMenu, checkoutMenu, checkoutRemoving, FOLDER_CHECKOUT_ITEMS, folderMenu, primaryCheckout, projectMenu, remotePurposeProblem, type MenuHost, type MenuItem } from "./workspaceManage";
import { projectIssueSource, projectRepository, settingsTarget } from "./projectSettings";
import { machineName, windowDeviceId, windowFirst } from "./screenMachine";
import { focusedRemoteDevice, localDeviceId, type AgentRow, type Checkout, type HideAiCheckout, type SleepingSession, type SnapshotRest, type Workspace } from "./snapshot";
import { useStartPanel } from "./startDraft";
import { HelpMenu } from "./components/help-menu";
import { ViewMenu } from "./components/view-menu";
import { useShellStore } from "./store";
import { draggedSidebarWidth, sidebarWidthToSend } from "./sidebarWidth";
import { useUiStore } from "./ui";
import { useOverviewCount } from "./Overview";
import { revealSidebarFocus, sidebarFocusPane, sidebarPanes } from "./sidebarFocus";
import { SidebarLensSwitch, TaskLens } from "./TaskLens";

function herdrRowLabel(state: string | null, t: TFunction<"translation">): string | null {
  if (state === "unconfigured" || state === "socket_missing") return t("sidebar.herdrSocketMissing");
  if (state === "not_connected" || state === "unreachable" || state === "stale") {
    return t("sidebar.herdrNotResponding");
  }
  return null;
}

export function Sidebar({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const herdrState = useShellStore((s) => s.herdrState);
  const visible = useShellStore((s) => s.rest?.ui_state?.left_sidebar_visible ?? true);
  // The row speaks for this machine's Herdr, so it is not shown over a
  // selected SSH device's lists; that device's state is on the canvas.
  const remote = useShellStore((s) => focusedRemoteDevice(s.rest) !== null);
  const status = remote ? null : herdrRowLabel(herdrState, t);
  // The rail is shown unless the operator hid it (quick device-rail-badges B1, B6).
  const rail = useShellStore((s) => railShown(s.rest));
  const body = useShellStore((s) => sidebarBody(s.rest));
  const lens = useShellStore((s) => s.rest?.ui_state?.sidebar_lens ?? "projects");
  useSecretaryStart(actions);
  // The switch has no chord of its own until the operator binds one (issue 170).
  const overviewChord = useShellStore((s) => commandLabel("overview", s.rest?.ui_state));
  const overviewCount = useOverviewCount();
  const overviewSelected = useUiStore((s) => s.overviewOpen || s.screen?.kind === "main");
  const factoryChord = useShellStore((s) => commandLabel("factory_open", s.rest?.ui_state));
  const factoryCount = useFactoryTurnCount();
  const factoryPlace = useUiStore((s) => (!s.overviewOpen && s.screen?.kind === "factory" ? s.screen.place.factory : undefined));
  const factorySelected = factoryPlace === null;
  const factoryProjects = useFactoryProjects(actions, factoryPlace);
  const secretary = useSecretaryRow(actions);
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
  // Under the head, the 프로젝트 | 태스크 switch picks the list (PRD sidebar-task-lens B1, D-10). The 태스크
  // list keeps an unresponsive device's last rows, dimmed, under its reason (B20).
  const list = lens === "tasks" ? <TaskLens actions={actions} /> : body === "disconnected" ? <DisconnectedDevice actions={actions} /> : <ProjectList actions={actions} />;

  return (
    // The width is a CSS variable, set here and on the shell for the top row
    // alike (`useSidebarColumns`), never the nav's own inline width, which the
    // e2e overflow check sets to try other widths (PRD sidebar-typography D-13).
    <div className="relative h-full w-(--sidebar-total) shrink-0" style={columns} data-sidebar-box="true">
      {/* The sidebar keeps its own text sizes whatever Appearance's interface
          font is (D-05): `interface-scale-exempt` is the generated rule in
          tokens.css that declares the sizes again at scale 1. */}
      <nav
        className={cn("interface-scale-exempt absolute inset-y-0 left-0 flex w-(--sidebar-total) bg-sidebar text-foreground", preview !== null && "z-30")}
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
            factory={FACTORY_OPEN ? { selected: factorySelected, count: factoryCount, chord: factoryChord || null, onOpen: () => openFactoryAt(actions, null), projects: factoryProjects, secretary } : "soon"}
            searchChord={commandLabel("search") || null}
            onSearch={() => actions.openSearch()}
          />
          {status ? (
            <div className="border-b border-border px-md py-sm text-caption text-muted-foreground">{status}</div>
          ) : null}
          <SidebarLensSwitch actions={actions} />
          <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-sidebar-content="true">
            {list}
            <div data-sidebar-footer="true" className="flex shrink-0 flex-wrap items-center gap-xs border-t border-border px-sm py-xs">
              <Usage actions={actions} />
              {/* Usage holds the left edge; the menus keep the right one, also on a wrapped line. */}
              <div className="ml-auto flex items-center gap-xs">
                <ViewMenu actions={actions} />
                <HelpMenu actions={actions} />
                <Hint label={`${t("common.settings")} (${commandLabel("settings")})`}>
                  <Button variant="ghost" size="icon-sm" data-open-settings="true" onClick={() => actions.openSettings()}>
                    <SettingsIcon />
                  </Button>
                </Hint>
              </div>
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

/** Opens the Factory where it was, filtered to one Factory or to all of them. */
function openFactoryAt(actions: Actions, factory: string | null) {
  actions.openFactory();
  useUiStore.getState().setFactoryPlace(factory === null ? { factory } : { factory, task: null, column: null });
}

/**
 * The Factories under the Factory row (PRD factory-observer B34): each
 * project's 내 차례 number and pause, opening the Factory filtered to it.
 */
function useFactoryProjects(actions: Actions, place: string | null | undefined): FactoryProjectRow[] {
  const summary = useShellStore((s) => s.factory?.summary ?? null);
  return useMemo(() => {
    const open = (summary?.factories ?? []).filter((view) => !view.closed);
    return open.map((view) => ({
      id: view.id,
      name: view.project_name,
      count: view.my_turn,
      paused: view.paused,
      selected: place === view.id,
      onOpen: () => openFactoryAt(actions, view.id),
    }));
  }, [summary, place, actions]);
}

/**
 * The secretary's row (PRD software-factory-ui B23), once a Factory exists:
 * its agent's status mark while the core still lists its pane.
 */
function useSecretaryRow(actions: Actions) {
  const exists = useShellStore((s) => s.factory?.summary?.factories.some((view) => !view.closed) ?? false);
  const pane = useShellStore((s) => s.rest?.ui_state?.factory_secretary_pane ?? null);
  const agent = useShellStore((s) => (pane ? (s.rest?.navigator?.agents ?? []).find((row) => row.pane_id === pane) ?? null : null));
  return useMemo(() => {
    if (!exists) return null;
    return { status: agent ? <StatusMark mark={agent.mark} className={markTone(agent)} data-secretary-status={agent.status_code} /> : null, onOpen: () => actions.openSecretary() };
  }, [exists, agent, actions]);
}

/**
 * Carries a secretary start through (B23): the Factory's default runtime
 * once its config answers, then the Home agent's pane, which the core keeps
 * as the secretary once a tab lists it, and the page opens. A refused or
 * unanswered step ends the start without starting anything, into the
 * diagnostic log, and the next press tries again.
 */
function useSecretaryStart(actions: Actions) {
  const start = useUiStore((s) => s.secretaryStart);
  const answer = useShellStore((s) => (start?.phase === "config" ? (s.factory?.actions.find((row) => row.request_id === start.requestId)?.answer ?? null) : null));
  const operation = useShellStore((s) => s.rest?.task_operation);
  const error = useShellStore((s) => s.rest?.status?.last_error);
  const listed = useShellStore((s) => (start?.phase === "listing" ? paneListed(s.rest, start.paneId) : false));
  useEffect(() => {
    if (start === null) return undefined;
    const timer = window.setTimeout(() => {
      if (useUiStore.getState().secretaryStart !== start) return;
      useUiStore.getState().setSecretaryStart(null);
      useShellStore.getState().noteDiagnostic(`factory secretary: no answer while ${start.phase}`);
    }, REQUEST_ANSWER_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [start]);
  useEffect(() => {
    if (start?.phase !== "config" || answer === null) return;
    if (!answer.ok) {
      useUiStore.getState().setSecretaryStart(null);
      useShellStore.getState().noteDiagnostic(`factory secretary: config refused (${answer.reason ?? "no reason"})`);
      return;
    }
    const runtime = (answer as { config?: { default_runtime?: string } }).config?.default_runtime;
    actions.startSecretary(runtime === "codex" ? "codex" : "claude");
  }, [start, answer, actions]);
  useEffect(() => {
    if (start?.phase !== "start") return;
    const ui = useUiStore.getState();
    if (operation?.request_id === start.requestId && operation.pane_id && operation.phase !== "failed") {
      ui.setSecretaryStart({ phase: "listing", paneId: operation.pane_id });
      ui.setFocusWhenListed(operation.pane_id);
      return;
    }
    if (error?.request_id === start.requestId || (operation?.request_id === start.requestId && operation.phase === "failed")) ui.setSecretaryStart(null);
  }, [start, operation, error, actions]);
  useEffect(() => {
    if (start?.phase !== "listing" || !listed) return;
    useUiStore.getState().setSecretaryStart(null);
    actions.factorySecretarySet(start.paneId);
  }, [start, listed, actions]);
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
  const workspaces = useShellStore((s) => contextAllWorkspaces(s.rest));
  const agents = useShellStore((s) => contextAgents(s.rest, s.agents));
  const scope = useShellStore((s) => deviceScope(s.rest, frontDeviceId(s.rest)));
  const drawn = useMemo(() => sidebarPanes(workspaces, scope), [workspaces, scope]);
  return useMemo(() => (workspaceScreen ? sidebarFocusPane(focusedPaneId, agents, drawn) : null), [workspaceScreen, focusedPaneId, agents, drawn]);
}

/** A selected device the core cannot read: its name, why, and the one way to try again, with the last tree left out (PRD home-device-rail B8, B9). */
function DisconnectedDevice({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const deviceId = useShellStore((s) => frontDeviceId(s.rest));
  const label = useShellStore((s) => machineName(s.rest?.navigator?.devices, frontDeviceId(s.rest), t));
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-sm px-md text-center" data-device-disconnected={deviceId}>
      <ServerIcon aria-hidden="true" className="size-(--size-icon-lg) text-subtle-foreground" />
      <span className="max-w-full truncate text-subhead font-semibold text-foreground" data-device-disconnected-name="true">
        {label}
      </span>
      <span role="status" className="text-caption text-muted-foreground">
        {t("devices.state.unavailable")}
      </span>
      <Button variant="secondary" size="sm" data-device-reconnect={deviceId} onClick={() => actions.retryDevice(deviceId)}>
        <RefreshCwIcon aria-hidden="true" />
        {t("sidebar.reconnect")}
      </Button>
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
 * screen. A selected SSH device lists its Herdr workspaces, one checkout
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
  const noAgentsFold = useShellStore((s) => (remote ? undefined : s.rest?.navigator?.no_agent_projects));
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
  const rows = useMemo(
    () => projectRows(workspaces, { inactive_projects: inactiveFold, no_agent_projects: noAgentsFold }, listedAgents, frontScope, raisedOpen),
    [workspaces, inactiveFold, noAgentsFold, listedAgents, frontScope, raisedOpen],
  );
  // The device's Needs You and Done come first, then the projects in their sections; Working and Seen are the Agents tab's.
  const raised = useMemo(() => rows.filter((row) => row.kind === "raised"), [rows]);
  const sections = useMemo(() => projectSections(rows), [rows]);
  useStuckHeaders(list, loaded);
  const agentRowMenu = useAgentRowMenu(actions);
  const children = useMemo(() => new Map(agents.map((agent) => [agent.pane_id, directChildren(agent, agents)])), [agents]);
  // A root row's PR mark gathers its family over the rows the 태스크 list reads, so the two agree (PRD project-tree-pr-family D-01, D-02).
  const families = useMemo(() => {
    const every = allPaneAgents(localAgents, remoteStatuses);
    const byPane = agentsByPane(every);
    return new Map(every.flatMap((agent) => (agent.state.root ? [[agent.pane_id, familyPulls(agent, byPane)] as const] : [])));
  }, [localAgents, remoteStatuses]);
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
  const context: ListContext = {
    agents,
    childrenOf: (agent) => children.get(agent.pane_id)!,
    familyOf: (agent) => families.get(agent.pane_id),
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
      {/* Needs You and Done stand still above the projects, so opening one row
          after another never scrolls them away: an opened row leaves its
          section and the next one moves up into its place. Up to half the
          height, past which they scroll on their own. */}
      {catalogLine || raised.length > 0 ? (
        <ul className="max-h-1/2 shrink-0 overflow-auto px-sm" data-raised-list="true">
          {catalogLine ? (
            <li role="status" className="px-md py-xs text-caption text-muted-foreground" data-device-catalog={catalogLine.state}>
              {catalogLine.text}
            </li>
          ) : null}
          {raised.map((row) => (
            <ProjectRowView key={rowKey(row)} row={row} context={context} />
          ))}
        </ul>
      ) : null}
      <ul ref={list} className="min-h-0 flex-1 overflow-auto px-sm" data-project-list="true">
        {/* Each section is its own box, so its header sticks to the list's top only while its rows pass under it, and the next section's header pushes it away (D-05). */}
        {sections.map((section, index) => (
          <li key={`section:${section.header.section}`} data-project-section={section.header.section}>
            <ul>
              <SectionHeader row={section.header} actions={actions} />
              {section.rows.map((row) => (
                <ProjectRowView key={rowKey(row)} row={row} context={context} />
              ))}
              {index === sections.length - 1 ? <CleanupRows workspaces={workspaces} context={context} deviceId={frontId} /> : null}
            </ul>
          </li>
        ))}
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
  childrenOf: (agent: AgentRow) => AgentRow[];
  /** A root's own and every descendant's PRs, undefined for a child or a root without children. */
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

const PROJECT_SECTION_TITLE: Record<Extract<ProjectRow, { kind: "header" }>["section"], MessageKey> = {
  pinned: "overview.pinned",
  recent: "sidebar.recentActivity",
};

function rowKey(row: ProjectRow): string {
  switch (row.kind) {
    case "header":
      return `header:${row.section}`;
    case "raised":
      return `raised:${row.group}`;
    case "workspace":
      return `workspace:${row.workspace.id}`;
    case "inactive_projects":
      return `inactive-projects:${row.group.device_id}`;
    case "no_agents":
      return `no-agents:${row.group.device_id}`;
  }
}

type SectionRow = Extract<ProjectRow, { kind: "header" }>;

/** The list cut at its section headers: each header with the rows under it. */
function projectSections(rows: ProjectRow[]): { header: SectionRow; rows: ProjectRow[] }[] {
  const sections: { header: SectionRow; rows: ProjectRow[] }[] = [];
  for (const row of rows) {
    if (row.kind === "raised") continue;
    if (row.kind === "header") sections.push({ header: row, rows: [] });
    else sections.at(-1)?.rows.push(row);
  }
  return sections;
}

/**
 * A section's header, stuck to the top of the list while its rows scroll
 * under it, with a hairline under it only while it is stuck (D-05). The
 * Pinned header carries the pin, which no pinned row repeats; the Projects
 * title carries the sidebar's one Add project, shown with or without
 * projects, where the host can pick a folder (PRD first-run-onboarding B51);
 * with none it is the bare word, over the empty list's line.
 */
function SectionHeader({ row, actions }: { row: SectionRow; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const add = row.section === "recent" && hostBridge();
  return (
    <li
      className="sticky top-0 z-10 flex h-(--size-sidebar-section-header) items-end gap-xs bg-sidebar pr-xs pb-xs pl-sm text-micro font-medium text-muted-foreground data-[stuck=true]:border-b data-[stuck=true]:border-border"
      data-section={row.section}
      data-section-sticky="true"
    >
      {row.section === "pinned" ? <PinIcon aria-hidden="true" data-section-pin="true" className="size-(--size-icon-sm) shrink-0" /> : null}
      <span className="min-w-0 flex-1 truncate">
        {row.section === "recent" && row.count === 0 ? t("sidebar.projects") : `${t(PROJECT_SECTION_TITLE[row.section])} · ${row.count}`}
      </span>
      {add ? (
        <Hint label={t("commands.new_workspace")} shortcut={commandLabel("new_workspace") || null}>
          <Button variant="ghost" size="icon-sm" className="size-(--size-sidebar-line)" data-sidebar-new-workspace="true" onClick={() => actions.openAddProject()}>
            <PlusIcon />
          </Button>
        </Hint>
      ) : null}
    </li>
  );
}

/**
 * Marks each section header that is stuck to the top of the list, which draws
 * its hairline; read on scroll and on any size change of the list or a row of
 * it, written straight to the element so scrolling renders nothing.
 */
function useStuckHeaders(list: { current: HTMLUListElement | null }, loaded: boolean) {
  // Set up once per list, not per snapshot: what moves a header is a scroll,
  // a size change, or a row of the list arriving or leaving.
  useEffect(() => {
    const element = list.current;
    if (!element) return undefined;
    const mark = () => {
      const top = element.getBoundingClientRect().top;
      for (const header of element.querySelectorAll<HTMLElement>("[data-section-sticky]")) {
        const section = header.parentElement!.getBoundingClientRect();
        header.dataset.stuck = String(section.top < top - 0.5 && section.bottom > top);
      }
    };
    const sizes = new ResizeObserver(mark);
    const watch = () => {
      sizes.disconnect();
      sizes.observe(element);
      for (const child of element.children) sizes.observe(child);
      mark();
    };
    watch();
    const rows = new MutationObserver(watch);
    rows.observe(element, { childList: true });
    element.addEventListener("scroll", mark, { passive: true });
    return () => {
      element.removeEventListener("scroll", mark);
      rows.disconnect();
      sizes.disconnect();
    };
  }, [list, loaded]);
}

const ProjectRowView = memo(function ProjectRowView({ row, context }: { row: ProjectRow; context: ListContext }) {
  const { t } = useInterfaceTranslation();
  if (row.kind === "header") throw new Error("A section header is drawn by its section");
  if (row.kind === "raised") return <RaisedSection row={row} context={context} />;
  if (row.kind === "inactive_projects" || row.kind === "no_agents") {
    const current = !row.group.expanded && context.focusedProjectId !== null && row.group.project_ids.includes(context.focusedProjectId);
    return (
      <li>
        {row.kind === "no_agents" ? (
          <FoldRow
            label={t("sidebar.noAgentProjects")}
            count={row.count}
            expanded={row.group.expanded}
            current={current}
            onToggle={() => context.actions.toggleSessionFold(`no_agents/${row.group.device_id}`)}
            data-no-agent-projects={row.group.device_id}
          />
        ) : (
          <FoldRow
            label={t("sidebar.inactiveProjects")}
            count={row.count}
            expanded={row.group.expanded}
            current={current}
            onToggle={() => context.actions.toggleInactiveProjects(row.group.device_id)}
            data-inactive-projects={row.group.device_id}
          />
        )}
      </li>
    );
  }
  return <WorkspaceRows workspace={row.workspace} level={row.level} context={context} />;
});

/**
 * A raised Needs You or Done section above the project tree
 * (docs/status-model.md): the Agents list's row for each agent, drawn
 * whatever its project, checkout or parent has folded. A raised row never
 * unfolds, so it always wears its descendant badge; opening it does what the
 * same row does in Agents. Past its cap the older rows wait behind a fold
 * row, as an inactive fold's do.
 */
function RaisedSection({ row, context }: { row: Extract<ProjectRow, { kind: "raised" }>; context: ListContext }) {
  const { t } = useInterfaceTranslation();
  const toggleRaised = useUiStore((s) => s.toggleRaised);
  return (
    <li data-raised-group={row.group}>
      <div className="px-sm pt-sm pb-xxs text-micro font-medium text-muted-foreground" data-section={row.group} id={`raised-group-${row.group}`}>
        {agentGroupTitle(row.group, t)} · {row.agents.length + row.more.length}
      </div>
      <ul aria-labelledby={`raised-group-${row.group}`}>
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
    </li>
  );
}

function RaisedAgentRow({ listed: { agent, device }, context }: { listed: ListedAgent; context: ListContext }) {
  return (
    <SidebarAgentRow
      agent={agent}
      // The device in front is the whole list, so no row names it (PRD home-device-rail B2).
      device={null}
      place={context.placeOf(device, agent.pane_id)}
      depth={0}
      ask={agent.state.needs_you}
      selected={agent.pane_id === context.focusRowPane}
      onOpen={(id) => context.actions.openListedAgent(id)}
      onOpenPullRequest={(url, external) => context.actions.openLink(url, external)}
      inset={PROJECT_COLUMN}
      number={context.numberOf?.(agent.pane_id, null) ?? null}
      menu={context.agentRowMenu}
      family={context.familyOf(agent)}
    />
  );
}

// The Projects columns (PRD sidebar-readability D-2, sidebar-quiet-mark-indent
// D-06): a project with checkouts draws no glyph, so its name starts at the
// row's inset, a checkout's glyph stands under it, and an opened checkout's
// agent marks on the checkout name's column. A plain folder's row keeps its
// glyph, which says the checkout's kind (D-07). Every row ends the
// same way, agent rows included: its time, then its fold slot, so the times
// and the chevrons stand in one column each. A row's menu opens on a
// right-click, or the menu key or ⇧F10 on the focused row; nothing on the
// row stands for it.
/** Where a project's name, a checkout's kind glyph, a plain folder's glyph and every fold start. */
const PROJECT_COLUMN = "var(--spacing-sm)";
/** Where a plain folder's name and its agent marks start. */
const PROJECT_NAME_COLUMN = "calc(var(--spacing-sm) + var(--size-checkout-icon) + var(--spacing-sm))";
/** Where a checkout's name, its line two and its agents' marks start. */
const CHECKOUT_NAME_COLUMN = `calc(${PROJECT_COLUMN} + var(--size-checkout-icon) + var(--spacing-xs))`;

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
  return <span className="flex shrink-0 items-center gap-xs">{children}</span>;
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
  onToggle,
  ...data
}: {
  label: string;
  count: number;
  expanded: boolean;
  current?: boolean;
  onToggle: () => void;
} & Record<`data-${string}`, string>) {
  const Chevron = expanded ? ChevronDownIcon : ChevronRightIcon;
  return (
    <button
      type="button"
      aria-expanded={expanded}
      aria-current={current ? "true" : undefined}
      className={cn(
        "flex min-h-(--size-control-lg) w-full items-center gap-sm rounded-sm pr-xs text-left text-caption font-medium outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring",
        current ? "bg-secondary text-foreground" : "text-subtle-foreground hover:bg-accent",
      )}
      style={{ paddingLeft: PROJECT_COLUMN }}
      onClick={onToggle}
      {...data}
    >
      <span className="min-w-0 flex-1 truncate">
        {label} {count}
      </span>
      <span className="flex w-(--size-lineage-chevron) shrink-0 justify-center text-muted-foreground">
        <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
      </span>
    </button>
  );
}

/** The fold on a project or checkout row: a folded one is always shown, an open one waits for the pointer (D-3). */
function FoldToggle({ open, label, onToggle, disabled, ...data }: { open: boolean; label: string; onToggle: () => void; disabled?: boolean } & Record<`data-${string}`, string>) {
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <button
      type="button"
      disabled={disabled}
      aria-expanded={open}
      aria-label={label}
      className={cn(
        "pointer-events-auto relative flex h-(--size-icon-button-toolbar) w-(--size-lineage-chevron) shrink-0 items-center justify-center rounded-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring",
        open && ROW_REVEALED,
      )}
      onClick={onToggle}
      {...data}
    >
      <Chevron aria-hidden="true" className="size-(--size-icon-sm)" />
    </button>
  );
}

/**
 * A project's name, at 600 while one of its awake agents draws a mark and
 * at 400 in the plain foreground otherwise, as the core decided (PRD
 * sidebar-titlebar-refresh D-02, B3). `hidden` keeps it from a screen reader
 * where the row button already names the project.
 */
function ProjectName({ workspace, hidden = false, grow = true }: { workspace: Workspace; hidden?: boolean; grow?: boolean }) {
  const working = workspace.agent_presence?.working === true;
  return (
    <span
      aria-hidden={hidden ? "true" : undefined}
      data-row-name="true"
      data-project-weight={working ? "working" : "quiet"}
      className={cn("min-w-0 truncate text-subhead text-foreground", grow && "flex-1", working ? "font-semibold" : "font-normal")}
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
 * menu is open; the cell keeps its width so showing them never moves the name.
 */
function RowActions({ workspace, items, onSelect }: { workspace: Workspace; items: () => MenuItem[]; onSelect: (id: MenuItem["id"]) => void }) {
  const { t } = useInterfaceTranslation();
  const [menuOpen, setMenuOpen] = useState(false);
  const more = useRef<HTMLButtonElement>(null);
  const control = "flex size-(--size-icon-button-toolbar) items-center justify-center rounded-xs text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring";
  return (
    <span className="grid shrink-0 items-center justify-items-end">
      <span className={cn("pointer-events-auto relative col-start-1 row-start-1 flex items-center gap-xxs", menuOpen ? "opacity-100" : ROW_REVEALED)} data-row-actions={workspace.id}>
        <Hint label={t("sidebar.startAgentIn", { name: workspace.label })}>
          <button
            type="button"
            aria-label={t("sidebar.startAgentIn", { name: workspace.label })}
            data-row-start={workspace.id}
            className={control}
            onClick={() => useStartPanel.getState().open(false, { projectId: workspace.id })}
          >
            <PlusIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </button>
        </Hint>
        <Hint label={t("projectSettings.open", { project: workspace.label })}>
          <button type="button" aria-label={t("projectSettings.open", { project: workspace.label })} data-row-settings={workspace.id} className={control} onClick={() => useUiStore.getState().openProjectSettings(settingsTarget(workspace))}>
            <SettingsIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          </button>
        </Hint>
        <EntryDropdown
          label={t("common.entryActions", { name: workspace.label })}
          items={items}
          onSelect={(id) => id === "project_settings" ? useUiStore.getState().openProjectSettings(settingsTarget(workspace), more.current) : onSelect(id)}
          align="start"
          hint={t("sidebar.moreActionsFor", { name: workspace.label })}
          onOpenChange={setMenuOpen}
          data-row-more-menu={workspace.id}
          trigger={
            <button ref={more} type="button" aria-label={t("sidebar.moreActionsFor", { name: workspace.label })} data-row-more={workspace.id} className={control}>
              <EllipsisIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            </button>
          }
        />
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
      <button
        type="button"
        aria-label={label}
        data-project-github={workspace.id}
        className="flex size-(--size-icon-button-toolbar) shrink-0 items-center justify-center rounded-xs text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
        onClick={(event) => actions.openRepository(repository, workspace.device_id, pullRequestOpenExternal(event))}
      >
        <GithubIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      </button>
    </Hint>
  );
}

function WorkspaceRows({ workspace, level, context }: { workspace: Workspace; level: "root" | "child"; context: ListContext }) {
  const { t } = useInterfaceTranslation();
  const { agents, actions, disclosure } = context;
  const active = activeCheckouts(workspace);
  const inactive = inactiveCheckouts(workspace);
  const rowsByCheckout = useMemo(() => checkoutAgentRows(workspace, agents, "sidebar"), [workspace, agents]);
  const hideAiListed = useShellStore((s) => s.rest?.navigator?.hide_ai_checkouts);
  const issueSource = useShellStore((s) => projectIssueSource(s.rest, workspace));
  const expanded = !disclosure || projectExpanded(workspace);
  const inset = level === "child" ? "pl-(--size-lineage-indent)" : "";
  const folder = folderCheckout(workspace);
  if (folder) {
    if (workspace.session_folds?.cleanup.includes(folder.id)) return null;
    return (
      <FolderRowView
        workspace={workspace}
        checkout={folder}
        agentRows={rowsByCheckout.get(folder.id) ?? NO_BOARD_ROWS}
        focused={context.workspaceScreen && folder.id === context.focusedCheckoutId}
        inset={inset}
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
    <li data-project={workspace.id} className={inset}>
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
            <ProjectName workspace={workspace} />
          </button>
          <RowActions
            workspace={workspace}
            items={() => projectMenu(workspace, menuHost(), t, issueSource)}
            onSelect={(item) => runProjectItem(actions, workspace, item)}
          />
          <RepositoryMark workspace={workspace} actions={actions} />
          {disclosure ? (
            <FoldToggle
              open={expanded}
              label={expanded ? t("sidebar.foldProject", { name: workspace.label }) : t("sidebar.unfoldProject", { name: workspace.label })}
              onToggle={() => actions.toggleProjectCheckouts(workspace)}
              data-project-toggle={workspace.id}
            />
          ) : (
            <FoldLane />
          )}
        </div>
      </EntryContextMenu>
      {expanded ? (
        <ul>
          {active.filter(drawn).map(checkoutRow)}
          {emptyFold.length > 0 ? <li>
            <FoldRow label={[t("agentSessions.emptyWorktrees"), workspace.session_folds!.open_prs > 0 ? t("agentSessions.openPrs", { count: workspace.session_folds!.open_prs }) : null].filter(Boolean).join(" · ")}
              count={emptyFold.length} expanded={workspace.session_folds!.empty_open} current={hiding === "empty"}
              onToggle={() => actions.toggleSessionFold(workspace.id)} data-empty-worktrees={workspace.id} />
            {workspace.session_folds!.empty_open ? <ul>{emptyFold.map(checkoutRow)}</ul> : null}
          </li> : null}
          {inactiveDrawn.length > 0 ? (
            <li>
              <FoldRow
                label={t("sidebar.inactive")}
                count={inactiveDrawn.length}
                expanded={workspace.inactive_checkouts.expanded}
                current={hiding === "inactive"}
                onToggle={() => actions.toggleInactiveCheckouts(workspace.path)}
                data-inactive-checkouts={workspace.path}
              />
            </li>
          ) : null}
          {workspace.inactive_checkouts.expanded ? inactiveDrawn.map(checkoutRow) : null}
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
      agentRows={checkoutAgentRows(workspace, context.agents, "sidebar").get(checkout.id) ?? NO_BOARD_ROWS}
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
  const purposeProblem = useShellStore((s) => remotePurposeProblem(workspace, s.rest?.status?.remote, t));
  const removing = useShellStore((s) => checkoutRemoving(s.rest?.worktree_removal, workspace.device_id, checkout.path, localDeviceId(s.rest)));
  const view = checkoutPresentation(workspace, checkout, Date.now(), t);
  const name = checkout.branch ?? checkout.label;
  const sleeping = useSleepingCheckout(checkout.path, workspace.device_id);
  const emptyTabs = useEmptyNamedTabs(checkout, workspace.device_id, agentRows, context.agents);
  const { foldable, open, purpose, secondLine, raisedFrom } = checkoutDisclosure(checkout, agentRows, view, context, t, sleeping.length + emptyTabs, hideAi);
  const visibleRows = open ? agentRows : agentRows.filter((row) => row.depth === 0 && row.agent.state.needs_you);
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
        items={() => checkoutMenu(workspace, checkout, menuHost(), t, purposeProblem)}
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
              Line one is 20 whatever it carries: the 24-high fold toggle overhangs it rather than moving line two. */}
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
  inset,
  context,
}: {
  workspace: Workspace;
  checkout: Checkout;
  agentRows: BoardRow[];
  focused: boolean;
  inset: string;
  context: ListContext;
}) {
  const { t } = useInterfaceTranslation();
  const { actions } = context;
  const purposeProblem = useShellStore((s) => remotePurposeProblem(workspace, s.rest?.status?.remote, t));
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
  const visibleRows = open ? agentRows : agentRows.filter((row) => row.depth === 0 && row.agent.state.needs_you);
  return (
    <li data-project={workspace.id} data-checkout-open={open ? "true" : undefined} className={inset}>
      <EntryContextMenu
        label={t("common.entryActions", { name: workspace.label })}
        items={() => folderMenu(workspace, checkout, menuHost(), t, purposeProblem, issueSource)}
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
            <FolderIcon aria-hidden="true" className={cn("size-(--size-checkout-icon) shrink-0", view.kindTone)} />
            <ProjectName workspace={workspace} hidden grow={false} />
            <CheckoutBadge checkout={checkout} />
            <span className="flex-1" />
            <RowEnd>
              <RowActions
                workspace={workspace}
                items={() => folderMenu(workspace, checkout, menuHost(), t, purposeProblem, issueSource)}
                onSelect={(item) => (FOLDER_CHECKOUT_ITEMS.has(item) ? runCheckoutItem(actions, workspace, checkout, item) : runProjectItem(actions, workspace, item))}
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
  return <KindIcon aria-hidden="true" data-checkout-glyph="true" className={cn("size-(--size-checkout-icon) shrink-0", view.kindTone)} />;
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
      <button
        type="button"
        data-checkout={checkout.id}
        data-checkout-kind={view.kind}
        aria-current={focused ? "true" : undefined}
        aria-label={label}
        disabled={disabled}
        className="absolute inset-0 rounded-sm outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-default"
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
  if (groupTabs && checkout.agent_groups) return <ul className="isolate mt-xxs mb-sm" data-checkout-agents-open={checkout.id}>
    {named.map(({ group, segments, drafts }) => {
      const foldKey = groupFoldKey(deviceId, checkout.id, group.id);
      const open = !closed.has(foldKey);
      const name = group.name ?? t("groups.ungrouped");
      const childInset = `calc(${inset} + var(--spacing-md))`;
      return <li key={group.id ?? "ungrouped"} data-sidebar-named-group={group.id ?? "ungrouped"}>
        <div className="flex h-(--size-sidebar-line) min-w-0 items-center gap-xs rounded-sm pr-xs text-caption hover:bg-accent" style={{ paddingLeft: inset }}>
          <button type="button" aria-label={t(open ? "groups.fold" : "groups.unfold", { name })} aria-expanded={open} onClick={() => useGroupFolds.getState().toggle(foldKey)} className="shrink-0 rounded-sm outline-none focus-visible:ring-1 focus-visible:ring-ring">{open ? <ChevronDownIcon className="size-(--size-icon-sm)" /> : <ChevronRightIcon className="size-(--size-icon-sm)" />}</button>
          <Hint label={name} reveals><button type="button" data-sidebar-group-select={group.id ?? "ungrouped"} className="flex min-w-0 flex-1 items-center gap-xs rounded-sm text-left outline-none focus-visible:ring-1 focus-visible:ring-ring" onClick={() => context.actions.agentGroup({ device_id: deviceId, path: checkout.path }, group.id, { action: "select" })}><BriefcaseBusinessIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /><span className="truncate">{name}</span></button></Hint>
        </div>
        {open ? <ul>{group.tabs.map((id) => {
          if (drafts.includes(id)) return <li key={id}><button type="button" data-sidebar-tab={id} className="flex h-(--size-sidebar-line) w-full min-w-0 items-center gap-xs rounded-sm pr-xs text-left text-caption text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring" style={{ paddingLeft: childInset }} onClick={() => context.actions.agentGroup({ device_id: deviceId, path: checkout.path }, group.id, { action: "select", tab_id: id })}><PanelsTopLeftIcon className="size-(--size-icon-sm) shrink-0" /><span className="truncate">{t("groups.newTab")}</span></button></li>;
          const segment = segments.find((segment) => segment.tab?.id === id);
          return segment ? <li key={id} className="relative" data-sidebar-tab-group={id}><TabSegmentHead group={segment} checkout={checkout} deviceId={deviceId} inset={childInset} glyphColumn={inset} context={context} /><AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={segment.rows} inset={childInset} context={context} /></li> : null;
        })}{segments.filter((segment) => segment.tab === null || !group.tabs.includes(segment.tab.id!)).map((segment) => <li key={segment.tab?.id ?? "unplaced"}><AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={segment.rows} inset={childInset} context={context} /></li>)}</ul> : null}
      </li>;
    })}
    {sleeping.map((session) => <SleepingSessionRow key={session.sleep_id} session={session} actions={context.actions} inset={inset} />)}
  </ul>;
  // An opened checkout's agents hang just under it, apart only so its fill and a selected row's never merge, and the
  // group's end carries the tab gap, so a row stands nearer its own checkout than the next one.
  return <ul className={cn("isolate", groupTabs && "mt-xxs mb-sm")} data-checkout-agents-open={checkout.id}>
    {groups.map((group, index) => (
      <li key={group.tab?.id ?? "unplaced"} className={cn("relative", groupTabs && index > 0 && "mt-sm")} data-sidebar-tab-group={group.tab?.id ?? undefined}>
        {groupTabs ? <TabSegmentHead group={group} checkout={checkout} deviceId={deviceId} inset={inset} glyphColumn={glyphColumn} context={context} /> : null}
        <AgentTreeRows checkoutId={checkout.id} deviceId={deviceId} agentRows={group.rows} inset={inset} context={context} />
      </li>
    ))}
    {sleeping.map((session) => <SleepingSessionRow key={session.sleep_id} session={session} actions={context.actions} inset={inset} />)}
  </ul>;
}

/** A tab's quiet line: a sidebar line and a hairline of air. */
const TAB_LINE = "calc(var(--size-sidebar-line) + var(--spacing-xxs))";

/**
 * A tab is a segment of its checkout's agent list, not a level (docs/UI_BEHAVIOR.md, Sidebar hierarchy): a tab holding two or more agents,
 * or one the operator named, draws a hairline guide centered under the checkout's kind glyph spanning the segment's rows, and opens with one quiet line inside the
 * guide, the tab glyph on the status-mark column and its name (the manual one, else `Tab N`) on the title column, so it reads as part of
 * the checkout rather than beside it. The line opens the real tab, and the hint says its name and agent count, or only its name with no agent. An unnamed single-agent
 * tab draws nothing: its row alone is the segment, and the gap between segments is the tab boundary.
 */
function TabSegmentHead({ group, checkout, deviceId, inset, glyphColumn, context }: { group: SidebarTabGroup; checkout: Checkout; deviceId: string; inset: string; glyphColumn: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const removing = useShellStore((s) => checkoutRemoving(s.rest?.worktree_removal, deviceId, checkout.path, localDeviceId(s.rest)));
  const tab = group.tab;
  const naming = tab?.sidebar;
  const name = naming ? naming.manual_name ?? t("sidebar.tabNumber", { number: naming.number }) : checkout.strip.find((entry) => entry.source_id === tab?.id)?.label ?? t("groups.newTab");
  // A tab the operator kept with no agent left says its name alone, never a count of none.
  const label = group.count === 0 ? name : t("sidebar.tabAgents", { name, count: group.count });
  const center = `calc(${glyphColumn} + var(--size-checkout-icon) / 2)`;
  const open = () => context.actions.openSurface(tabSurface(checkout, tab!.id!, deviceId));
  if (!group.glyph) return null;
  return (
    <>
      {/* It stands above a selected or hovered row's fill, so only a tab boundary breaks it; the list isolates its stacking so it never rises over the sticky section header. */}
      <span aria-hidden="true" data-sidebar-tab-guide={tab!.id!} className="pointer-events-none absolute top-xs bottom-xs z-10 w-(--size-hairline) bg-lineage-rail" style={{ left: `calc(${center} - var(--size-hairline) / 2)` }} />
      <div data-sidebar-tab-line={tab!.id!} className="group/row relative rounded-sm hover:bg-accent" style={{ height: TAB_LINE }}>
          <Hint label={label} reveals>
            <button type="button" data-sidebar-tab={tab!.id!} disabled={removing} aria-label={label} className="absolute inset-0 rounded-sm outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring" onClick={open} />
          </Hint>
          <span aria-hidden="true" className="pointer-events-none absolute top-1/2 -translate-x-1/2 -translate-y-1/2 text-subtle-foreground" style={{ left: `calc(${inset} + var(--size-agent-mark) / 2)` }}><PanelsTopLeftIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span>
          <span aria-hidden="true" data-sidebar-tab-name="true" className="pointer-events-none absolute inset-y-0 right-xs flex min-w-0 items-center text-caption text-subtle-foreground" style={{ left: `calc(${inset} + var(--size-agent-mark) + var(--spacing-xs))` }}><span className="truncate">{name}</span></span>
        </div>
    </>
  );
}

function AgentTreeRows({ checkoutId, deviceId, agentRows, inset, context }: { checkoutId: string; deviceId: string; agentRows: BoardRow[]; inset: string; context: TreeContext }) {
  const { t } = useInterfaceTranslation();
  const [shownAll, setShownAll] = useState<ReadonlySet<string>>(NO_PANES);
  const lines = useMemo(() => sidebarTreeLines(agentRows, shownAll), [agentRows, shownAll]);
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
              <button type="button" data-tree-more={line.count} className="flex min-w-0 flex-1 items-center gap-xs py-xs pl-xxs text-left text-caption text-subtle-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring" onClick={() => setShownAll((before) => new Set(before).add(line.parent))}>
                <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                <span className="min-w-0 truncate">{t("agentSessions.tree.more", { count: line.count })}</span>
              </button>
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
          open: hasChildren ? depth < 2 && !agent.lineage_collapsed : null,
          onToggle: () => context.actions.toggleAgentTree(agent.pane_id),
          popover: hasChildren && depth >= 2 ? {
            agents: context.agents,
            onOpenChild: (id, label) => context.actions.followRelation(agent.pane_id, id, label),
            onTask: () => context.actions.revealTask(agent.pane_id),
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
            family={context.familyOf(agent)}
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
          case "copy_title":
            return actions.copyText(agent.identity_label, "title");
          case "copy_session_id":
            return agent.session_id ? actions.copyText(agent.session_id, "session id") : undefined;
          case "copy_pane_id":
            return actions.copyText(herdrPaneId(agent.pane_id), "pane id");
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
