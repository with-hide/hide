import { projectCheckout } from "./projects";
import { deviceConnected, readsHere } from "./devices";
import { sameGithubPullRequest } from "./pullRequestIdentity";
import { useGroupFeedback } from "./groupTabs";
import { areaFrame } from "./areaFrames";
import { AGENT_WORDS, agentAreaStepUnavailable, agentMenu, type AgentAreaStep, type AgentCommand } from "./agentLayout";
import { findArea as findAgentArea, activeDisplay as activeAgentDisplay, adjacentInOrder as adjacentAgentArea, locateDisplay as locateAgentTab, neighbourArea as neighbourAgentArea, resizeTarget as resizeAgentTarget, type Edge as AgentEdge } from "./areaLayout";
// Every shell command in one place, so a shortcut, a button and a menu run
// the same code against the same snapshot. Each action is one core event
// (dispatch is fire-and-forget; a sequence would arrive as several frames).

import type { AgentLogoMode, AgentLogoModeSetPayload, ColorThemeSetPayload, FocusCheckoutPayload, InterfaceLanguageSetPayload, UiStateUsageHints, UsagePinSetPayload, UsageHiddenSetPayload } from "./generated/hided-ws";
import type { MessageKey } from "./i18n/catalogs";
import { translate } from "./i18n/client";
import type { InterfaceLanguage } from "./i18n/locale";
import type { ThemeChoice } from "./theme";
import { hostBridge, type HostKind } from "./host";
import {
  closeWithSaveOutcome,
  deleteBuffer,
  documentCloseCarries,
  flushBuffer,
  settledBuffer,
  storedDraftOnClose,
  tabBufferKey,
  type BufferKey,
  type CloseWatch,
  type CloseWatchFrame,
} from "./buffers";
import { createCatalogObserver, type AgentKind, type ProviderKind } from "./agentPicker";
import { closeScope, closeDecision, statusUnknownNotice, subtreeOf } from "./close";
import { draftExported, unstoredDeviceDrafts, type SettingsTab } from "./settings";
import { latestDraft, noteClosing, noteSent } from "./editor/draft";
import { RELATION_ANSWER_TIMEOUT_MS, relationState } from "./lineage";
import { frontDeviceId, localDeviceId, railShown } from "./devices";
import { machineName } from "./screenMachine";
import { allAgents, boardProjects, type OpenTarget } from "./navigation";
import { cardOfItem } from "./overviewTasks";
import { backlogIntent } from "./overviewBacklog";
import { paneProgress, sessionOfAgent, workKey } from "./sessionWork";
import { expectSurface, type Surface } from "./recent";
import { useStartPanel, useInlineStarts } from "./startDraft";
import { contextAgents, remoteConnected, remoteContext, remoteControl, remoteRequestId, remoteTargetOfPane, remoteView, withDeviceForward, type RemoteAction, type RemoteView } from "./remote";
import { armKeyTarget } from "./keyTarget";
import {
  catalogWorkspaces,
  deviceOfCheckout,
  editorFor,
  editorTabFor,
  explorerContext,
  focusedCheckout,
  frontCheckout,
  visibleTab,
  type AgentRow,
  type Checkout,
  type EditorDocumentSnapshot,
  type EditorTabSnapshot,
  type ForkPlacement,
  type IssueSettings,
  type KitComponentId,
  type MemoryAction,
  type ScreenIntro,
  type Tab,
  type Workspace,
  type LinkTarget,
  type PrMergeMethod,
} from "./snapshot";
import { fileUrl, focusBrowserDisplay, isHtmlFile } from "./browserViews";
import { requestLine } from "./editor/lineRequest";
import { terminalPathOwner, type FoundPath } from "./terminalLinkProvider";
import { homeOpening } from "./hideAiOrigin";
import { agentsByPane, allAgents as allPaneAgents } from "./paneAgents";
import { archiveWorking } from "./archive";
import { useShellStore } from "./store";
import { useUiStore, type MainView, type OverviewPlace, type PendingClose } from "./ui";
import type { DispatchFn } from "./ws";
import { closeShortcutPolicy, drawnViews, keyboardCommandOwner, keyboardInputRevision, keyboardOwner, newTabPolicy } from "./viewFocus";
import { focusTerminal } from "./terminals";
import { zoomViewer } from "./viewers/viewerZoom";
import { submitDrop, type CapturedDrop } from "./explorerDrop";
import {
  activeDisplay,
  adjacentInOrder,
  areasOf,
  displaysOfDocument,
  findArea,
  locateDisplay,
  menuEdge,
  neighbourArea,
  resizeTarget,
  viewAreaStepUnavailable,
  viewLayoutPayload,
  workspaceKey,
  type Edge,
  type ViewAreaStep,
  type ViewFrame,
  type ViewMenuId,
  type ViewWorkspace,
} from "./viewLayout";
import { workspaceViewOf, type LaneTool, type Tool } from "./workspace";

/** The `device_id` an event carries: none for the core's own node, which the core takes as the default. */
function deviceField(device: string, node: string): { device_id?: string } {
  return device === node || device === "" ? {} : { device_id: device };
}

export type Actions = ReturnType<typeof createActions>;

/**
 * Whether an event puts the front Workspace's unpinned lane up (PRD
 * right-lane B15, B16): a tool, a reveal or a linked folder. Read here,
 * where every operator event passes, so no path can forget it; it is page
 * state and never sent. Choosing an agent or a tab leaves the lane as it
 * is: only the operator closes it. An open, a History selection or a Recent
 * Panels display calls the lane through the core's call number instead
 * (`laneCalls.ts`), the one way a call from the CLI or another page arrives too.
 */
export function laneCalled(event: Parameters<DispatchFn>[0]): boolean {
  const payload = event.payload;
  switch (event.kind) {
    case "workspace_view":
      return typeof payload.tool === "string" || payload.reveal !== undefined;
    case "reveal_path":
      // A folder a terminal link names shows in the files tool's tree.
      return payload.is_directory === true;
    default:
      return false;
  }
}

export function createActions(send: DispatchFn) {
  const dispatch: DispatchFn = (event) => {
    if (laneCalled(event)) useUiStore.getState().showLane();
    return send(event);
  };
  const rest = () => useShellStore.getState().rest;
  const catalogObserver = createCatalogObserver((observing) => dispatch({ schema_version: 2, kind: "ai_settings", payload: { start_observing: observing } }));
  const diagnostic = (message: string) => useShellStore.getState().noteDiagnostic(message);
  const ui = () => useUiStore.getState();



  /** Remembers what was asked to come forward; `CenterScreen` shows it once it is in front. */
  const beginOpening = (target: OpenTarget) => {
    useUiStore.setState({ overviewOpen: false, overviewReturnFocus: ui().overviewOpen ? document.querySelector<HTMLElement>('[data-pane-view][data-focused="true"] .xterm-helper-textarea') : null, opening: { target, errorBefore: rest()?.status?.last_error?.occurred_at ?? null, failure: null } });
  };

  const current = (): { checkout: Checkout; tab: Tab | null } | null => {
    const checkout = focusedCheckout(rest());
    return checkout ? { checkout, tab: visibleTab(checkout) } : null;
  };

  /**
   * The checkout an Explorer change is made in: the one in front on the
   * selected device, named with that device so the core refuses the change
   * once another device's tree is on screen (S5.5 B16, B34).
   */
  const explorerTarget = (): { root: string; device_id?: string } | null => {
    const context = explorerContext(rest());
    if (!context.checkout) return null;
    return { root: context.checkout.path, ...deviceField(context.device, localDeviceId(rest())) };
  };

  const setLeftSidebarVisible = (visible: boolean) => {
    const state = rest()?.ui_state;
    if (!state || state.left_sidebar_visible === visible) return;
    updateUiState({ left_sidebar_visible: visible });
  };

  const setDeviceRailVisible = (visible: boolean) => {
    const state = rest()?.ui_state;
    if (!state || railShown(rest()) === visible) return;
    updateUiState({ device_rail_visible: visible });
  };

  /**
   * The core's ui state with one patch applied. The event replaces the whole
   * state rather than merging it, so every field the snapshot carries rides
   * along; a field the web omits would fall back to the core's default and
   * erase a value another surface owns. The two registration lists are the
   * exception: their own events (`register_device`, `workspace_pin_set`, …)
   * own them and the core keeps them when they are absent, so an echo of an
   * older snapshot can never undo a registration that landed after it.
   * Sidebar width is also patch-only: echoing it could undo a completed drag
   * while its snapshot is still in flight. The folds (`collapsed_workspace_ids`,
   * `expanded_checkout_ids`, `expanded_agent_pane_ids`,
   * `sessions_expanded_agent_pane_ids`) are left out for the same reason: the
   * core owns them through `project_checkouts_fold`, `checkout_agents_toggle`,
   * `agent_tree_toggle` and `session_tree_toggle`. The recent lists
   * (`recent_checkouts`, `recent_pane_ids`) ride along and the core ignores
   * them, keeping its own. The keyboard's pane, checkout and device are left
   * out too: the core moves them on their own events, and this copy can be
   * older than a click it has not echoed yet (issue 672).
   */
  const updateUiState = (patch: Record<string, unknown>) => {
    const state = rest()?.ui_state;
    if (!state) return;
    const {
      workspace_registrations: _workspaces,
      device_registrations: _devices,
      sidebar_width: _width,
      interface_language: _language,
      collapsed_workspace_ids: _projectFolds,
      expanded_checkout_ids: _checkoutFolds,
      expanded_agent_pane_ids: _lineageFolds,
      sessions_expanded_agent_pane_ids: _sessionFolds,
      selected_pane_id: _pane,
      focused_checkout_id: _checkout,
      focused_device_id: _device,
      ...owned
    } = state;
    void _workspaces;
    void _devices;
    void _width;
    void _language;
    void _projectFolds;
    void _checkoutFolds;
    void _lineageFolds;
    void _sessionFolds;
    void _pane;
    void _checkout;
    void _device;
    dispatch({ schema_version: 2, kind: "ui_state_update", payload: { ...owned, ...patch } });
  };

  /**
   * The core applies the fold against its own set, so two folds sent before
   * the first one's snapshot arrives still land one after the other.
   * `expanded` absent flips it.
   */
  const foldProject = (workspace: Workspace, expanded?: boolean) => {
    dispatch({ schema_version: 2, kind: "project_checkouts_fold", payload: { workspace_id: workspace.id, ...(expanded === undefined ? {} : { expanded }) } });
  };

  /**
   * The selected host, when a pane or tab command can go to it now. A device
   * that is not connected gets a notice and nothing is sent; the core would
   * refuse it too (`remote.control.not_connected`), but only in its log.
   */
  const remoteHost = (command: MessageKey): { targetId: string; view: RemoteView | null; agents: AgentRow[] } | null => {
    const context = remoteContext(rest());
    if (!context) return null;
    if (!remoteConnected(context)) {
      ui().setNotice({ text: translate("shell.deviceNotConnected", { device: machineName(rest()?.navigator?.devices, context.device.id, translate), command: translate(command) }), refreshable: false });
      return null;
    }
    return { targetId: context.device.id, view: remoteView(context.session, rest()?.workspace_view), agents: context.session?.agents ?? [] };
  };

  const sendRemote = (targetId: string, request: RemoteAction) => dispatch(remoteControl(targetId, request));

  /**
   * A child chip, a Return or a relationship Open (S6 B15, B16): one
   * focus_pane carrying a request id, so the core's receipt says whether this
   * click landed. A second click on the same target while the first is in
   * flight is dropped. A pane on another device is asked for there, and the
   * device's answer comes back as the same receipt.
   */
  const followRelation = (sourcePaneId: string, targetPaneId: string, label: string) => {
    const current = ui().relation;
    if (current?.targetPaneId === targetPaneId && relationState(current, rest()?.status?.pane_focus_request, translate)?.phase === "pending") return;
    const requestId = remoteRequestId();
    ui().setRelation({ requestId, sourcePaneId, targetPaneId, label });
    setTimeout(() => {
      const asked = ui().relation;
      if (asked?.requestId !== requestId || relationState(asked, rest()?.status?.pane_focus_request, translate)?.phase !== "pending") return;
      ui().setRelation({ ...asked, timedOut: true });
    }, RELATION_ANSWER_TIMEOUT_MS);
    ui().setScreen({ kind: "workspace" });
    const targetId = remoteTargetOfPane(rest(), targetPaneId);
    if (targetId) {
      dispatch({
        schema_version: 2,
        kind: "remote_control",
        payload: { target_id: targetId, request_id: requestId, report_pane_focus_outcome: true, action: "focus_pane", pane_id: targetPaneId },
      });
      return;
    }
    dispatch({ schema_version: 2, kind: "focus_pane", payload: { pane_id: targetPaneId, origin: "operator", request_id: requestId } });
  };

  /** The id of the core's task slot now, so a request can tell its own answer from an older one. */
  const taskIdNow = () => rest()?.task_operation?.id ?? 0;

  /**
   * One close through the consequence flow. `targetId` names the SSH device the
   * pane or tab lives on, and the close goes there as `remote_control`; a
   * local close is the core's own `close_pane`/`close_tab`.
   */
  const requestClose = (kind: "pane" | "tab", id: string, panes: Tab["panes"], targetId: string | null) => {
    const consequence = closeScope(rest(), panes.map((p) => p.id));
    const decision = closeDecision(consequence);
    if (decision.action === "status_unknown") {
      ui().setNotice({ text: statusUnknownNotice(decision.label, translate), refreshable: true });
      return;
    }
    // An agent with live descendants outside what closes asks the subtree
    // sheet instead of the Stop-work one (PRD close-agent-subtree B2); which
    // of the two shows is re-derived while it is open (B28).
    if (decision.action === "confirm" || subtreeOf(consequence, everyAgent())) {
      ui().setPendingClose({ kind, id, targetId });
      return;
    }
    sendClose(kind, id, targetId, false);
  };

  /**
   * What an open close sheet is about, as the snapshot has it now: the
   * target's panes and the agent rows of its host, or null once the pane or
   * tab is gone or its device is no longer connected, which is the one thing
   * that closes the sheet by itself (B28, D-40).
   */
  const closeTarget = (pending: PendingClose): { panes: Tab["panes"]; agents: AgentRow[] } | null => {
    const status = pending.targetId ? rest()?.status?.remote?.find((row) => row.target_id === pending.targetId) : null;
    if (pending.targetId && status?.state !== "connected") return null;
    const workspaces = pending.targetId ? status?.session?.workspaces : rest()?.navigator?.workspaces;
    const tabs = (workspaces ?? []).flatMap((row) => row.checkouts).flatMap((row) => row.tabs);
    const panes = pending.kind === "tab" ? (tabs.find((tab) => tab.id === pending.id)?.panes ?? []) : tabs.flatMap((tab) => tab.panes).filter((pane) => pane.id === pending.id);
    if (panes.length === 0) return null;
    return { panes, agents: pending.targetId ? (status?.session?.agents ?? []) : useShellStore.getState().agents };
  };

  /** Every current agent row, this machine's and each connected device's, as the lists draw them. */
  const everyAgent = (): AgentRow[] => allAgents(rest()?.status?.remote, useShellStore.getState().agents, rest()?.navigator?.agent_scope, rest()?.navigator?.devices).map((row) => row.agent);

  const sendClose = (kind: "pane" | "tab", id: string, targetId: string | null, confirmed: boolean) => {
    if (targetId) {
      sendRemote(targetId, kind === "pane" ? { action: "close_pane", pane_id: id, confirmed } : { action: "close_tab", tab_id: id, confirmed });
      return;
    }
    dispatch({
      schema_version: 2,
      kind: kind === "pane" ? "close_pane" : "close_tab",
      payload: kind === "pane" ? { pane_id: id, confirmed } : { tab_id: id, confirmed },
    });
  };

  /**
   * A project or checkout row. On a selected SSH device a project can hold
   * several Herdr workspaces, and the checkout names the one to focus.
   */
  const focusCheckout = (workspaceId: string, checkoutId: string, expanded?: FocusCheckoutPayload["expanded"], projectExpanded?: boolean) => {
    const context = remoteContext(rest());
    if (context) {
      const host = remoteHost("shell.command.switchWorkspace");
      if (!host) return;
      const project = context.session?.workspaces.find((row) => row.id === workspaceId);
      const checkout = project?.checkouts.find((row) => row.id === checkoutId);
      if (!checkout) {
        return diagnostic(`focus_workspace: ${checkoutId} is not on ${context.device.id}`);
      }
      sendRemote(host.targetId, { action: "focus_workspace", workspace_id: workspaceId, checkout_id: checkoutId });
      return;
    }
    dispatch({ schema_version: 2, kind: "focus_checkout", payload: { workspace_id: workspaceId, checkout_id: checkoutId, ...(expanded === undefined ? {} : { expanded }), ...(projectExpanded === undefined ? {} : { project_expanded: projectExpanded }) } });
  };

  /**
   * Keyboard focus to one pane. The pane's own id says which host it is on,
   * so a remote pane is focused there and a local one here, whatever the
   * context was when the click started.
   */
  const focusPane = (paneId: string) => {
    const targetId = remoteTargetOfPane(rest(), paneId);
    if (targetId) {
      sendRemote(targetId, { action: "focus_pane", pane_id: paneId });
      return;
    }
    if (remoteContext(rest())) return diagnostic(`focus_pane: ${paneId} is not on the selected device`);
    dispatch({ schema_version: 2, kind: "focus_pane", payload: { pane_id: paneId, origin: "operator" } });
  };

  /**
   * The front Workspace's lane (PRD right-lane D-11, D-12): one
   * `workspace_view` event naming only what changes, each field the value it
   * should end at, so the same event sent twice lands where it did once. The
   * core keeps the lane per Workspace, so another Workspace is never touched.
   */
  const setWorkspaceView = (patch: { tool?: Tool | null; pinned?: boolean; tree?: boolean; prs_only_checkout?: boolean; lane_width?: number; panel_width?: number; tree_width?: number; width_request_id?: string; reveal?: string }) => {
    if (!workspaceViewOf(rest())) return diagnostic("workspace_view: no Workspace in front");
    return dispatch({ schema_version: 2, kind: "workspace_view", payload: patch });
  };

  /** Whether the lane is on screen now with this tool, as the Workspace screen draws it. */
  const laneUpWith = (tool: LaneTool) => {
    const drawn = ui().drawnLane;
    return drawn.tool === tool && drawn.mode !== "closed" && drawn.mode !== "hidden";
  };

  /** Whether the lane draws the 보기 slot rather than a tool (PRD sidebar-task-lens D-50). */
  const detailShown = () => {
    const detail = rest()?.ui_state?.detail;
    return Boolean(detail?.shown && detail.items.length > 0);
  };

  /**
   * Opens the lane on one tool (B2, B16): an unpinned lane comes up, and
   * nothing is sent when it already holds the tool. A lane drawing the 보기
   * slot turns to the tool, the slot staying on the rail (sidebar-task-lens B46).
   */
  const showTool = (tool: Tool) => {
    const view = workspaceViewOf(rest());
    if (!view) return diagnostic("workspace_view: no Workspace in front");
    ui().showLane();
    if (view.tool !== tool || detailShown()) setWorkspaceView({ tool });
  };

  /** Puts `item` in the 보기 slot in front (B14, B15): the Workspace, its Agent tab and the keyboard stay. */
  const openDetail = (item: import("./snapshot").DetailItem) => {
    if (!workspaceViewOf(rest())) return diagnostic("detail_open: no Workspace in front");
    ui().showLane();
    dispatch({ schema_version: 2, kind: "detail_open", payload: { item } });
  };

  /** Whether a Workspace is in front to draw the lane, rather than Home (B17). */
  const workspaceInFront = () => ui().screen?.kind === "workspace" && Boolean(frontCheckout(rest()) && workspaceViewOf(rest()));

  /** Closes the lane (B2, B7): the close button, or a tool's rail icon or chord while it shows. */
  const closeLane = () => {
    ui().putLaneAway();
    setWorkspaceView({ tool: null });
  };

  /** A rail icon (B2, B20): the tool showing closes the lane, otherwise the lane opens on it. */
  const toggleTool = (tool: Tool) => (laneUpWith(tool) ? closeLane() : showTool(tool));
  /** ⌘E and the path's menu (B20): the lane closes while it shows, and otherwise comes back on the tool it last showed, Files for a Workspace whose lane never showed one. */
  const toggleLane = () => {
    const { mode } = ui().drawnLane;
    if (mode !== "closed" && mode !== "hidden") return closeLane();
    showTool(workspaceViewOf(rest())?.last_tool ?? "files");
  };

  /**
   * The tree button or ⌘⇧E (B9, B11): with the files tool on screen the tree
   * turns on or off; otherwise the lane opens on the files tool with its tree.
   */
  const toggleFileTree = () => {
    const view = workspaceViewOf(rest());
    if (!view) return diagnostic("toggle file tree: no Workspace in front");
    if (!laneUpWith("files")) {
      ui().showLane();
      return setWorkspaceView({ tool: "files", tree: true });
    }
    // With no view the tree fills the lane whatever is stored, so a press
    // there would only flip a setting the operator cannot see (B12).
    if ((view.layout?.display_count ?? 0) === 0) return diagnostic("toggle file tree: the tree fills a lane with no view");
    setWorkspaceView({ tree: !view.tree });
  };

  /** The pin (B5): pinned, the lane docks; unpinned, it stays up over Agent Views until the operator closes it or puts it away with Escape. */
  const setLanePinned = (pinned: boolean) => {
    if (!pinned) ui().showLane();
    setWorkspaceView({ pinned });
  };

  /**
   * The folder chain to an opened file under the focused checkout, so the
   * Explorer shows and highlights the row without the core's `reveal_path`,
   * which would open a pinned tab (B3).
   */
  const revealAncestors = (path: string) => {
    // The Explorer tool is the Workspace's to show or hide (B10); opening a
    // file only unfolds its folders, so the row is highlighted whenever the
    // tool is open.
    const state = rest()?.ui_state;
    const context = explorerContext(rest());
    const root = context.checkout?.path;
    if (!state || !root || !path.startsWith(`${root}/`)) return;
    const parts = path.slice(root.length + 1).split("/");
    parts.pop();
    const expanded = new Set(context.expanded);
    for (let depth = 1; depth <= parts.length; depth += 1) {
      const ancestor = `${root}/${parts.slice(0, depth).join("/")}`;
      expanded.add(ancestor);
    }
    updateUiState(expandedPatch(context.device, [...expanded]));
  };

  /** The ui_state field that holds one device's expanded folders. */
  const expandedPatch = (device: string, paths: string[]) =>
    device === localDeviceId(rest())
      ? { expanded_paths: paths }
      : { device_expanded_paths: { ...(rest()?.ui_state?.device_expanded_paths ?? {}), [device]: paths } };

  /** The checkout the Explorer and the palette act on, on the selected device. */
  const explorerHere = () => {
    const context = explorerContext(rest());
    return context.checkout ? { checkout: context.checkout, device: context.device } : null;
  };

  /**
   * `file_open` for a path in the checkout in front, naming its device.
   * `beside` asks for a pinned display in the area next to the active one,
   * or a new area to its right (S7 B4); the core places it.
   */
  const openInFront = (path: string, preview: boolean, what: string, beside = false) => {
    const here = explorerHere();
    if (!here) return diagnostic(`${what}: no focused checkout`);
    revealAncestors(path);
    followOpen(path, "file");
    dispatch({
      schema_version: 2,
      kind: "file_open",
      payload: {
        path,
        workspace_id: here.checkout.workspace_id,
        checkout_id: here.checkout.id,
        preview: beside ? false : preview,
        ...(beside ? { beside: true } : {}),
        ...deviceField(here.device, localDeviceId(rest())),
      },
    });
  };

  /**
   * The keyboard follows what the operator opened into the view that shows
   * it once the core places it (B20), so ⌘W, ⌘S and typing act on that file
   * rather than on the tree, row, palette or terminal it was opened from.
   * `workspace` names where the open lands when it is not the front one.
   */
  const followOpen = (path: string, kind: "file" | "diff", committed: boolean | null = null, workspace = frontViewWorkspace()) => {
    if (workspace) ui().setViewFocusRequest({ workspace: workspaceKey(workspace), opened: { path, kind, committed } });
  };

  /**
   * A page the operator opened takes the keyboard the same way (B20): the
   * open names itself, and the core's answer names the display it shows.
   * Returns the `request_id` the `browser_open` carries.
   */
  const followBrowserOpen = (workspace: ViewWorkspace): string => {
    const requestId = `follow:${remoteRequestId()}`;
    ui().setViewFocusRequest({ workspace: workspaceKey(workspace), browserOpen: requestId });
    return requestId;
  };

  /**
   * The View areas the operator acted on (contract 4.1): the frame the page
   * has drawn, else the front Workspace's as the store holds it, for a chord
   * while the Views are hidden. Every View action names this frame's
   * Workspace and reads its ids from this frame's layout.
   */
  const frameNow = (): ViewFrame | null => {
    const drawn = drawnViews();
    if (drawn) return { workspace: drawn.workspace, layout: drawn.layout };
    const view = workspaceViewOf(rest());
    return view?.layout ? { workspace: { device_id: view.device_id, path: view.path }, layout: view.layout } : null;
  };

  /** The Workspace in front, whose View areas an open lands in. */
  const frontViewWorkspace = (): ViewWorkspace | null => {
    const view = workspaceViewOf(rest());
    return view ? { device_id: view.device_id, path: view.path } : null;
  };

  /** The frame an action acts on, or null with a diagnostic naming the action. */
  const frameFor = (action: string): ViewFrame | null => {
    const frame = frameNow();
    if (!frame) diagnostic(`view_layout ${action}: no View areas in front`);
    return frame;
  };

  /**
   * One `view_layout` action (S7 contract 4.1): one operator action is one
   * event, naming the Workspace of the frame it was taken on, and the core
   * applies it whole or refuses it.
   */
  const viewLayout = (frame: ViewFrame, action: { action: string } & Record<string, unknown>) =>
    dispatch({ schema_version: 2, kind: "view_layout", payload: viewLayoutPayload(frame.workspace, action) });

  /**
   * Moves the keyboard to a display or an area once the core shows it there
   * (S7 B20). A display is asked for from where it stands in this frame, so
   * a move or split of the active view waits for the frame that lands it.
   */
  const followFocus = (frame: ViewFrame, target: { displayId: string } | { areaId: string }) => {
    const workspace = workspaceKey(frame.workspace);
    if ("areaId" in target) return ui().setViewFocusRequest({ workspace, areaId: target.areaId });
    const located = locateDisplay(frame.layout.root, target.displayId);
    ui().setViewFocusRequest({ workspace, displayId: target.displayId, from: located ? { areaId: located.area.id, index: located.index } : null });
  };

  const focusView = (displayId: string, moveKeyboard = false) => {
    const frame = frameFor("focus");
    if (!frame) return;
    if (moveKeyboard) ui().setViewFocusRequest({ workspace: workspaceKey(frame.workspace), displayId, from: null });
    viewLayout(frame, { action: "focus", display_id: displayId });
  };

  const focusViewArea = (areaId: string) => {
    const frame = frameFor("focus_area");
    if (!frame) return;
    followFocus(frame, { areaId });
    viewLayout(frame, { action: "focus_area", area_id: areaId });
  };

  /** A display into `areaId` at `index`, its final position there; a reorder when it is already there. */
  const moveView = (displayId: string, areaId: string, index: number) => {
    const frame = frameFor("move");
    if (!frame) return;
    followFocus(frame, { displayId });
    viewLayout(frame, { action: "move", display_id: displayId, area_id: areaId, index });
  };

  /** A new area at `edge` of `areaId`, taking half of it, with the display moved in (S7 B7, B9). */
  const splitView = (displayId: string, areaId: string, edge: Edge) => {
    const frame = frameFor("split");
    if (!frame) return;
    followFocus(frame, { displayId });
    // A repeated request id is a no-op in the core, so a split that arrives
    // twice splits once (S7 B18).
    viewLayout(frame, { action: "split", display_id: displayId, area_id: areaId, edge, request_id: remoteRequestId() });
  };

  /** A split's ratio after a divider drag or a keyboard step; one event per landing (S7 B9). */
  const resizeViewSplit = (splitId: string, ratio: number) => {
    const frame = frameFor("resize");
    if (frame) viewLayout(frame, { action: "resize", split_id: splitId, ratio });
  };

  /** The active View area's active display, which a chord acts on. */
  const activeDisplayNow = () => {
    const frame = frameNow();
    return frame ? activeDisplay(frame.layout) : null;
  };

  /**
   * The contents a save or a close would send for one document, or null: the
   * newest keystroke, else the document a display shows (the `documents`
   * section carries only the documents on screen).
   */
  const draftFor = (tabId: string): string | null =>
    latestDraft(tabId) ?? useShellStore.getState().documents[tabId]?.contents_utf8 ?? null;

  /** Saves the document of the active display, or the named document when one is given. */
  const saveFile = (tabId?: string) => {
    const state = useShellStore.getState();
    const tab = editorTabFor(state.editor, tabId ?? state.editor?.active_tab_id ?? null);
    if (!tab || tab.kind !== "file") {
      diagnostic("file_save: no showing document");
      return false;
    }
    // A read-only or preview-only document has nothing the core would accept:
    // a save of it is refused, and the refusal would mark it dirty.
    const document = state.documents[tab.id] ?? null;
    if (document && (document.readonly_reason !== null || document.contents_utf8 === null)) {
      diagnostic(`file_save: ${tab.path} is not editable here`);
      return false;
    }
    const contents = draftFor(tab.id);
    if (contents === null) {
      diagnostic(`file_save: no contents for ${tab.path}`);
      return false;
    }
    // A device file's path means nothing to this machine's checkout roots,
    // so the save names the device and the daemon hands it to the core,
    // which writes only to the place the tab was opened from.
    const device = deviceOfCheckout(state.rest, tab.checkout_id);
    const sent = dispatch({
      schema_version: 2,
      kind: "file_save",
      payload: {
        tab_id: tab.id,
        path: tab.path,
        // The core checks the save against the revision it read when it
        // opened the file; the shell sends only the draft.
        contents_utf8: contents,
        ...deviceField(device, localDeviceId(rest())),
      },
    });
    if (sent === false) return false;
    noteSent(tab.id, contents);
    return true;
  };

  /**
   * `changes_select` for a History row of the checkout in front (S4, S7
   * contract 4.2): the core places a diff by the rules a file open follows,
   * so the lane opens on the files tool to show it (PRD right-lane B16).
   */
  const selectChangeIn = (path: string, committed: boolean, preview: boolean, beside: boolean) => {
    const here = explorerContext(rest()).checkout;
    const changes = useShellStore.getState().changes;
    const scope = rest()?.navigator?.changes_root_path;
    if (!here || !scope || changes?.root_path !== scope || (scope !== here.path && !scope.startsWith(`${here.path}/`))) {
      return diagnostic("changes_select: checkout is unavailable");
    }
    const group = committed ? changes.committed : changes.entries;
    if (!group.some((entry) => entry.path === path)) return diagnostic("changes_select: row is no longer available");
    followOpen(path, "diff", committed);
    dispatch({ schema_version: 2, kind: "changes_select", payload: { path, committed, preview, ...(beside ? { beside: true } : {}) } });
  };

  /** What one frame says about a watched close (`closeWithSaveOutcome`). */
  const closeWatchFrame = (state: ReturnType<typeof useShellStore.getState>, tabId: string): CloseWatchFrame => {
    const document = state.documents[tabId];
    const save = document?.save?.state;
    const view = workspaceViewOf(state.rest);
    const layout = view?.layout;
    return {
      connection: state.connection,
      hostId: state.daemon?.host_id,
      tabIds: (state.editor?.tabs ?? []).map((row) => row.id),
      deviceIds: (state.rest?.navigator?.devices ?? []).map((row) => row.id),
      node: localDeviceId(state.rest),
      workspace: view ? workspaceKey(view) : null,
      displayIds: layout ? areasOf(layout.root).flatMap((area) => area.displays.map((display) => display.id)) : [],
      error: state.rest?.status?.last_error ?? null,
      draft: latestDraft(tabId),
      save: !document ? "hidden" : save === "saving" || save === "waiting" || save === "checking" ? "in_flight" : "settled",
    };
  };

  /**
   * Settles the stored draft of a document whose last display this frame
   * closes (D-14). A close that carries a save is watched until it lands,
   * or until nothing more can come of it, and meanwhile the document is not
   * autosaved or saved on leaving, since the close carries that save. A
   * close with nothing to carry deletes a stored draft only when it matches
   * what the core holds, once this tab's queued write has landed.
   */
  const watchDocumentClose = (
    tab: EditorTabSnapshot,
    target: { workspace: ViewWorkspace; displayId: string },
    pending: { tab_id: string; path: string; contents_utf8: string } | null,
    document: EditorDocumentSnapshot | null,
  ) => {
    const state = useShellStore.getState();
    const draftKey = tabBufferKey(state.daemon?.host_id, state.rest, tab);
    if (!pending) {
      if (!draftKey) return;
      const known = document ? { contents_utf8: document.contents_utf8, dirty: document.dirty } : null;
      void settledBuffer(draftKey).then((stored) => {
        const decision = storedDraftOnClose(stored, known);
        if (decision === "delete") void deleteBuffer(draftKey);
        else if (decision === "keep") diagnostic(`view_layout close: the stored draft of ${tab.path} is kept as a recovery item`);
      });
      return;
    }
    // The draft goes when the close lands, not when it is asked for: a
    // close the core refuses, one that turns out not to be the last view, or
    // a tab that leaves for another reason keeps the recovery copy.
    let watch: CloseWatch = {
      tabId: tab.id,
      workspace: workspaceKey(target.workspace),
      displayId: target.displayId,
      hostId: state.daemon?.host_id,
      device: deviceOfCheckout(state.rest, tab.checkout_id),
      errorAt: state.rest?.status?.last_error?.occurred_at ?? null,
      contents: pending.contents_utf8,
      sawSave: false,
    };
    noteClosing(tab.id, true);
    const unsubscribe = useShellStore.subscribe((next, previous) => {
      // Only a frame that can change the answer is read; terminal output is not one.
      if (
        next.editor === previous.editor &&
        next.documents === previous.documents &&
        next.rest === previous.rest &&
        next.connection === previous.connection &&
        next.daemon === previous.daemon
      ) {
        return;
      }
      const { outcome, sawSave } = closeWithSaveOutcome(watch, closeWatchFrame(next, tab.id));
      watch = { ...watch, sawSave };
      if (outcome === "wait") return;
      unsubscribe();
      noteClosing(tab.id, false);
      if (outcome === "landed" && draftKey) void deleteBuffer(draftKey);
      else if (outcome === "keep") diagnostic(`view_layout close: ${tab.path} did not close; its draft is kept`);
    });
  };

  /**
   * Closes one display (S7 B5, contract 4.1). The document's unsaved text
   * rides every close as `pending_save`, since the core and not this frame
   * knows whether the display is the document's last; on any other display
   * the core drops it and the document keeps its draft and dirty state. When
   * this frame shows the document's last display, the close is watched, and
   * unsaved work this page cannot reproduce is not sent at all: a notice
   * offers Don't save, the only close that drops unsaved text.
   */
  let stopCloseFocus: (() => void) | null = null;
  const followViewClose = (frame: ViewFrame, displayId: string) => {
    stopCloseFocus?.();
    stopCloseFocus = null;
    const located = locateDisplay(frame.layout.root, displayId);
    const owner = keyboardCommandOwner();
    const checkout = frontCheckout(rest());
    if (!located || owner.kind !== "view" || owner.workspace !== checkout?.id) return;
    if (located.area.active !== displayId || owner.areaId !== located.area.id) {
      // Clicking a background X borrows native keyboard response even when
      // preventDefault keeps the shell's DOM focus. Give the visible page back.
      const inputArea = findArea(frame.layout.root, owner.areaId);
      const page = inputArea?.displays.find((display) => display.id === inputArea.active && display.kind === "browser");
      if (page) focusBrowserDisplay(workspaceKey(frame.workspace), page.id);
      return;
    }
    const revision = keyboardInputRevision();
    const workspace = workspaceKey(frame.workspace);
    const errorAt = rest()?.status?.last_error?.occurred_at;
    const finish = () => { stop(); if (stopCloseFocus === finish) stopCloseFocus = null; };
    // One pending close follow, ended on removal, newer input, leaving the
    // Workspace or disconnect. The core alone selects the MRU replacement.
    const stop = useShellStore.subscribe((next) => {
      const view = workspaceViewOf(next.rest);
      if (keyboardInputRevision() !== revision || !view || workspaceKey(view) !== workspace || next.connection !== "live" || next.rest?.status?.last_error?.occurred_at !== errorAt) {
        finish();
        return;
      }
      if (!view.layout || locateDisplay(view.layout.root, displayId)) return;
      finish();
      requestAnimationFrame(() => {
        if (keyboardInputRevision() !== revision) return;
        const current = workspaceViewOf(rest());
        if (!current?.layout || workspaceKey(current) !== workspace) return;
        const area = findArea(current.layout.root, located.area.id) ?? findArea(current.layout.root, current.layout.active_area);
        const screen = document.querySelector<HTMLElement>(`[data-workspace-screen="${CSS.escape(checkout.id)}"]`);
        if (current.layout.display_count > 0 && area) {
          const element = screen?.querySelector<HTMLElement>(`[data-view-area-id="${CSS.escape(area.id)}"]`);
          const page = element?.querySelector<HTMLElement>("[data-browser-slot]")?.dataset.browserSlot;
          if (page) focusBrowserDisplay(workspace, page);
          else {
            const content = element?.querySelector<HTMLElement>('.cm-content');
            if (content) content.focus({ preventScroll: true });
            else ui().setViewFocusRequest({ workspace, areaId: area.id, revision });
          }
        } else if (current.tool === "files" && current.tree) {
          screen?.querySelector<HTMLElement>("[data-explorer-tree]")?.focus({ preventScroll: true });
        } else {
          const pane = useShellStore.getState().focusedPaneId;
          if (pane && screen?.querySelector(`[data-pane-view="${CSS.escape(pane)}"]`)) focusTerminal(pane);
        }
      });
    });
    stopCloseFocus = finish;
  };

  const closeView = (displayId: string) => {
    const frame = frameFor("close");
    if (!frame) return;
    const located = locateDisplay(frame.layout.root, displayId);
    if (!located) return diagnostic(`view_layout close: ${displayId} is not open`);
    const state = useShellStore.getState();
    const tab = editorTabFor(state.editor, located.display.tab_id);
    if (!tab || tab.kind !== "file") {
      followViewClose(frame, displayId);
      if (viewLayout(frame, { action: "close", display_id: displayId }) === false) stopCloseFocus?.();
      return;
    }
    const last = displaysOfDocument(frame.layout.root, tab.id).length === 1;
    const document = state.documents[tab.id] ?? null;
    const carries = documentCloseCarries({ draft: latestDraft(tab.id), document, dirty: tab.dirty });
    if (carries.kind === "unloaded" && last) {
      ui().setNotice({
        text: translate("shell.unloadedChanges", { name: tab.label }),
        refreshable: false,
        dontSave: { workspace: frame.workspace, displayId },
      });
      return;
    }
    const pending = carries.kind === "save" ? { tab_id: tab.id, path: tab.path, contents_utf8: carries.contents } : null;
    followViewClose(frame, displayId);
    if (last) watchDocumentClose(tab, { workspace: frame.workspace, displayId }, pending, document);
    if (viewLayout(frame, { action: "close", display_id: displayId, ...(pending ? { pending_save: pending } : {}) }) === false) stopCloseFocus?.();
  };

  /**
   * The operator's explicit Don't save (contract 4.1): the view closes, and
   * with it its document without the unsaved text, on the Workspace the
   * close was asked on. A copy stored in this browser stays a recovery item.
   */
  const closeViewWithoutSaving = (target: { workspace: ViewWorkspace; displayId: string }) => {
    ui().setNotice(null);
    const frame = frameNow();
    if (frame && workspaceKey(frame.workspace) === workspaceKey(target.workspace)) followViewClose(frame, target.displayId);
    if (dispatch({ schema_version: 2, kind: "view_layout", payload: viewLayoutPayload(target.workspace, { action: "close", display_id: target.displayId, discard: true }) }) === false) stopCloseFocus?.();
  };

  /** A preview display becomes an ordinary one (B2): the named one, or the active one. */
  const keepViewOpen = (displayId?: string) => {
    const frame = frameNow();
    if (!frame) return;
    const located = displayId ? locateDisplay(frame.layout.root, displayId) : activeDisplay(frame.layout);
    if (!located?.display.preview) return;
    viewLayout(frame, { action: "keep_open", display_id: located.display.id });
  };

  /** Selects a file in the OS file manager through the desktop host; refusals are the host's to log. */
  const revealExternal = (path: string) => {
    const bridge = hostBridge();
    if (!bridge) return diagnostic("reveal externally: this host has no file manager");
    bridge.revealPath(path);
  };

  /** Opens a folder of this computer as a VS Code window through the desktop host; its outcome is the host's to log (B16). */
  const openInEditor = (path: string) => {
    const bridge = hostBridge();
    if (!bridge) return diagnostic("open in VS Code: this host has no desktop app");
    bridge.openInEditor(path);
  };

  /** Shows the files tool's tree with a file's row unfolded and selected; nothing is opened (B16). */
  const revealInExplorer = (path: string) => {
    setWorkspaceView({ reveal: path });
    ui().setExplorerSelection(path);
  };

  /** One command of a display's tab menu, its area's overflow menu or the palette (B11, B20). */
  const runViewMenu = (id: ViewMenuId, displayId: string) => {
    const frame = frameFor(id);
    const located = frame ? locateDisplay(frame.layout.root, displayId) : null;
    if (!frame || !located) return diagnostic(`view menu ${id}: ${displayId} is not open`);
    const edge = menuEdge(id);
    if (id === "keep_open") return keepViewOpen(displayId);
    if (id === "copy_path") {
      const text = located.display.kind === "browser" ? (located.display.url ?? "") : located.display.path;
      return void navigator.clipboard?.writeText(text).catch(() => undefined);
    }
    if (id === "select_in_tree") return revealInExplorer(located.display.path);
    if (id === "reveal_external") return revealExternal(located.display.path);
    if (id === "close_view") return closeView(displayId);
    if (!edge) return;
    if (id.startsWith("split_")) return splitView(displayId, located.area.id, edge);
    const target = findArea(frame.layout.root, neighbourArea(frame.layout.root, located.area.id, edge) ?? "");
    if (!target) return diagnostic(`view menu ${id}: no view area lies ${edge} of ${located.area.id}`);
    moveView(displayId, target.id, target.displays.length);
  };

  /**
   * The View area commands (PRD cmdk-navigation D-05): focus to the next or
   * previous area, or grow or shrink the one in use. One that cannot run now
   * is not sent and says why in the diagnostic log.
   */
  const runViewArea = (step: ViewAreaStep) => {
    const frame = frameFor(step);
    if (!frame) return;
    const drawn = drawnViews();
    const reason = viewAreaStepUnavailable(frame.layout, drawn, step);
    if (reason) return diagnostic(`view area ${step}: ${reason}`);
    if (step === "focus_next" || step === "focus_previous") {
      const next = adjacentInOrder(frame.layout.root, frame.layout.active_area, step === "focus_next" ? 1 : -1);
      if (next) focusViewArea(next.id);
      return;
    }
    const target = resizeTarget(frame.layout, drawn?.geometry ?? null, step === "grow");
    if ("ratio" in target) resizeViewSplit(target.splitId, target.ratio);
  };

  const createTab = (areaId?: string) => {
      const view = workspaceViewOf(rest());
      if (view?.agent_layout?.groups) {
        agentGroup({ device_id: view.device_id, path: view.path }, view.agent_layout.group_id ?? null, { action: "new_tab", terminal: true, ...(areaId ? { area_id: areaId } : {}) });
        return;
      }
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.newTab");
        if (!host) return;
        const checkout = host.view?.checkout;
        if (!checkout) return diagnostic("create_tab: the remote device has no workspace open");
        sendRemote(host.targetId, { action: "create_tab", workspace_id: checkout.workspace_id, checkout_id: checkout.id, cwd: checkout.path, label: checkout.next_tab_label });
        return;
      }
      const here = current();
      if (!here) return diagnostic("create_tab: no focused checkout");
      const requestId = remoteRequestId();
      dispatch({
        schema_version: 2,
        kind: "create_tab",
        payload: { workspace_id: here.checkout.workspace_id, checkout_id: here.checkout.id, label: here.checkout.next_tab_label, request_id: requestId, ...(areaId ? { area_id: areaId } : {}) },
      });
      armKeyTarget(requestId);
    };

  const closeTab = (tabId: string) => {
      const frame = areaFrame("agent");
      if (frame?.layout.drafts?.includes(tabId)) {
        const request = agentGroup(frame.workspace, frame.layout.group_id ?? null, { action: "close_draft", tab_id: tabId });
        useInlineStarts.getState().noteClose(request, frame.workspace.device_id, frame.workspace.path, tabId);
        return;
      }
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.closeTab");
        if (!host) return;
        const tab = host.view?.checkout.tabs.find((row) => row.id === (tabId ?? host.view?.tab?.id));
        if (!tab?.id) return diagnostic("close_tab: no visible remote tab");
        requestClose("tab", tab.id, tab.panes, host.targetId);
        return;
      }
      const here = current();
      const id = tabId ?? here?.tab?.id;
      if (!here || !id) return diagnostic("close_tab: no visible tab");
      const tab = here.checkout.tabs.find((row) => row.id === id);
      requestClose("tab", id, tab?.panes ?? [], null);
    };

  const agentGroup = (workspace: ViewWorkspace, groupId: string | null, action: { action: string } & Record<string, unknown>, requestId = remoteRequestId()) => {
    if (["select", "create", "new_tab", "move", "dissolve", "undo"].includes(action.action)) {
      const checkout = catalogWorkspaces(rest()).filter((project) => project.device_id === workspace.device_id).flatMap((project) => project.checkouts).find((checkout) => checkout.path === workspace.path);
      if (checkout) beginOpening({ checkoutId: checkout.id, deviceId: workspace.device_id, path: checkout.path, workspaceId: checkout.workspace_id });
    }
    useGroupFeedback.setState({ request: { workspace, id: requestId } });
    dispatch({ schema_version: 2, kind: "agent_group", payload: { workspace, group_id: groupId, request_id: requestId, ...action } });
    if (action.action === "new_tab" && action.terminal === true && workspace.device_id === localDeviceId(rest())) armKeyTarget(requestId);
    return requestId;
  };
  const agentLayout = (action: { action: string } & Record<string, unknown>) => {
    const frame = areaFrame("agent");
    if (!frame) return diagnostic("agent_layout: no Agent areas are drawn");
    dispatch({ schema_version: 2, kind: "agent_layout", payload: { workspace: frame.workspace, group_id: frame.layout.group_id ?? null, ...action } });
  };
  const runAgentCommand = (command: AgentCommand, tabId?: string) => {
    const frame = areaFrame("agent");
    if (!frame) return;
    const id = tabId ?? activeAgentDisplay(frame.layout)?.display.id;
    // The area steps are the keyboard's and name no tab; every other command is a tab menu's.
    const step = command === "focus_next" || command === "focus_previous" || command === "grow" || command === "shrink" ? (command as AgentAreaStep) : null;
    if (step) {
      if (agentAreaStepUnavailable(frame, step)) return;
    } else {
      const entry = tabId ? agentMenu(frame, tabId).find((row) => row.id === command) : undefined;
      if (!entry || entry.unavailable) return;
    }
    if (command === "new_tab") return createTab(id ? locateAgentTab(frame.layout.root, id)?.area.id : undefined);
    if (command === "focus_next" || command === "focus_previous") {
      const area = adjacentAgentArea(frame.layout.root, frame.layout.active_area, command === "focus_next" ? 1 : -1);
      if (area) agentLayout({ action: "focus_area", area_id: area.id });
      return;
    }
    if (command === "grow" || command === "shrink") {
      const target = resizeAgentTarget(frame.layout, frame.geometry, command === "grow", AGENT_WORDS);
      if (!("reason" in target)) agentLayout({ action: "resize", split_id: target.splitId, ratio: target.ratio });
      return;
    }
    if (!id) return;
    const located = locateAgentTab(frame.layout.root, id);
    if (!located) return;
    if (command.startsWith("split_")) agentLayout({ action: "split", tab_id: id, area_id: located.area.id, edge: command.slice(6), request_id: remoteRequestId() });
    if (command.startsWith("move_")) {
      const area = neighbourAgentArea(frame.layout.root, located.area.id, command.slice(5) as AgentEdge);
      if (area) agentLayout({ action: "move", tab_id: id, area_id: area, index: findAgentArea(frame.layout.root, area)?.displays.length ?? 0 });
    }
    if (command === "copy_name") {
      const checkout = remoteContext(rest()) ? remoteView(remoteContext(rest())!.session, rest()?.workspace_view)?.checkout : current()?.checkout;
      const label = checkout?.strip.find((row) => row.source_id === id)?.label;
      if (label) void navigator.clipboard?.writeText(label);
    }
    if (command === "close_tab") closeTab(id);
  };

  return {
    dispatch,
    revealAncestors,
    taskIdNow,

    openSettings(tab: SettingsTab = "general") {
      ui().openSettings(tab);
    },

    setAccent(hex: string) {
      updateUiState({ accent_hex: hex });
    },

    /** One typed event; the core owns the shared interface choice. */
    setInterfaceLanguage(language: InterfaceLanguage | null) {
      const payload = { language } satisfies InterfaceLanguageSetPayload;
      dispatch({ schema_version: 2, kind: "interface_language_set", payload });
    },
    /** One typed event; the core owns the choice and stores it (D-15). */
    setTheme(theme: ThemeChoice) {
      dispatch({ schema_version: 2, kind: "theme_set", payload: { theme } });
    },
    /** One typed event; the core owns the color theme and stores it (hide-color-theme D-08). */
    setColorTheme(color_theme: ColorThemeSetPayload["color_theme"]) {
      const payload = { color_theme } satisfies ColorThemeSetPayload;
      dispatch({ schema_version: 2, kind: "color_theme_set", payload });
    },

    /** One typed event; the core owns whether list rows and tabs draw the agent's logo (PRD agent-logo-view-mode D-07). */
    setAgentLogoMode(mode: AgentLogoMode) {
      const payload = { mode } satisfies AgentLogoModeSetPayload;
      dispatch({ schema_version: 2, kind: "agent_logo_mode_set", payload });
    },

    /** Sleep idle agents after `hours`; null is Never (PRD agent-sleep B2). */
    setAgentSleepAfter(hours: number | null) {
      dispatch({ schema_version: 2, kind: "agent_sleep_set", payload: { after_hours: hours } });
    },

    /** Sleep agent from the pane menu (B15). */
    sleepAgent(paneId: string) {
      dispatch({ schema_version: 2, kind: "agent_sleep", payload: { pane_id: paneId } });
    },

    /**
     * `세션 보관` (PRD session-archive B5-B7): a session that is not working,
     * or waits on the operator, goes at once; one with work running asks
     * first with what is working.
     */
    archiveSession(paneId: string) {
      const byPane = agentsByPane(allPaneAgents(useShellStore.getState().agents, rest()?.status?.remote));
      const root = byPane.get(paneId);
      if (root && archiveWorking(root, byPane).length > 0) return ui().setPendingArchive({ paneId });
      dispatch({ schema_version: 2, kind: "archive_session", payload: { pane_id: paneId, confirmed: false } });
    },
    /** 작업 멈추고 보관: the confirmation the working session asked for (B6). */
    confirmArchive() {
      const pending = ui().pendingArchive;
      if (!pending) return;
      ui().setPendingArchive(null);
      dispatch({ schema_version: 2, kind: "archive_session", payload: { pane_id: pending.paneId, confirmed: true } });
    },
    cancelArchive() {
      ui().setPendingArchive(null);
    },
    /** 이어가기: the archived root and its descendants, or the one member named (B21, B24). */
    resumeArchived(archiveId: string, sessionId?: string) {
      dispatch({ schema_version: 2, kind: "archive_resume", payload: { archive_id: archiveId, ...(sessionId ? { session_id: sessionId } : {}) } });
    },
    /** 상태 확인 after a restart left an archive or a resume unknown (B34). */
    checkArchived(archiveId: string) {
      dispatch({ schema_version: 2, kind: "archive_check", payload: { archive_id: archiveId } });
    },
    /** 보관함에서 지우기…: asks first (B28). */
    deleteArchived(archiveId: string) {
      ui().setPendingArchiveDelete({ id: archiveId });
    },
    confirmArchiveDelete() {
      const pending = ui().pendingArchiveDelete;
      if (!pending) return;
      ui().setPendingArchiveDelete(null);
      dispatch({ schema_version: 2, kind: "archive_delete", payload: { archive_id: pending.id } });
    },
    keepArchived() {
      ui().setPendingArchiveDelete(null);
    },

    /**
     * Fork agent (`tab`) and Fork agent to the side (`side`) from the pane menu: a
     * new execution of this agent's conversation, the operator's own root, which
     * takes the keyboard. Keys typed before its agent starts wait for it.
     */
    forkPane(paneId: string, placement: ForkPlacement) {
      const requestId = remoteRequestId();
      dispatch({ schema_version: 2, kind: "fork_pane", payload: { pane_id: paneId, placement, origin: "operator", request_id: requestId } });
      armKeyTarget(requestId);
    },

    /** Wake agent and Retry resume the conversation; Start new session is `fresh` (B12, B14). */
    wakeAgent(paneId: string, fresh = false) {
      dispatch({ schema_version: 2, kind: "agent_wake", payload: { pane_id: paneId, fresh } });
    },

    /** A closed pane has no input/focus address. Wake the saved conversation explicitly. */
    wakeSleepingSession(sleepId: string) {
      dispatch({ schema_version: 2, kind: "wake_sleeping_session", payload: { sleep_id: sleepId } });
    },

    checkSleepingSession(sleepId: string) {
      dispatch({ schema_version: 2, kind: "check_sleeping_session", payload: { sleep_id: sleepId } });
    },

    /** Reopen a pane Hide cannot hear, in place and on the same conversation (the Not connected popover, B29). */
    reopenPane(paneId: string) {
      dispatch({ schema_version: 2, kind: "pane_reopen", payload: { pane_id: paneId } });
    },

    /** Turn off Codex's shared server on one machine (the core's own node id for this Mac), the popover's secondary link (B30). */
    turnOffCodexSharedServer(deviceId: string) {
      dispatch({ schema_version: 2, kind: "codex_daemon_disable", payload: { device_id: deviceId } });
    },

    setFontSize(size: number) {
      updateUiState({ font_size: size });
    },

    /** The width a sidebar drag or its reset lands on, sent once per gesture (PRD sidebar-typography D-09). */
    setSidebarWidth(width: number) {
      updateUiState({ sidebar_width: width });
    },

    /** `host`'s pane chords, replaced as a whole; the other host's set is untouched. */
    setPaneShortcuts(host: HostKind, bindings: Record<string, string>) {
      updateUiState(host === "electron" ? { shortcut_bindings: bindings } : { browser_shortcut_bindings: bindings });
    },

    /** Whether this page is looking at the Agents tab; the daemon owns the core's flag. */
    /** Settings > Mobile's switch; hided runs tailscale serve (PRD mobile-companion D-06). */
    setMobileEnabled(enabled: boolean) {
      dispatch({ schema_version: 2, kind: "mobile_enable", payload: { enabled } });
    },
    /** Settings > Mobile open or closed: hided rechecks the checklist while it is open; a code is made only by Show QR. */
    observeMobile(observing: boolean) {
      dispatch({ schema_version: 2, kind: "mobile_observe", payload: { observing } });
    },
    /** Show QR: hided makes one pairing code and shows it in the `mobile` frame (PRD settings-cleanup B58). */
    showMobileCode() {
      dispatch({ schema_version: 2, kind: "mobile_show_code", payload: {} });
    },
    /** Hide QR: the code leaves the frame. */
    hideMobileCode() {
      dispatch({ schema_version: 2, kind: "mobile_hide_code", payload: {} });
    },
    revokePhone(phoneId: string) {
      dispatch({ schema_version: 2, kind: "mobile_revoke", payload: { phone_id: phoneId } });
    },
    setPushMode(mode: "off" | "app_closed" | "always") {
      dispatch({ schema_version: 2, kind: "mobile_push_mode", payload: { mode } });
    },
    observeAgents(observing: boolean) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { observing } });
    },
    /** Whether this page shows the request view; while one does, the core re-reads running checks (PRD overview-request-view D-32). */
    observeRequestView(observing: boolean) {
      dispatch({ schema_version: 2, kind: "request_view", payload: { observing } });
    },
    /** A finished row opened in the request view: the pane is read, the focus stays (D-29). */
    openResult(paneId: string) {
      dispatch({ schema_version: 2, kind: "overview_open_result", payload: { pane_id: paneId } });
    },

    /**
     * Whether the window is on screen and whether the Weekly Usage popover is
     * open, the two hints the core's usage timer reads. They ride the UI-state
     * event, which replaces the whole state, so they go with its echo.
     */
    setUsagePin(payload: UsagePinSetPayload) {
      dispatch({ schema_version: 2, kind: "usage_pin_set", payload: { ...payload } });
    },
    setUsageHidden(payload: UsageHiddenSetPayload) {
      dispatch({ schema_version: 2, kind: "usage_hidden_set", payload: { ...payload } });
    },
    observeUsage(hints: UiStateUsageHints) {
      updateUiState(hints);
    },

    chooseAi(provider: string, model?: string) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: model === undefined ? { provider } : { provider, model } });
    },
    /** The `에이전트 요약` switch (PRD overview-request-view D-11): off, agent labels are neither asked for nor shown. */
    setAgentSummary(on: boolean) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { agent_summary: on } });
    },
    /** The usage limit (PRD hide-ai-usage-limit D-01): a percent, or `"none"`; the core refuses a number outside 1 to 100. */
    setAiUsageLimit(limit: number | "none") {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { usage_limit: limit } });
    },
    /** Use Hide AI (PRD settings-cleanup D-14): off, no model is asked anything and the other Hide AI values stay stored. */
    setHideAiEnabled(on: boolean) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { enabled: on } });
    },
    /** Adds an agent at the end of "If <Runs on> can't answer" (D-16). */
    addAiFallback(provider: string) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { fallback_add: provider } });
    },
    removeAiFallback(provider: string) {
      dispatch({ schema_version: 2, kind: "ai_settings", payload: { fallback_remove: provider } });
    },

    /** Reinstall on a machine's row repairs every part that needs it; a hook row names its one part. */
    reinstallKit(deviceId: string, components?: KitComponentId[], agents?: string[]) {
      dispatch({
        schema_version: 2,
        kind: "kit_reinstall",
        payload: { device_id: deviceId, ...(components ? { components } : {}), ...(agents ? { agents } : {}) },
      });
    },

    /**
     * The wizard's first step, Continue: the agents left on are switched on
     * here and on every device that waits for the choice; once the choice is
     * answered, only the agents whose switch changed move (B45). The core
     * moves the wizard on when this Mac's kit pass lands, and `finish` ends
     * it there, a browser tab's whole wizard (B26).
     */
    applyAgentOnboarding(agents: string[], finish = false) {
      dispatch({ schema_version: 2, kind: "agent_onboarding_apply", payload: { agents, finish } });
    },

    /**
     * The operator left the wizard's step `from` (Continue, Not now, Skip for now, a project added); the core moves only the step on screen.
     * `notifications` is the desktop switch the notification step leaves with: on for Continue on a Mac that already allows Hide (B16),
     * off for Not now (B21), absent to leave it as it is. The agent step leaves this way only while this Mac's pass is refused: the
     * wizard closes with the question unanswered.
     */
    advanceFirstRun(from: "agents" | "devices" | "notifications" | "project", notifications?: boolean) {
      dispatch({ schema_version: 2, kind: "first_run_advance", payload: notifications === undefined ? { from } : { from, notifications } });
    },
    backFirstRun(from: "devices" | "notifications" | "project") {
      dispatch({ schema_version: 2, kind: "first_run_back", payload: { from } });
    },
    saveFirstRunDraft(draft: { agents?: string[]; devices?: string[]; project_device?: string; project_path?: string; project_form?: import("./snapshot").FirstRunProjectForm }) {
      dispatch({ schema_version: 2, kind: "first_run_draft", payload: draft });
    },
    listHerdrProfiles() {
      dispatch({ schema_version: 2, kind: "herdr_profiles_list", payload: {} });
    },
    connectHerdrProfile(profileId: string) {
      dispatch({ schema_version: 2, kind: "herdr_profile_connect", payload: { profile_id: profileId, host_consent: true } });
    },

    /** Needs You, with an optional Done choice for the reopened wizard's Not now. */
    setProductAnalytics(enabled: boolean) {
      dispatch({ schema_version: 2, kind: "product_analytics_set", payload: { enabled } });
    },
    setDesktopNotifications(enabled: boolean, doneEnabled?: boolean) {
      dispatch({ schema_version: 2, kind: "desktop_notifications_set", payload: doneEnabled === undefined ? { enabled } : { enabled, done_enabled: doneEnabled } });
    },

    setDesktopNotificationsDone(enabled: boolean) {
      dispatch({ schema_version: 2, kind: "desktop_notifications_done_set", payload: { enabled } });
    },

    /** A screen's one-line introduction was shown, so it is not shown again (B43). */
    markScreenIntroSeen(screen: ScreenIntro) {
      dispatch({ schema_version: 2, kind: "screen_intro_seen", payload: { screen } });
    },

    /** This desktop window shows agent notifications, or stops; hided judges agent changes only while one does. */
    wantAgentNotices(wanted: boolean) {
      dispatch({ schema_version: 2, kind: "desktop_notices", payload: { wanted } });
    },

    /** An agent's switch on its machine's row (agent adapters): its skill and hook go in or come out. */
    setKitAgent(deviceId: string, agent: string, enabled: boolean) {
      dispatch({ schema_version: 2, kind: "kit_agent_set", payload: { device_id: deviceId, agent, enabled } });
    },

    /** A tab showing the kit opened: this Mac's parts are read once. */
    checkKit() {
      dispatch({ schema_version: 2, kind: "kit_check", payload: {} });
    },

    /** Add device opened: hided reads the account's ssh config Hosts once and publishes `status.ssh_hosts`. */
    listSshHosts() {
      dispatch({ schema_version: 2, kind: "ssh_hosts_list", payload: {} });
    },

    registerDevice(id: string, label: string, alias: string, options: { hostConsent: boolean; herdrSocketPath: string | null }) {
      dispatch({
        schema_version: 2,
        kind: "register_device",
        payload: { id, label, ssh_alias: alias, host_consent: options.hostConsent, herdr_socket_path: options.herdrSocketPath },
      });
    },

    /** Gives or withdraws the one consent for Hide's helper on a device (PRD S5.5 B50-B52). */
    setDeviceHostConsent(deviceId: string, allow: boolean) {
      dispatch({ schema_version: 2, kind: "device_host_consent", payload: { device_id: deviceId, allow } });
    },

    retryDeviceHost(deviceId: string) {
      dispatch({ schema_version: 2, kind: "device_host_retry", payload: { device_id: deviceId } });
    },

    testDevice(deviceId: string) {
      dispatch({ schema_version: 2, kind: "test_device", payload: { device_id: deviceId } });
    },

    retryDevice(deviceId: string) {
      dispatch({ schema_version: 2, kind: "retry_connect", payload: { target_id: deviceId } });
    },

    /**
     * Asks the process's supervisor about the core move (PRD
     * core-host-node-move B3 to B5): run the checks toward a device or back
     * to this machine, start the move once they passed, or give up its wait
     * on its other machine where the frame offers it. The answer is the
     * `core_move` frame, or a refusal naming why.
     */
    coreMove(request: { action: "check" | "start"; device: string } | { action: "check_back" | "back" | "abandon" }) {
      useShellStore.setState({ coreMoveRefusal: null });
      dispatch({ schema_version: 2, kind: "core_move", payload: request });
    },

    /** A node's window ends or restores its link to the core on another machine (B16); the node's own daemon answers. */
    coreLink(action: "disconnect" | "reconnect") {
      dispatch({ schema_version: 2, kind: "core_link", payload: { action } });
    },

    /**
     * The core closes the device's file tabs without saving, so every draft of
     * them this browser is still writing lands first and stays as a recovery
     * item (B26). A draft that could not be stored then (B44) would be lost
     * with its tab, so nothing is sent and its path is returned instead.
     */
    async removeDevice(deviceId: string): Promise<string[]> {
      const state = useShellStore.getState();
      const keys = (state.editor?.tabs ?? [])
        .filter((tab) => tab.checkout_id.startsWith(`remote:${deviceId}:`))
        .map((tab) => tabBufferKey(state.daemon?.host_id, state.rest, tab))
        .filter((key): key is BufferKey => key !== null);
      await Promise.all(keys.map((key) => flushBuffer(key)));
      const flushed = useShellStore.getState();
      const unstored = unstoredDeviceDrafts(deviceId, flushed.editor?.tabs ?? [], flushed.bufferWarnings, draftExported(flushed.exportedDrafts, latestDraft));
      if (unstored.length > 0) {
        diagnostic(`remove_device: ${deviceId} kept because ${unstored.length} draft(s) could not be stored`);
        return unstored;
      }
      dispatch({ schema_version: 2, kind: "remove_device", payload: { device_id: deviceId } });
      return [];
    },

    /**
     * A rail tile (PRD home-device-rail D-09): that device in front. A device
     * Overview on screen follows the device, so the sidebar and the center
     * keep naming the same one.
     */
    focusDevice(deviceId: string) {
      const screen = ui().screen;
      if (screen?.kind === "main" && screen.deviceId !== deviceId) {
        // The rail's choice replaces a device the Overview's own switch picked earlier (D-02, D-17).
        ui().setOverview({ deviceId: null, projectId: null, panel: null });
        ui().setScreen({ kind: "main", deviceId });
      }
      if (frontDeviceId(rest()) === deviceId) return;
      dispatch({ schema_version: 2, kind: "focus_device", payload: { device_id: deviceId } });
    },

    /** Every entry point of Add device (the rail's `+`, the hidden rail's menu, Add project's Host list) opens the one dialog (D-11). */
    openAddDevice() {
      ui().openAddDevice();
    },

    /**
     * A Workspace chosen on All projects or an Overview, on any device (S6 B2, B21).
     * A Workspace on another device than the one in front is one event that
     * also brings its device forward, so a refusal moves neither. The screen
     * follows once the core has moved there (`opening`).
     */
    openWorkspace(deviceId: string, workspaceId: string, checkoutId: string, expanded?: FocusCheckoutPayload["expanded"], projectExpanded?: boolean) {
      const path =
        (deviceId === localDeviceId(rest()) ? rest()?.navigator?.workspaces : rest()?.status?.remote?.find((row) => row.target_id === deviceId)?.session?.workspaces)
          ?.flatMap((row) => row.checkouts)
          .find((row) => row.id === checkoutId)?.path ?? null;
      beginOpening({ checkoutId, deviceId, path, workspaceId, expanded });
      const front = frontDeviceId(rest());
      if (front === deviceId) return focusCheckout(workspaceId, checkoutId, expanded, projectExpanded);
      if (deviceId === localDeviceId(rest())) {
        dispatch({ schema_version: 2, kind: "focus_checkout", payload: { workspace_id: workspaceId, checkout_id: checkoutId, focus_device: true, ...(expanded === undefined ? {} : { expanded }), ...(projectExpanded === undefined ? {} : { project_expanded: projectExpanded }) } });
        return;
      }
      dispatch(withDeviceForward(remoteControl(deviceId, { action: "focus_workspace", workspace_id: workspaceId, checkout_id: checkoutId })));
    },

    followRelation,

    /** Asks for the failed relationship focus again, as a new request. */
    retryRelation() {
      const relation = ui().relation;
      if (relation) followRelation(relation.sourcePaneId, relation.targetPaneId, relation.label);
    },

    dismissRelation() {
      ui().setRelation(null);
    },

    dismissOpening() {
      ui().setOpening(null);
    },

    /**
     * Asks the core to read a local Git project's pull requests and issues,
     * the tasks the sidebar's 태스크 list and the Overview's backlog draw; the core reads a project
     * once and keeps the answer, so asking again costs nothing (D-14).
     */
    readProjectTasks(workspaceId: string) {
      dispatch({ schema_version: 2, kind: "github_request", payload: { workspace_id: workspaceId, refresh: false } });
    },

    /**
     * Says the operator is looking at this local Git project; the core reads it
     * again when its last good answer is old, and shows no spinner for it.
     */
    lookAtProject(workspaceId: string) {
      dispatch({ schema_version: 2, kind: "github_request", payload: { workspace_id: workspaceId, refresh: false, look: true } });
    },

    /** Reads each of these local Git projects' GitHub answer again (General's GitHub row, Check again). */
    refreshGithub(workspaceIds: readonly string[]) {
      for (const workspaceId of workspaceIds) dispatch({ schema_version: 2, kind: "github_request", payload: { workspace_id: workspaceId, refresh: true } });
    },

    /** Re-read a local Git project's checkout facts and pull requests when its Overview opens. */
    refreshProjectOverview(workspaceId: string) {
      dispatch({ schema_version: 2, kind: "overview_refresh", payload: { workspace_id: workspaceId } });
    },

    /**
     * A new tab in a checkout, or in a device's Home (`home`), with the
     * provider started in it (D-05); `terminal` is the tab alone. `prompt` is
     * the agent's first prompt, sent once it is ready, `model` a catalog id
     * (absent is the CLI's default), and `requestId` how the surface finds its
     * own answer among the core's receipts. A device's start names `deviceId`;
     * this machine's leaves it out.
     */
    startAgent(request: {
      target: { checkoutPath: string } | { home: true };
      deviceId?: string;
      provider: AgentKind;
      model?: string | null;
      prompt?: string | null;
      requestId?: string;
      tabTarget?: import("./agentLayout").AgentStartTarget;
      confirmOnly?: boolean;
      /** The session to resume instead of a new conversation (PRD link-graph B12); never with a prompt. */
      resumeSessionId?: string;
      resumeSessionPath?: string;
    }) {
      if (!request.tabTarget && request.requestId && !request.resumeSessionId) beginOpening({ startRequestId: request.requestId });
      dispatch({
        schema_version: 2,
        kind: "agent_start_in_checkout",
        payload: {
          ...("checkoutPath" in request.target ? { checkout_path: request.target.checkoutPath } : { home: true }),
          ...(request.deviceId ? deviceField(request.deviceId, localDeviceId(rest())) : {}),
          provider: request.provider,
          ...(request.model ? { model: request.model } : {}),
          ...(request.prompt ? { prompt: request.prompt } : {}),
          ...(request.requestId ? { request_id: request.requestId } : {}),
          ...(request.tabTarget ? { target: request.tabTarget } : {}),
          ...(request.confirmOnly ? { confirm_only: true } : {}),
          ...(request.resumeSessionId ? { resume_session_id: request.resumeSessionId } : {}),
          ...(request.resumeSessionPath ? { resume_session_path: request.resumeSessionPath } : {}),
        },
      });
      // Keys typed before the new tab holds the keyboard reach its agent once
      // it has started, never the pane that had the keyboard.
      if (!request.confirmOnly && request.requestId && (!request.deviceId || request.deviceId === localDeviceId(rest()))) armKeyTarget(request.requestId);
    },

    /**
     * Opens the start panel (⌘N, or ⌘K's `에이전트 시작…`); the keyboard goes
     * to its text box. The draft it kept comes back.
     */
    /**
     * Opens the start panel. Over Settings the panel takes Settings' place,
     * because Settings holds the keyboard, and Settings is what was in front (B25).
     */
    openStartPanel(projectId?: string) {
      const overSettings = ui().overlay === "settings";
      if (overSettings) ui().closeOverlay("settings");
      useStartPanel.getState().open(overSettings, projectId ? { projectId } : null);
    },

    /**
     * Watches the model catalog for a picker on screen; the returned release
     * is called when it goes away. The first watcher asks hided to read the
     * catalog and the last one stops it, so two pickers never switch each other off.
     */
    observeCatalog(): () => void {
      return catalogObserver.acquire();
    },

    /** An issue's body, labels, author, assignees and comments for its panel and the Start dialog, answered in `issue_work.detail`. */
    requestIssueDetail(workspaceId: string, taskKey: string, commentsAfter?: string) {
      dispatch({ schema_version: 2, kind: "issue_detail_request", payload: { workspace_id: workspaceId, task_key: taskKey, ...(commentsAfter ? { comments_after: commentsAfter } : {}) } });
    },

    /** Asks the background AI to name a worktree; `prefix` stays at the front. Answered in `issue_work.name`. */
    suggestWorktreeName(requestId: string, prefix: string, title: string, body: string) {
      dispatch({ schema_version: 2, kind: "worktree_name_suggest", payload: { request_id: requestId, prefix, title, body } });
    },

    /** A project's issue source; `auto` returns it to the default. */
    setIssueSource(projectPath: string, source: "auto" | "github" | "local") {
      dispatch({ schema_version: 2, kind: "issue_source_set", payload: { project_path: projectPath, source } });
    },

    setIssueSettings(patch: Partial<IssueSettings>) {
      dispatch({ schema_version: 2, kind: "issue_settings_set", payload: patch });
    },

    /** A Local issue's title and body, edited in its panel; answered in `issue_work.update` by `requestId`. The project is named since a key names only a path. */
    updateLocalIssue(requestId: string, workspaceId: string, taskKey: string, title: string, body: string) {
      dispatch({ schema_version: 2, kind: "local_issue_update", payload: { request_id: requestId, workspace_id: workspaceId, task_key: taskKey, title, body } });
    },

    /**
     * Links a pull request to an issue of its project, one the source has or
     * a new one (PRD overview-lenses-prs D-13, D-31, D-34); a retry sends the
     * same issue again. Answered in `pr_work.link` by `requestId`.
     */
    linkPullRequestIssue(requestId: string, workspaceId: string, prNumber: number, issue: { key: string } | { title: string; body: string }) {
      const target = "key" in issue ? { issue_key: issue.key } : { new_issue: { title: issue.title, body: issue.body } };
      dispatch({ schema_version: 2, kind: "pr_link_issue", payload: { request_id: requestId, workspace_id: workspaceId, pr_number: prNumber, ...target } });
    },

    /** A pull request's failed checks and change requests, for an agent's first prompt (D-46); answered in `pr_work.feedback`. */
    readPullRequestFeedback(requestId: string, workspaceId: string, prNumber: number) {
      dispatch({ schema_version: 2, kind: "pr_feedback_read", payload: { request_id: requestId, workspace_id: workspaceId, pr_number: prNumber } });
    },

    /** An agent started on a pull request's branch with `prompt` (D-12); it reports through `task_operation`. */
    delegatePullRequest(workspaceId: string, prNumber: number, provider: ProviderKind, model: string | null, prompt: string) {
      dispatch({ schema_version: 2, kind: "pr_delegate", payload: { workspace_id: workspaceId, pr_number: prNumber, provider, ...(model ? { model } : {}), prompt } });
    },

    /**
     * The link record of one pull request or issue for the panel showing it
     * (PRD link-graph B24, B25): read off the core's lock and answered in
     * `link_panel`; Retry sends the same event.
     */
    openLinks(workspaceId: string, target: LinkTarget) {
      dispatch({ schema_version: 2, kind: "links_open", payload: { workspace_id: workspaceId, target } });
    },

    /** The panel went away; an answer still on its way is dropped. */
    closeLinks() {
      dispatch({ schema_version: 2, kind: "links_close", payload: {} });
    },

    /** A session link retains repository identity even when its maker moved projects. */
    openSessionPullRequest(pull: {workspace_id: string | null; url: string; number: number}) {
      const projects = catalogWorkspaces(rest());
      const project = projects.find((row) => row.id === pull.workspace_id && row.pull_requests?.some((pr) => pr.number === pull.number && sameGithubPullRequest(pr.url, pull.url)))
        ?? projects.find((row) => row.pull_requests?.some((pr) => pr.number === pull.number && sameGithubPullRequest(pr.url, pull.url)));
      if (project) this.openPrCard(project.id, pull.number);
      else this.openLink(pull.url, false);
    },

    /**
     * A pull request's decision card (PRD sidebar-task-lens D-43, D-50,
     * B14, B17): one event, and the core puts the card in the lane's 보기
     * slot; the Workspace in front, its Agent tab and the keyboard stay. At
     * Home the PR's Workspace opens too, with the card in its lane. A project
     * this core does not read opens the page on GitHub instead. A modal
     * Overview closes first so the card shows.
     */
    openPrCard(projectId: string, number: number, checkoutId?: string) {
      const project = catalogWorkspaces(rest()).find((row) => row.id === projectId);
      if (!project) return diagnostic(`pr_open: no project ${projectId}`);
      const pull = project.pull_requests?.find((row) => row.number === number);
      if (!readsHere(project, rest()?.navigator?.devices)) {
        if (pull) this.openPullRequest(pull.url, project.device_id, true);
        return;
      }
      const state = ui();
      const openWorkspace = !workspaceInFront();
      if (state.overviewOpen || state.screen?.kind !== "workspace") state.setScreen({ kind: "workspace" });
      state.showLane();
      dispatch({ schema_version: 2, kind: "pr_open", payload: { workspace_id: projectId, pr_number: number, ...(checkoutId ? { checkout_id: checkoutId } : {}), ...(openWorkspace ? { open_workspace: true } : {}) } });
    },

    /** The PRs tool, for a list that is not tied to one pull request (`이슈 없는 PR N`). */
    openWorkPrs() {
      const state = ui();
      if (state.overviewOpen || state.screen?.kind !== "workspace") state.setScreen({ kind: "workspace" });
      showTool("prs");
    },

    /** The PRs tool's scope chip: this checkout only, or all checkouts (PRD sidebar-task-lens D-31). */
    setPrsScope(onlyCheckout: boolean) {
      return setWorkspaceView({ prs_only_checkout: onlyCheckout });
    },

    /** ← in the 보기 head (B47): the item opened before, or the slot closes. */
    detailBack() {
      dispatch({ schema_version: 2, kind: "detail_back", payload: {} });
    },

    /** ✕ in the 보기 head (B47): the slot empties and the lane goes back to its tool, or closes. */
    detailClose() {
      dispatch({ schema_version: 2, kind: "detail_close", payload: {} });
    },

    /** The rail's 보기 slot (B46): the lane draws the slot again, or, pressed while it does, closes. */
    toggleDetail() {
      if (detailShown() && laneUpWith("detail")) {
        if (!workspaceViewOf(rest())?.tool) ui().putLaneAway();
        return dispatch({ schema_version: 2, kind: "detail_show", payload: { shown: false } });
      }
      ui().showLane();
      dispatch({ schema_version: 2, kind: "detail_show", payload: { shown: true } });
    },

    /** The 프로젝트 | 태스크 switch (B1), or ⇧⌘B with `show` set while the sidebar is hidden (B2). */
    setSidebarLens(lens: import("./snapshot").SidebarLens, show = false) {
      dispatch({ schema_version: 2, kind: "sidebar_lens_set", payload: { lens, ...(show ? { show } : {}) } });
    },

    /** ⇧⌘B (B2): the other list, and the sidebar shown when it was hidden. */
    toggleSidebarLens() {
      const state = rest()?.ui_state;
      if (!state) return;
      this.setSidebarLens(state.sidebar_lens === "tasks" ? "projects" : "tasks", !state.left_sidebar_visible);
    },

    /** 상태별 | 프로젝트별 (B4, B6). */
    setTaskView(view: import("./snapshot").TaskView) {
      dispatch({ schema_version: 2, kind: "task_view_set", payload: { view } });
    },

    /**
     * A task's row asked for from outside the list (B40): a pane head's agent
     * tree button or a started backlog item. The sidebar shows the 태스크 list
     * with the row unfolded and selected, and the agent's record opens in the
     * 보기 slot, or in the Overview's panel at Home.
     */
    revealTask(paneId: string) {
      const agent = allPaneAgents(useShellStore.getState().agents, rest()?.status?.remote).find((row) => row.pane_id === paneId);
      const session = agent ? sessionOfAgent(agent, rest()) : null;
      const inFront = workspaceInFront() && !ui().overviewOpen;
      if (inFront && session) ui().showLane();
      dispatch({ schema_version: 2, kind: "task_reveal", payload: { pane_id: paneId, ...(inFront && session ? { session } : {}) } });
      if (!inFront && session) this.openOverview(session.device_id, { panel: { kind: "record", session } });
    },

    /** Reads a card's body, files and review threads again, or the part that failed. */
    readPrCard(workspaceId: string, number: number) {
      dispatch({ schema_version: 2, kind: "pr_card_read", payload: { workspace_id: workspaceId, pr_number: number } });
    },

    /** The operator's Look again after a refused write: dismisses its notice and reads the list and the card. */
    lookAgainPrCard(workspaceId: string, number: number) {
      dispatch({ schema_version: 2, kind: "pr_card_read", payload: { workspace_id: workspaceId, pr_number: number, look_again: true } });
    },

    /** Merge (or add to the merge queue) from a card, conditioned on the head commit the card showed (D-21, D-22). */
    mergePullRequest(workspaceId: string, number: number, head: string, method: PrMergeMethod | null) {
      dispatch({ schema_version: 2, kind: "pr_merge", payload: { workspace_id: workspaceId, pr_number: number, head, ...(method ? { method } : {}) } });
    },

    /** Brings a behind pull request's branch up to its base, conditioned on the head commit the card showed. */
    updatePullRequestBranch(workspaceId: string, number: number, head: string) {
      dispatch({ schema_version: 2, kind: "pr_update_branch", payload: { workspace_id: workspaceId, pr_number: number, head } });
    },

    /**
     * The Overview of one device at a place (PRD overview-device-tasks D-02,
     * D-30, D-31): as a layer over work, or as the page when no Workspace is
     * in front. Only the Overview moves to the device; the rail, the sidebar
     * and the Workspace in front stay (B2).
     */
    openOverview(deviceId: string, place: Partial<OverviewPlace> & { view?: MainView } = {}) {
      const state = ui();
      const { view, ...patch } = place;
      const modal = state.screen?.kind === "workspace" && Boolean(frontCheckout(rest()) && rest()?.workspace_view);
      const target = state.overlay === "search" ? state.searchReturnFocus : document.activeElement instanceof HTMLElement ? document.activeElement : null;
      if (view) state.setMainView(view);
      state.setOverview({ deviceId, projectId: null, panel: null, ...patch });
      useUiStore.setState({ opening: null, overviewOpen: modal || state.overviewOpen, overviewReturnFocus: state.overviewOpen ? state.overviewReturnFocus : target, overlay: "none", ...(!modal && !state.overviewOpen ? { screen: { kind: "main" as const, deviceId } } : {}) });
    },

    /** A project's panel on the Overview's Projects (PRD sidebar-task-lens B41): a breadcrumb's project, a cross-project chip, a Recent Panels project. */
    openProjectPanel(deviceId: string, projectId: string) {
      this.openOverview(deviceId, { view: "projects", projectId });
    },

    /**
     * An issue or memo from ⌘K, a Factory card, a recorded issue or a missed
     * line's `· 백로그 #N` (B44, B60): the task card it is on when it was
     * started, else the item in the backlog.
     */
    openBacklogItem(deviceId: string, projectId: string, taskKey: string) {
      const found = cardOfItem(boardProjects(rest(), useShellStore.getState().agents, deviceId), taskKey);
      if (found) return this.revealTask(found.card.root);
      this.openOverview(deviceId, { view: "backlog", projectId, panel: { kind: "item", workspaceId: projectId, taskKey } });
    },

    /**
     * `View conversation` on a linked session (B58): its record in the 보기
     * slot, or in the Overview's panel when no Workspace is in front.
     */
    openSessionConversation(session: import("./snapshot").WorkSession) {
      if (frontCheckout(rest()) && rest()?.workspace_view) this.closeOverview();
      this.openSessionRecord(session);
    },

    /** ⌘K's `GitHub에서 "…" 검색` row: one search of this Mac's GitHub projects, answered in `issue_work.search` by `requestId`. */
    searchGithub(requestId: string, query: string) {
      dispatch({ schema_version: 2, kind: "github_search", payload: { request_id: requestId, query } });
    },

    /** Closes or reopens a Local issue of a project; a GitHub one closes on GitHub. */
    setIssueOpen(workspaceId: string, taskKey: string, open: boolean) {
      dispatch({ schema_version: 2, kind: "issue_set_open", payload: { workspace_id: workspaceId, task_key: taskKey, open } });
    },


    /** Opening the disk cleanup sheet, or its `다시 검토`: the core reviews the project and measures it again. */
    reviewDiskCleanup(workspaceId: string) {
      dispatch({ schema_version: 2, kind: "cleanup_review", payload: { workspace_id: workspaceId } });
    },

    /** The one event a `정리` press is: the worktrees to remove and the cache cells to empty (D-15, B27). */
    confirmDiskCleanup(id: number, paths: string[], cells: { path: string; layer: string }[]) {
      dispatch({ schema_version: 2, kind: "cleanup_confirm", payload: { id, paths, cells } });
    },

    /** Closing the sheet; the core keeps a cleanup that is still removing. */
    dismissDiskCleanup() {
      dispatch({ schema_version: 2, kind: "cleanup_dismiss", payload: {} });
    },

    /**
     * Opens a catalog Project's Memory for the Memory tab (PRD
     * project-memory-v2 D-17); the core keeps one opened Project per daemon
     * and reads it again when asked for the same one.
     */
    memoryOpen(workspaceId: string) {
      dispatch({ schema_version: 2, kind: "memory_open", payload: { workspace_id: workspaceId } });
    },

    setProjectSettings(target: import("./projectSettings").ProjectSettingsTarget, patch: import("./projectSettings").ProjectSettingsPatch, requestId: string) {
      return dispatch({ schema_version: 2, kind: "project_settings_set", payload: { device_id: target.deviceId, root: target.root, request_id: requestId, patch } });
    },

    openProjectMemory(deviceId: string, workspaceId: string) {
      const workspace = catalogWorkspaces(rest()).find((row) => row.id === workspaceId && row.device_id === deviceId);
      const checkout = workspace && deviceConnected(rest(), deviceId) ? projectCheckout(workspace, rest()?.ui_state?.recent_checkouts) : null;
      if (!checkout) return false;
      beginOpening({ checkoutId: checkout.id, deviceId, path: checkout.path, workspaceId, tool: "memory" });
      return dispatch({ schema_version: 2, kind: "project_memory_navigate", payload: { device_id: deviceId, workspace_id: workspaceId, checkout_id: checkout.id, request_id: crypto.randomUUID() } });
    },

    /** Shows one card of the opened Project in full; null folds it. */
    memoryExpand(workspaceId: string, number: number | null) {
      dispatch({ schema_version: 2, kind: "memory_expand", payload: { workspace_id: workspaceId, number } });
    },

    /** Searches the opened Project's cards; empty words clear the search. */
    memorySearch(workspaceId: string, query: string) {
      dispatch({ schema_version: 2, kind: "memory_search", payload: { workspace_id: workspaceId, query } });
    },

    /** One operator action on the opened Project's Memory or one of its cards. */
    memoryAction(workspaceId: string, action: MemoryAction, number: number | null = null, numbers: readonly number[] = []) {
      dispatch({ schema_version: 2, kind: "memory_action", payload: { workspace_id: workspaceId, action, number, ...(numbers.length ? { numbers } : {}) } });
    },

    /**
     * A card number under a past conversation (B33): that Project's Memory
     * opens with the card in full, in the Memory tab of one of its checkouts.
     */
    openMemoryCard(deviceId: string, workspaceId: string, number: number) {
      this.memoryOpen(workspaceId);
      this.memoryExpand(workspaceId, number);
      this.openProjectById(deviceId, workspaceId);
      showTool("memory");
    },

    /**
     * Deliver now on a pane whose letters wait on a possible draft: one event,
     * and the core rings them as its doorbell would (docs/delivery.md). A press
     * the core no longer stands behind is logged there, never shown.
     */
    deliverLettersNow(paneId: string) {
      dispatch({ schema_version: 2, kind: "deliver_letters_now", payload: { pane_id: paneId } });
    },

    /** An agent chosen on an Overview or in the Agents list: its Workspace and pane (B12). */
    resolveSession(paneId: string) {
      dispatch({ schema_version: 2, kind: "resolve_session", payload: { pane_id: paneId } });
    },

    /** ★ a task so it waits in 다시 볼 것 while quiet (PRD revisit-group D-03). */
    starSession(paneId: string) {
      dispatch({ schema_version: 2, kind: "revisit_star", payload: { pane_id: paneId } });
    },

    /** 다 봤음: the ★ goes and the read ask standing now is put away (D-05). */
    finishRevisit(paneId: string) {
      dispatch({ schema_version: 2, kind: "revisit_done", payload: { pane_id: paneId } });
    },

    /**
     * An agent chosen in a list that shows only what the operator has to do
     * (the sidebar, Sessions, a digit): a row its home owns opens the home,
     * the Memory tab with that attempt's row open, and any other row its pane
     * (PRD hide-ai-work-home B11). Its pane stays one link away from there.
     */
    openListedAgent(paneId: string) {
      const state = useShellStore.getState();
      const agent = agentsByPane(allPaneAgents(state.agents, state.rest?.status?.remote)).get(paneId);
      const home = homeOpening(state.rest, agent);
      if (home) return this.openMemoryCard(home.deviceId, home.workspaceId, home.number);
      this.openAgent(paneId);
    },

    openAgent(paneId: string) {
      beginOpening({ paneId });
      const node = localDeviceId(rest());
      const target = remoteTargetOfPane(rest(), paneId) ?? node;
      const forward = frontDeviceId(rest()) !== target;
      if (target !== node) {
        const event = remoteControl(target, { action: "focus_pane", pane_id: paneId });
        return dispatch(forward ? withDeviceForward(event) : event);
      }
      dispatch({ schema_version: 2, kind: "focus_pane", payload: { pane_id: paneId, origin: "operator", focus_device: forward } });
      // The core's answer moves the keys only when it moves the focus, so a
      // pane already focused would leave them on the row, link or mark that
      // was pressed. The operator asked for the pane, so the open takes the
      // keys itself while the pane is on screen; the terminal reports no
      // click for a focus it is given (`focusTerminal`).
      if (ui().screen?.kind === "workspace" && useShellStore.getState().focusedPaneId === paneId) focusTerminal(paneId);
    },

    /**
     * Pins or unpins a project. A row Herdr shows without a registration is
     * registered with its device and root by the same event (PRD
     * sidebar-context-menus D-14), so pinning it is one action.
     */
    setPinned(workspaceId: string, pinned: boolean) {
      dispatch({ schema_version: 2, kind: "workspace_pin_set", payload: { workspace_id: workspaceId, pinned } });
    },

    /**
     * A new tab in one checkout, brought to the front (PRD sidebar-context-menus
     * D-08): the core focuses the checkout and asks Herdr for the tab in the
     * same event. A device's checkout gets its tab on that device's Herdr.
     */
    newTabIn(deviceId: string, checkout: Checkout) {
      beginOpening({ checkoutId: checkout.id, deviceId, path: checkout.path, workspaceId: checkout.workspace_id });
      if (checkout.agent_groups) {
        agentGroup({ device_id: deviceId, path: checkout.path }, checkout.agent_groups.selected, { action: "new_tab", terminal: true });
        return;
      }
      if (deviceId !== localDeviceId(rest())) {
        const host = remoteHost("shell.command.newTab");
        if (!host) return ui().setOpening(null);
        if (host.targetId !== deviceId) return diagnostic(`create_tab: ${checkout.id} is not on the device in front`);
        sendRemote(deviceId, { action: "create_tab", workspace_id: checkout.workspace_id, checkout_id: checkout.id, cwd: checkout.path, label: checkout.next_tab_label });
        return;
      }
      const requestId = remoteRequestId();
      dispatch({ schema_version: 2, kind: "create_tab", payload: { workspace_id: checkout.workspace_id, checkout_id: checkout.id, label: checkout.next_tab_label, request_id: requestId } });
      armKeyTarget(requestId);
    },

    /** Makes a checkout its project's default: the home glyph and the first place move to it (D-03). */
    setPrimaryCheckout(workspaceId: string, checkoutId: string) {
      dispatch({ schema_version: 2, kind: "set_primary_checkout", payload: { workspace_id: workspaceId, checkout_id: checkoutId } });
    },

    /**
     * Closes the tab that holds a pane, wherever that tab is, through the tab
     * close confirmation (D-06): an agent row's tab need not be the one on
     * screen, nor on this machine.
     */
    closeTabOfPane(paneId: string) {
      const targetId = remoteTargetOfPane(rest(), paneId);
      const status = targetId ? rest()?.status?.remote?.find((row) => row.target_id === targetId) : null;
      const workspaces = targetId ? status?.session?.workspaces : rest()?.navigator?.workspaces;
      const tab = workspaces?.flatMap((row) => row.checkouts).flatMap((row) => row.tabs).find((row) => row.panes.some((pane) => pane.id === paneId));
      if (!tab?.id) return diagnostic(`close_tab: no tab holds ${paneId}`);
      if (targetId && status?.state !== "connected") {
        const label = machineName(rest()?.navigator?.devices, targetId, translate);
        ui().setNotice({ text: translate("shell.deviceNotConnected", { device: label, command: translate("shell.command.closeTab") }), refreshable: false });
        return;
      }
      requestClose("tab", tab.id, tab.panes, targetId);
    },

    /** Puts text on the clipboard; a refused write is the log's, not a notice (design principle 13). */
    copyText(text: string, what: string) {
      if (!navigator.clipboard) return diagnostic(`copy ${what}: the clipboard is unavailable`);
      navigator.clipboard.writeText(text).catch((error: unknown) => diagnostic(`copy ${what}: ${String(error)}`));
    },

    /**
     * The native folder picker of the desktop app, for Add a project. A cancel
     * answers null; so does a picker that failed, which goes to the log.
     */
    async pickFolder(): Promise<string | null> {
      const bridge = hostBridge();
      if (!bridge) {
        diagnostic("add project: this host has no folder picker");
        return null;
      }
      try {
        return await bridge.pickFolder();
      } catch (error) {
        diagnostic(`add project: the folder picker failed: ${String(error)}`);
        return null;
      }
    },

    /**
     * `reveal_external` (issue 324): the desktop app selects a file or folder
     * of this computer in its parent folder in the OS file manager; a browser
     * tab has no such item.
     */
    revealExternal,
    openInEditor,

    /** A new worktree; `taskKey` links it to that issue, and `prompt` is its agent's first prompt. */
    createWorktree(request: {
      requestId?: string;
      deviceId: string;
      repositoryRoot: string;
      branch: string;
      baseBranch: string | null;
      agentKind: string | null;
      model?: string | null;
      purpose: string | null;
      taskKey?: string | null;
      prompt?: string | null;
    }) {
      dispatch({
        schema_version: 2,
        kind: "create_worktree",
        payload: {
          ...(request.requestId ? { request_id: request.requestId } : {}),
          device_id: request.deviceId,
          repository_root: request.repositoryRoot,
          branch: request.branch,
          base_branch: request.baseBranch,
          agent_kind: request.agentKind,
          ...(request.model ? { model: request.model } : {}),
          purpose: request.purpose,
          ...(request.taskKey ? { task_key: request.taskKey } : {}),
          ...(request.prompt ? { prompt: request.prompt } : {}),
        },
      });
    },

    setPurpose(checkoutId: string, text: string) {
      dispatch({ schema_version: 2, kind: "set_checkout_purpose", payload: { checkout_id: checkoutId, text } });
    },

    /** `discardChanges`: the operator ticked the discard the core's gate offered, accepting the folder's uncommitted work is lost. */
    /** `closeDescendants`: the agents outside the worktree the operator chose to close first (PRD close-agent-subtree D-36). */
    removeWorktree(deviceId: string, checkoutPath: string, deleteBranch: boolean, discardChanges: boolean, closeDescendants: string[] = [], ignoredRepositories: string[] = []) {
      dispatch({
        schema_version: 2,
        kind: "remove_worktree",
        payload: { device_id: deviceId, checkout_path: checkoutPath, delete_branch: deleteBranch, discard_changes: discardChanges, expected_ignored_repositories: ignoredRepositories, ...(closeDescendants.length > 0 ? { close_descendant_pane_ids: closeDescendants } : {}) },
      });
    },

    retryTaskAgent(id: number) {
      dispatch({ schema_version: 2, kind: "task_agent_retry", payload: { id } });
    },

    focusPane,

    createTab,
    agentGroup,
    agentLayout,
    runAgentCommand,

    focusTab(tabId: string) {
      const frame = areaFrame("agent");
      if (frame?.layout.drafts?.includes(tabId)) return agentLayout({ action: "focus", tab_id: tabId });
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.switchTab");
        if (!host) return;
        const event = remoteControl(host.targetId, { action: "focus_tab", tab_id: tabId });
        dispatch(event);
        return;
      }
      const here = current();
      if (!here) return;
      dispatch({
        schema_version: 2,
        kind: "focus_tab",
        payload: { workspace_id: here.checkout.workspace_id, checkout_id: here.checkout.id, tab_id: tabId },
      });
    },

    /**
     * A Recent Panels or Recent Projects commit: the surface comes forward in
     * its own checkout, bringing this machine and that Workspace to the front
     * when they are not, as one event. A Herdr tab is a `focus_tab`; a display
     * is the checkout's `focus_checkout` naming it, and the keyboard follows
     * into the display once the core shows it.
     */
    openSurface(surface: Surface) {
      if (surface.deviceId !== localDeviceId(rest())) {
        // A device's tab comes forward on that device's Herdr, bringing the device with it: one event for rail, sidebar and center (D-16).
        const status = rest()?.status?.remote?.find((row) => row.target_id === surface.deviceId);
        const checkout = status?.session?.workspaces.flatMap((row) => row.checkouts).find((row) => row.id === surface.checkoutId);
        if (!checkout) return diagnostic(`recent: ${surface.checkoutId} is no longer open on ${surface.deviceId}`);
        expectSurface(surface.key);
        beginOpening({ checkoutId: checkout.id, deviceId: surface.deviceId, path: checkout.path, workspaceId: surface.workspaceId });
        dispatch(withDeviceForward(remoteControl(surface.deviceId, { action: "focus_tab", tab_id: surface.id })));
        return;
      }
      const checkout = rest()?.navigator?.workspaces?.flatMap((row) => row.checkouts).find((row) => row.id === surface.checkoutId);
      if (!checkout) return diagnostic(`recent: ${surface.checkoutId} is no longer open`);
      const node = localDeviceId(rest());
      const focusDevice = frontDeviceId(rest()) !== node ? { focus_device: true } : {};
      const ids = { workspace_id: surface.workspaceId, checkout_id: surface.checkoutId };
      expectSurface(surface.key);
      beginOpening({ checkoutId: checkout.id, deviceId: node, path: checkout.path, workspaceId: surface.workspaceId });
      if (surface.kind === "herdr") {
        dispatch({ schema_version: 2, kind: "focus_tab", payload: { ...ids, tab_id: surface.id, ...focusDevice } });
        return;
      }
      ui().setViewFocusRequest({ workspace: workspaceKey({ device_id: node, path: checkout.path }), displayId: surface.id, from: null });
      dispatch({ schema_version: 2, kind: "focus_checkout", payload: { ...ids, display_id: surface.id, ...focusDevice } });
    },

    renameTab(tabId: string, label: string, requestId: string) {
      dispatch({ schema_version: 2, kind: "rename_tab", payload: { tab_id: tabId, label, request_id: requestId } });
    },

    reorderTab(stripId: string, toIndex: number) {
      // A device's strip is arranged by the core as this machine's is; a
      // Herdr tab moves on the device's own Herdr (`reorder_tab`).
      // A file slot moves with the device offline too; the core refuses a
      // Herdr move while the device is not connected.
      const context = remoteContext(rest());
      if (context) {
        const checkout = remoteView(context.session, rest()?.workspace_view)?.checkout;
        if (!checkout) return diagnostic("reorder_tab: no device checkout in front");
        dispatch({
          schema_version: 2,
          kind: "reorder_tab",
          payload: { workspace_id: checkout.workspace_id, checkout_id: checkout.id, tab_id: stripId, to_index: toIndex },
        });
        return;
      }
      diagnostic("reorder_tab: local tabs are ordered by their Agent area");
    },

    closeFocused() {
      const owner = keyboardCommandOwner();
      const agents = areaFrame("agent");
      const active = agents ? activeAgentDisplay(agents.layout)?.display.id : null;
      if (owner.kind !== "view" && active && agents?.layout.drafts?.includes(active)) return closeTab(active);
      const frame = frameNow();
      const area = owner.kind === "view" && frame ? findArea(frame.layout.root, owner.areaId) : null;
      const checkout = frontCheckout(rest());
      const tab = remoteContext(rest()) ? remoteView(remoteContext(rest())?.session ?? null, rest()?.workspace_view)?.tab : current()?.tab;
      const target = closeShortcutPolicy({
        owner,
        workspace: ui().screen?.kind === "workspace" ? checkout?.id ?? null : null,
        displayId: ui().drawnLane.tool === "files" && ui().drawnLane.mode !== "closed" && ui().drawnLane.mode !== "hidden" ? area?.active ?? null : null,
        paneIds: tab?.panes.map((pane) => pane.id) ?? [],
        detailShown: ui().screen?.kind === "workspace" && !ui().overviewOpen && laneUpWith("detail"),
      });
      if (target.kind === "detail") return this.detailClose();
      if (target.kind === "view") return closeView(target.id);
      if (target.kind === "pane") return this.closePane(target.id);
      diagnostic(`close_shortcut: ${target.reason}`);
    },

    /**
     * The new tab chord (issue 239): the View strip's New tab in the View
     * area that holds the keyboard, an agent tab at the end of the Agent area
     * showing the keyboard's pane, else one in the Agent active area.
     */
    newTabFocused() {
      const drawn = drawnViews();
      const agents = areaFrame("agent");
      const tabs = current()?.checkout.tabs ?? [];
      const paneAreas: Record<string, string> = {};
      for (const area of agents ? areasOf(agents.layout.root) : []) {
        const shown = agents?.layout.canvases[area.id] ?? area.active;
        for (const pane of tabs.find((tab) => tab.id === shown)?.panes ?? []) paneAreas[pane.id] = area.id;
      }
      const target = newTabPolicy({
        owner: keyboardCommandOwner(),
        workspace: ui().screen?.kind === "workspace" ? frontCheckout(rest())?.id ?? null : null,
        viewAreaIds: drawn ? areasOf(drawn.layout.root).map((area) => area.id) : [],
        paneAreas,
      });
      if (target.kind === "view" && drawn) return this.openBrowser("", drawn.workspace, target.areaId);
      createTab(target.areaId ?? undefined);
    },

    closeTab,

    closePane(paneId?: string) {
      const id = paneId ?? useShellStore.getState().focusedPaneId;
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.closePane");
        if (!host) return;
        const pane = host.view?.tab?.panes.find((row) => row.id === id);
        if (!id || !pane) return diagnostic("close_pane: no focused remote pane");
        requestClose("pane", id, [pane], host.targetId);
        return;
      }
      const here = current();
      if (!here || !id) return diagnostic("close_pane: no focused pane");
      const pane = here.checkout.tabs.flatMap((tab) => tab.panes).find((row) => row.id === id);
      if (!pane) return diagnostic("close_pane: the pane is no longer visible");
      requestClose("pane", id, [pane], null);
    },

    /**
     * The operator chose "Stop work and close" on the confirmation, or
     * 이것만 닫기 on the subtree sheet: the target alone, as an ordinary
     * close, with no second question (B12).
     */
    confirmClose() {
      const pending = ui().pendingClose;
      if (!pending) return;
      ui().setPendingClose(null);
      // The confirmation names the host it was asked about; switching devices
      // while it was open does not move the close to another one.
      sendClose(pending.kind, pending.id, pending.targetId, true);
    },

    /** 모두 닫기: the target and exactly the descendants the sheet shows at the press, in one event (D-20, D-21). */
    closeSubtree(ids: string[]) {
      const pending = ui().pendingClose;
      if (!pending) return;
      ui().setPendingClose(null);
      dispatch({
        schema_version: 2,
        kind: "close_tree",
        payload: {
          target: pending.kind === "pane" ? { kind: "pane", pane_id: pending.id } : { kind: "tab", tab_id: pending.id },
          pane_ids: ids,
          confirmed: true,
        },
      });
    },

    /** Every current agent row the close lists read, this machine's and each connected device's. */
    everyAgent,

    closeTarget,

    keepOpen() {
      ui().setPendingClose(null);
    },

    refreshStatus() {
      ui().setNotice(null);
      dispatch({ schema_version: 2, kind: "refresh_status", payload: {} });
    },

    /** A close whose outcome the core could not read; asks it to check (`check_close_status`). */
    retryAgentClose(key: string) {
      dispatch({ schema_version: 2, kind: "retry_agent_close", payload: { key } });
    },
    dismissAgentClose(key: string) {
      dispatch({ schema_version: 2, kind: "dismiss_agent_close", payload: { key } });
    },
    checkCloseStatus(key: string) {
      dispatch({ schema_version: 2, kind: "check_close_status", payload: { key } });
    },

    reopenClosed() {
      // The core reopens the newest close of the device in front, and
      // `recent_closed` already speaks for that device.
      const recent = rest()?.recent_closed;
      if (!recent?.can_reopen) {
        diagnostic(`reopen_closed: nothing to reopen${recent?.reopen_blocked_reason ? ` (${recent.reopen_blocked_reason})` : ""}`);
        return;
      }
      dispatch({ schema_version: 2, kind: "reopen_closed", payload: {} });
    },

    /**
     * Splits `target`, a pane the operator pointed at, else the focused pane.
     * The core splits its current pane, which the pointing already focused.
     */
    split(direction: "right" | "down", target?: string) {
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.split");
        if (!host) return;
        const pane = host.view?.tab?.panes.find((row) => row.id === (target ?? host.view?.focusedPaneId));
        if (!pane) return diagnostic("split_pane: no focused remote pane");
        sendRemote(host.targetId, { action: "split_pane", pane_id: pane.id, direction, cwd: pane.cwd || null });
        return;
      }
      const here = current();
      const paneId = target ?? useShellStore.getState().focusedPaneId;
      const pane = here?.tab?.panes.find((row) => row.id === paneId);
      if (!here?.tab?.id || !pane) return diagnostic("create_pane: no focused pane");
      const requestId = remoteRequestId();
      dispatch({
        schema_version: 2,
        kind: "create_pane",
        payload: { tab_id: here.tab.id, cwd: pane.cwd, command: null, direction, request_id: requestId },
      });
      armKeyTarget(requestId);
    },

    /** Zooms or unzooms `target`, a pane the operator pointed at, else the focused pane. */
    toggleZoom(target?: string) {
      if (remoteContext(rest())) {
        const host = remoteHost("shell.command.zoomPane");
        if (!host) return;
        const paneId = target ?? host.view?.focusedPaneId;
        if (!paneId) return diagnostic("toggle_pane_zoom: no focused remote pane");
        sendRemote(host.targetId, { action: "toggle_pane_zoom", pane_id: paneId });
        return;
      }
      const paneId = target ?? useShellStore.getState().focusedPaneId;
      if (!paneId) return diagnostic("toggle_zoom: no focused pane");
      dispatch({ schema_version: 2, kind: "toggle_zoom", payload: { pane_id: paneId } });
    },

    /** ⌘= / ⌘- / ⌘0 scale whichever surface is showing: an image or PDF
     * zooms when its display holds the keyboard, the document's text when an
     * editor tab owns the canvas, else the focused terminal pane's. */
    textScale(direction: "in" | "out" | "reset") {
      // A viewer's zoom is its display's own, page-local and never sent.
      const owner = keyboardCommandOwner();
      const frame = frameNow();
      const area = owner.kind === "view" && frame ? findArea(frame.layout.root, owner.areaId) : null;
      if (area?.active && zoomViewer(area.active, direction)) return;
      // A pane's text size is this page's drawing, stored in the core's ui
      // state by pane id; a remote pane is sized the same way and nothing is
      // sent to its host.
      if (editorFor(useShellStore.getState().editor) && owner.kind === "view") {
        dispatch({ schema_version: 2, kind: "editor_text_scale", payload: { direction } });
        return;
      }
      const paneId = useShellStore.getState().focusedPaneId;
      if (!paneId) return diagnostic("pane_text_scale: no focused pane");
      dispatch({ schema_version: 2, kind: "pane_text_scale", payload: { pane_id: paneId, direction } });
    },

    focusCheckout,

    toggleSessionFold(key: string) {
      dispatch({ schema_version: 2, kind: "session_fold_toggle", payload: { key } });
    },

    toggleInactiveCheckouts(projectPath: string) {
      dispatch({ schema_version: 2, kind: "inactive_checkouts_toggle", payload: { project_path: projectPath } });
    },

    /** Reports the keyboard's visit to a pane; the core keeps the Agent cycle's order (`ui_state.recent_pane_ids`). */
    paneVisit(paneId: string) {
      dispatch({ schema_version: 2, kind: "pane_visit", payload: { pane_id: paneId } });
    },

    /** Folds or unfolds an agent's children in the sidebar tree; the core keeps the choice. */
    toggleAgentTree(paneId: string) {
      dispatch({ schema_version: 2, kind: "agent_tree_toggle", payload: { pane_id: paneId } });
    },

    /** Folds or unfolds an agent's children in the 태스크 list, kept apart from the 프로젝트 tree's (PRD sidebar-task-lens D-48). */
    toggleSessionTree(paneId: string) {
      dispatch({ schema_version: 2, kind: "session_tree_toggle", payload: { pane_id: paneId } });
    },

    toggleInactiveProjects(deviceId: string) {
      dispatch({ schema_version: 2, kind: "inactive_projects_toggle", payload: { device_id: deviceId } });
    },

    /** Folds or unfolds a project's checkouts in the Projects list; the core keeps the choice and says it back as `expanded`. */
    toggleProjectCheckouts(workspace: Workspace) {
      foldProject(workspace);
    },

    /** Open the project's usable checkout and admit its disclosure in the same event. */
    openProject(workspace: Workspace, expanded: boolean | undefined) {
      const checkout = deviceConnected(rest(), workspace.device_id) ? projectCheckout(workspace, rest()?.ui_state?.recent_checkouts) : null;
      if (!checkout) return this.openProjectPanel(workspace.device_id, workspace.id);
      this.openWorkspace(workspace.device_id, workspace.id, checkout.id, undefined, expanded);
    },

    openProjectById(deviceId: string, projectId: string) {
      const workspace = catalogWorkspaces(rest()).find((row) => row.id === projectId && row.device_id === deviceId);
      if (workspace) return this.openProject(workspace, undefined);
      this.openProjectPanel(deviceId, projectId);
    },

    /** Opens or closes the agent rows under a checkout in the Projects list; they start closed and the core keeps the choice. */
    toggleCheckoutAgents(checkoutId: string) {
      dispatch({ schema_version: 2, kind: "checkout_agents_toggle", payload: { checkout_id: checkoutId } });
    },

    toggleLeftSidebar() {
      const state = rest()?.ui_state;
      if (state) setLeftSidebarVisible(!state.left_sidebar_visible);
    },

    /** Shows or hides the device rail; the core keeps the choice across a restart. */
    toggleDeviceRail() {
      setDeviceRailVisible(!railShown(rest()));
    },

    hideDeviceRail() {
      setDeviceRailVisible(false);
    },

    showDeviceRail() {
      setDeviceRailVisible(true);
    },

    focusSidebar() {
      useUiStore.setState({ overviewOpen: false, overviewReturnFocus: null, sidebarFocus: ui().sidebarFocus + 1 });
      if (!rest()?.ui_state?.left_sidebar_visible) setLeftSidebarVisible(true);
    },

    toggleOverview() {
      const state = ui();
      if (state.overviewOpen) return state.setOverviewOpen(false);
      if (state.overlay !== "none" && (state.overlay !== "search" || state.searchOver !== "none")) return;
      if (state.workspaceDialog || state.pendingClose || (state.overlay !== "search" && state.escapeLayers.length > 0)) return;
      if (state.overlay === "search") state.closeOverlay();
      if (state.screen?.kind === "main") {
        if (frontCheckout(rest()) && rest()?.workspace_view) state.setScreen({ kind: "workspace" });
        return;
      }
      // Every open starts on the rail's front device with no project chosen (B1), as a page or as a layer.
      state.setOverview({ deviceId: null, projectId: null, panel: null });
      if (state.screen?.kind !== "workspace" || !frontCheckout(rest()) || !rest()?.workspace_view) {
        state.setScreen({ kind: "main" });
        return;
      }
      const target = state.overlay === "search" ? state.searchReturnFocus : document.activeElement instanceof HTMLElement ? document.activeElement : null;
      useUiStore.setState({ overviewOpen: true, overviewReturnFocus: target, overlay: "none" });
    },

    /**
     * ⌘⇧M (PRD overview-device-tasks B37): the quick add over any screen, a
     * terminal holding the keyboard included; ⌘K and the find bar close
     * first, and the keyboard goes back to where it was before them. A second
     * press closes it.
     */
    openQuickAdd() {
      const state = ui();
      if (state.quickAdd) return useUiStore.setState({ quickAdd: null });
      const returnFocus = state.overlay === "search" ? state.searchReturnFocus : document.activeElement instanceof HTMLElement ? document.activeElement : null;
      if (state.overlay === "search" || state.overlay === "find") state.closeOverlay();
      useUiStore.setState({ quickAdd: { returnFocus } });
    },

    closeQuickAdd() {
      useUiStore.setState({ quickAdd: null });
    },

    openOverviewEntry() {
      if (ui().screen?.kind === "main") return;
      this.toggleOverview();
    },

    openUsageHistory() {
      // The Usage tab is the device Overview's; from a Workspace the Overview opens over it first.
      if (ui().screen?.kind !== "main" && !ui().overviewOpen) this.toggleOverview();
      ui().setMainView("usage");
    },

    closeOverview() {
      if (ui().overviewOpen) return ui().setOverviewOpen(false);
      if (frontCheckout(rest()) && rest()?.workspace_view) ui().setScreen({ kind: "workspace" });
    },

    /**
     * A session's record (PRD sidebar-task-lens D-44, B15, B17): in the
     * lane's 보기 slot, where the core starts its read, or in the Overview's
     * panel when no Workspace is in front. The keyboard stays where it is.
     */
    openSessionRecord(session: import("./snapshot").WorkSession) {
      if (workspaceInFront() && !ui().overviewOpen) return openDetail({ kind: "record", session });
      this.openOverview(session.device_id, { panel: { kind: "record", session } });
    },
    /** A child's record from the one shown (B16): it takes the slot, and ← comes back to the parent. */
    openChildRecord(session: import("./snapshot").WorkSession) {
      dispatch({ schema_version: 2, kind: "detail_open", payload: { item: { kind: "record", session } } });
    },
    /**
     * ⇧⌘U: pins the focused pane's task checklist open, or closes it when it
     * is pinned there. A pane without tasks draws no chip and pins nothing, so
     * a list never opens by itself once that pane later has tasks.
     */
    toggleTaskList(target?: string) {
      const state = useShellStore.getState();
      const paneId = target ?? state.focusedPaneId;
      if (!paneId) return;
      if (ui().taskListPane === paneId) return ui().setTaskListPane(null);
      if (paneProgress(state, paneId)) ui().setTaskListPane(paneId);
    },
    /**
     * A pane head's title (B15): the slot shows that session's record, or,
     * when the lane already draws it, the lane goes back to its tool. The
     * keyboard stays in the pane's terminal.
     */
    togglePaneRecord(session: import("./snapshot").WorkSession) {
      const top = rest()?.ui_state?.detail?.items.at(-1);
      if (top?.kind === "record" && workKey(top.session) === workKey(session) && laneUpWith("detail")) return this.toggleDetail();
      this.openSessionRecord(session);
    },
    searchSessionWork(query: string, options?: import("./snapshot").SessionSearchOptions) {
      dispatch({ schema_version: 2, kind: "session_work_search", payload: { query, ...(options ? { options } : {}) } });
    },
    setMissedStatus(session: import("./snapshot").WorkSession, id: string, status: "open" | "dismissed") {
      dispatch({ schema_version: 2, kind: "session_missed_status", payload: { session, id, status } });
    },
    /**
     * A title for a project's backlog (PRD overview-device-tasks D-10, D-13,
     * D-34): a memo at once, or one title-only GitHub issue; a missed line's
     * `백로그로` names the line. The returned intent names the item in
     * `issue_work.backlog` and is what a retry sends again.
     */
    addBacklog(workspaceId: string, title: string, missed?: { session: import("./snapshot").WorkSession; id: string }, destination?: { kind: "local" } | { kind: "github"; repository: string }): string {
      const intent = backlogIntent();
      dispatch({ schema_version: 2, kind: "backlog_add", payload: { intent, workspace_id: workspaceId, title, destination: destination?.kind ?? "local", ...(destination?.kind === "github" ? { repository: destination.repository } : {}), ...(missed ? { missed } : {}) } });
      return intent;
    },
    /** `다시 시도` on a failed item: the same intent, so a create GitHub already made is joined, not repeated (B30, B45). */
    retryBacklog(intent: string) {
      dispatch({ schema_version: 2, kind: "backlog_retry", payload: { intent } });
    },
    /** `지우기` on a failed item; for a missed line's item the core also puts the line away (one event). */
    dismissBacklog(intent: string) {
      dispatch({ schema_version: 2, kind: "backlog_dismiss", payload: { intent } });
    },
    /** `되돌리기` (B32): a memo is deleted, an issue closed, and a missed line it came from opens again. */
    undoBacklog(workspaceId: string, taskKey: string) {
      dispatch({ schema_version: 2, kind: "backlog_undo", payload: { workspace_id: workspaceId, task_key: taskKey } });
    },
    /** Where an item came from, for its panel (B31). */
    readBacklogOrigin(workspaceId: string, deviceId: string, taskKey: string) {
      dispatch({ schema_version: 2, kind: "backlog_origin", payload: { workspace_id: workspaceId, device_id: deviceId, task_key: taskKey } });
    },
    /** No screen shows that item's origin any more: the core drops the session text it held for it. */
    closeBacklogOrigin(workspaceId: string, deviceId: string, taskKey: string) {
      dispatch({ schema_version: 2, kind: "backlog_origin_close", payload: { workspace_id: workspaceId, device_id: deviceId, task_key: taskKey } });
    },
    /** The projects the Projects tab shows, in its order, for the disk sweep; none when it is hidden (D-15, B53). */
    watchDisk(workspaceIds: readonly string[]) {
      dispatch({ schema_version: 2, kind: "disk_watch", payload: { workspace_ids: [...workspaceIds] } });
    },
    /** The Overview panel's record: read, without the lane moving. */
    readSessionRecord(session: import("./snapshot").WorkSession) {
      dispatch({ schema_version: 2, kind: "session_record_open", payload: { session } });
    },
    /** The Overview panel's record closed: the 보기 slot's own record, when it holds one, is read again. */
    leaveOverviewRecord() {
      // Another record taking the panel reads its own session; only a record that left the screen hands the read back.
      const state = ui();
      const shown = state.overviewOpen || state.screen?.kind === "main";
      if (shown && state.overview.panel?.kind === "record") return;
      const top = rest()?.ui_state?.detail?.items.at(-1);
      const lane = top?.kind === "record" ? top.session : null;
      if (lane) dispatch({ schema_version: 2, kind: "session_record_open", payload: { session: lane } });
      else dispatch({ schema_version: 2, kind: "session_record_close", payload: {} });
    },
    /** `lane에서 보기` (B18): the Overview closes, the checkout opens and the 보기 slot shows the same record. */
    viewRecordInLane(deviceId: string, workspaceId: string, checkoutId: string, session: import("./snapshot").WorkSession) {
      this.closeOverview();
      this.openWorkspace(deviceId, workspaceId, checkoutId);
      ui().showLane();
      dispatch({ schema_version: 2, kind: "detail_open", payload: { item: { kind: "record", session } } });
    },
    openRecordedIssue(session: import("./snapshot").WorkSession, repository: string, number: number) {
      const project = catalogWorkspaces(rest()).find((row) => row.device_id === session.device_id && row.tasks?.source?.name?.toLowerCase() === repository.toLowerCase());
      if (project) this.openBacklogItem(project.device_id, project.id, `github:${repository.toLowerCase()}#${number}`);
      else this.openLink(`https://github.com/${repository}/issues/${number}`, true);
    },
    openRecordedOutput(output: import("./snapshot").WorkOutput) {
      if (output.kind === "pr") this.openSessionPullRequest({ workspace_id: null, number: output.number, url: output.url });
      else this.openRecordedIssue(output.source, output.repository, output.number);
    },

    toggleTool,
    toggleFileTree,
    closeLane,
    setLanePinned,

    toggleLane,

    setWorkspaceView,

    revealInExplorer,

    showTool,

    /** A History row's diff in the active View area's preview (a single click) or pinned (a double click). */
    selectChange(path: string, committed: boolean, preview: boolean) {
      selectChangeIn(path, committed, preview, false);
    },

    /** "Open to the side" on a History row (S7 B4): its diff pinned beside the active View area. */
    openChangeBeside(path: string, committed: boolean) {
      selectChangeIn(path, committed, false, true);
    },

    openShortcuts() {
      ui().openOverlay(ui().overlay === "shortcuts" ? "none" : "shortcuts");
    },

    /**
     * ⌘F on the focused pane. An agent that searches its own conversation
     * hands the question to the core, which opens the agent's search or
     * answers that this pane's find bar should open (`FindBar` acts on the
     * answer); any other pane's find bar opens now. The core searches a
     * device pane through that device's Herdr.
     */
    openFind(target?: string) {
      if (ui().overlay === "find") {
        document.querySelector<HTMLInputElement>("[data-find-bar] input")?.focus();
        return;
      }
      // A pointed-at pane is focused by the pointing; the find bar follows the
      // focused pane, so only the agent's own search needs the target named.
      const paneId = target ?? useShellStore.getState().focusedPaneId;
      const agent = paneId ? contextAgents(rest(), rest()?.navigator?.agents ?? []).find((row) => row.pane_id === paneId) : undefined;
      if (paneId && agent?.own_find) {
        const requestId = remoteRequestId();
        ui().setAgentFindRequest({ id: requestId, owner: keyboardOwner() });
        dispatch({ schema_version: 2, kind: "pane_find_open", payload: { pane_id: paneId, request_id: requestId } });
        return;
      }
      ui().openOverlay("find");
    },

    /** Add a project; a browser tab has no folder picker, so only the desktop app offers it. */
    openAddProject() {
      if (!hostBridge()) return diagnostic("add project: this host has no folder picker");
      ui().openOverlay("add_project");
    },

    /** ⌘P: the file palette over hided's index of the focused checkout. */
    openFilePalette() {
      ui().openOverlay(ui().overlay === "file_palette" ? "none" : "file_palette");
    },

    /** ⌘K: the palette that searches the snapshot's agents, projects and checkouts. */
    openSearch() {
      ui().openOverlay(ui().overlay === "search" ? "none" : "search");
    },

    /** Asks for the index of `root` on `device`; a device's is walked by its helper. */
    requestFileIndex(root: string, query: string, device: string) {
      dispatch({ schema_version: 2, kind: "file_index", payload: { root, query, ...deviceField(device, localDeviceId(rest())) } });
    },

    /** A palette pick opens in the checkout's preview tab (B12) and closes the palette. */
    openIndexEntry(path: string) {
      ui().closeOverlay();
      // The core's selected_path may already be this file from an earlier
      // open, so the row is highlighted from the pick itself (B3).
      ui().setExplorerSelection(path);
      openInFront(path, true, "file_open");
    },

    /** A pick from "Open file to the side": a pinned display beside the active area (S7 B4). */
    openIndexEntryBeside(path: string) {
      ui().closeOverlay();
      ui().setExplorerSelection(path);
      openInFront(path, false, "file_open", true);
    },

    /** Registers a folder on `deviceId`; a device's own helper judges it against that device's home. */
    createWorkspace(path: string, label: string, deviceId = localDeviceId(rest())) {
      dispatch({ schema_version: 2, kind: "create_workspace", payload: { ...deviceField(deviceId, localDeviceId(rest())), path, label, initialize_git: false } });
    },

    /** Asks hided whether a clone could land at `parent/name`; answered as a `clone_target` frame. */
    checkCloneTarget(parent: string, name: string) {
      dispatch({ schema_version: 2, kind: "clone_target", payload: { parent, name } });
    },

    /** Clones `url` into `parent/name` on this machine; the core reports it as `repository_clone`. */
    cloneRepository(url: string, parent: string, name: string) {
      dispatch({ schema_version: 2, kind: "clone_repository", payload: { url, parent, name } });
    },

    cancelRepositoryClone(id: number) {
      dispatch({ schema_version: 2, kind: "cancel_repository_clone", payload: { id } });
    },

    /** Asks hided where Create new project would make `name` in `parent`; answered as a `project_target` frame. */
    probeProjectTarget(parent: string, name: string) {
      dispatch({ schema_version: 2, kind: "project_target", payload: { parent, name } });
    },

    /** Makes `path` as a new folder with its own Git repository, then registers it; on the core's machine only, whichever window asks. */
    createProject(path: string, label: string) {
      dispatch({ schema_version: 2, kind: "create_workspace", payload: { path, label, initialize_git: true, new_folder: true } });
    },

    /** Removes a registration after its panes close (D-10); the folder is never touched. */
    removeWorkspace(workspaceId: string, closeDescendants: string[] = []) {
      dispatch({ schema_version: 2, kind: "remove_workspace", payload: { workspace_id: workspaceId, ...(closeDescendants.length > 0 ? { close_descendant_pane_ids: closeDescendants } : {}) } });
    },

    /** One checkout folder's children, answered by hided as a `directory_list`. */
    listChildren(root: string, path: string) {
      const device = explorerContext(rest()).device;
      dispatch({ schema_version: 2, kind: "file_list", payload: { root, path, ...deviceField(device, localDeviceId(rest())) } });
    },

    /** The core owns which folders the tree has expanded, per device; this replaces the selected device's set. */
    setExpandedPaths(paths: string[]) {
      updateUiState(expandedPatch(explorerContext(rest()).device, paths));
    },

    /** A single click opens the active area's preview slot; a double click pins it. */
    openFile(path: string, preview: boolean) {
      openInFront(path, preview, "file_open");
    },

    /** "Open to the side" (S7 B4): a second, pinned display in the next area. */
    openFileBeside(path: string) {
      openInFront(path, false, "file_open", true);
    },

    /**
     * A browser display of `url` in a Workspace (issue 155): the front one,
     * or the one a page that opened a tab belongs to, beside that page
     * (`besideDisplay`) so it stays in view. An address the Workspace
     * already shows is focused and loaded again.
     */
    openBrowser(url: string, workspace?: ViewWorkspace, areaId?: string, besideDisplay?: string, openerPane?: string) {
      const target = workspace ?? frontViewWorkspace();
      if (!target) return diagnostic("browser_open: no Workspace in front");
      // The keyboard goes with a page the operator opened, as with a file;
      // a page's own new tab leaves it with the page that asked, and a New
      // tab's address field takes it as it mounts.
      const follows = url !== "" && !besideDisplay;
      dispatch({
        schema_version: 2,
        kind: "browser_open",
        payload: {
          url,
          workspace: { device_id: target.device_id, path: target.path },
          ...(areaId ? { area_id: areaId } : {}),
          ...(besideDisplay ? { beside_display: besideDisplay } : {}),
          ...(openerPane ? { opener_pane: openerPane } : {}),
          ...(follows ? { request_id: followBrowserOpen(target) } : {}),
        },
      });
    },

    /**
     * A web address a pane or a card links to: a browser display in the
     * Workspace in front, or the default browser when the operator asked for
     * it with ⌘ (Ctrl off macOS) or when no Workspace is in front (All
     * projects, an Overview). The default browser is reached the way a page's
     * own link is: the desktop host routes `window.open` to the system, and a
     * browser tab opens a tab.
     */
    openLink(url: string, external: boolean, openerPane?: string) {
      // The core keeps a Workspace in front while the page shows All projects
      // or an Overview; a display opened there would land out of sight.
      const inFront = ui().screen?.kind === "workspace" ? frontViewWorkspace() : null;
      if (external || !inFront) {
        window.open(url, "_blank", "noopener,noreferrer");
        return;
      }
      this.openBrowser(url, undefined, undefined, undefined, openerPane);
    },

    /** A checkout's pull request (PRD checkout-pr-glyph-card D-02, D-11): a link, and the default browser's for an SSH device's checkout. */
    openPullRequest(url: string, deviceId: string, external: boolean) {
      this.openLink(url, external || deviceId !== localDeviceId(rest()));
    },

    /** A project's GitHub repository page: the default browser's for an SSH device's project, as for its pull request. */
    openRepository(repository: string, deviceId: string, external: boolean) {
      this.openLink(`https://github.com/${repository}`, external || deviceId !== localDeviceId(rest()));
    },

    /**
     * A path a terminal link names on this window's machine (docs/ARCHITECTURE.md, A
     * clicked path is one event). Inside a registered checkout it is one
     * event naming the checkout whose physical root holds it longest:
     * `browser_open` for HTML, `file_open` or `reveal_path` for another file
     * or Open source, `reveal_path` for a folder. A file of another checkout
     * opens in the Workspace in front and moves no checkout; a file of the
     * checkout in front, or a folder, is revealed with its checkout forward.
     * Outside every checkout, or on ⌘ (Ctrl off macOS), the desktop host
     * hands it to the operating system.
     */
    openTerminalPath(found: FoundPath, line: number | null, column: number | null, external: boolean, mode: "auto" | "source" = "auto", openerPane?: string) {
      const bridge = hostBridge();
      if (!bridge) return diagnostic("terminal link: this host cannot open a path");
      if (external) return bridge.openPath(found.real);
      // The pane's agent opens what lands in View (PRD view-opener-feedback D-02).
      const opener = openerPane ? { opener_pane: openerPane } : {};
      void terminalPathOwner(found).then((owner) => {
        if (owner === undefined) return;
        if (!owner) {
          bridge.openPath(found.real);
          return;
        }
        const { path, deviceId } = owner;
        // The core keeps a Workspace in front while the page shows All
        // projects or an Overview; a file opened there would land out of sight.
        const front = ui().screen?.kind === "workspace" ? frontViewWorkspace() : null;
        const here = found.kind === "file" && front?.device_id === deviceId && front.path !== owner.checkout.path ? front : null;
        if (found.kind === "file" && isHtmlFile(path) && mode === "auto") {
          const request_id = followBrowserOpen(here ?? { device_id: deviceId, path: owner.checkout.path });
          dispatch({
            schema_version: 2,
            kind: "browser_open",
            payload: here
              ? { url: fileUrl(path), workspace: here, request_id, ...opener }
              : { url: fileUrl(path), workspace: { device_id: deviceId, path: owner.checkout.path }, reveal: true, request_id, ...opener },
          });
          return;
        }
        if (found.kind === "file" && line !== null) requestLine([path, found.real], line, column);
        if (found.kind === "file") followOpen(path, "file", null, here ?? { device_id: deviceId, path: owner.checkout.path });
        // A node's own checkout is its machine's path: hided leaves it to that node's helper, not the core's roots.
        const device = deviceField(deviceId, localDeviceId(rest()));
        if (here) {
          dispatch({
            schema_version: 2,
            kind: "file_open",
            payload: { path, workspace_id: owner.checkout.workspace_id, checkout_id: owner.checkout.id, preview: false, view: here, ...device, ...opener },
          });
          return;
        }
        dispatch({
          schema_version: 2,
          kind: "reveal_path",
          payload: { path, workspace_id: owner.checkout.workspace_id, checkout_id: owner.checkout.id, is_directory: found.kind === "directory", ...device, ...opener },
        });
      });
    },

    /** Explorer "Open in Browser" on an HTML file of this machine's checkout. */
    openInBrowser(path: string) {
      const target = frontViewWorkspace();
      if (!target) return diagnostic("browser_open: no Workspace in front");
      dispatch({ schema_version: 2, kind: "browser_open", payload: { url: fileUrl(path), workspace: { device_id: target.device_id, path: target.path }, request_id: followBrowserOpen(target) } });
    },

    /**
     * One change to the operator's comments on a View (PRD
     * view-opener-feedback): add, edit, delete, undo or relocate. An image
     * or PDF comment's crop rides as `shot_png`, which hided stages before
     * the core sees the event (D-23).
     */
    commentView(workspace: ViewWorkspace, action: { action: "add" | "edit" | "delete" | "undo" | "relocate" } & Record<string, unknown>) {
      dispatch({ schema_version: 2, kind: "view_comment", payload: { workspace: { device_id: workspace.device_id, path: workspace.path }, ...action } });
    },

    /**
     * Sends a recipient group's comments as one letter from the operator
     * (B16-B20). A Retry sends the same `requestId` with `to` `retry`: the
     * core sends again what that Send asked, and a letter the first try
     * stored is not stored twice (B27, engineering principle 11).
     */
    sendViewFeedback(workspace: ViewWorkspace, requestId: string, group: string, to: { kind: "default" } | { kind: "pane"; pane_id: string } | { kind: "reopen" } | { kind: "retry" }, displayId: string | null) {
      dispatch({
        schema_version: 2,
        kind: "view_feedback_send",
        payload: { workspace: { device_id: workspace.device_id, path: workspace.path }, request_id: requestId, group, to, ...(displayId ? { display_id: displayId } : {}) },
      });
    },

    /**
     * Comment mode on a browser View, on or off, and the pins it draws
     * (B9-B11): hided holds the page while it is on (`browser_comment`).
     */
    browserComment(payload: Record<string, unknown>) {
      dispatch({ schema_version: 2, kind: "browser_comment", payload });
    },

    /** The address field: the display loads what was typed. */
    navigateBrowser(displayId: string, url: string) {
      const frame = frameFor("navigate");
      if (frame) viewLayout(frame, { action: "navigate", display_id: displayId, url });
    },


    /** What a page says (its address and title), recorded so the tab and a relaunch show it. */
    reportBrowserState(workspace: ViewWorkspace, displayId: string, url: string, title: string, load: number, loading: boolean, failure: string | null, present: boolean) {
      dispatch({
        schema_version: 2,
        kind: "browser_state",
        payload: { workspace: { device_id: workspace.device_id, path: workspace.path }, display_id: displayId, url, title, load, loading, failure, present },
      });
    },

    /** ⌘⇧K, a double click or Keep open: a preview display becomes an ordinary one. */
    keepViewOpen,

    focusView,
    /** A View an agent opened, chosen from its pane's `N views` (B8): the lane shows the files tool and the View comes forward. */
    revealView(displayId: string) {
      showTool("files");
      focusView(displayId, true);
    },
    focusViewArea,
    moveView,
    splitView,
    resizeViewSplit,
    closeView,
    closeViewWithoutSaving,
    runViewMenu,
    runViewArea,

    /** Re-reads an unavailable display's file (S7 B16). */
    retryView(displayId: string) {
      const frame = frameFor("retry");
      if (frame) viewLayout(frame, { action: "retry", display_id: displayId });
    },

    /** A new file or folder in `parent`; the core opens a created file (B9). */
    createEntry(parent: string, name: string, isDirectory: boolean) {
      const here = explorerTarget();
      if (!here) return diagnostic("explorer create: no focused checkout");
      dispatch({
        schema_version: 2,
        kind: isDirectory ? "dir_create" : "file_create",
        payload: { ...here, parent, name },
      });
    },

    renameEntry(path: string, name: string) {
      const here = explorerTarget();
      if (!here) return diagnostic("path_rename: no focused checkout");
      dispatch({ schema_version: 2, kind: "path_rename", payload: { ...here, path, name } });
    },

    /** A drag that landed: one `path_move` into the folder it was dropped on. */
    moveEntry(path: string, destination: string) {
      const here = explorerTarget();
      if (!here) return diagnostic("path_move: no focused checkout");
      dispatch({ schema_version: 2, kind: "path_move", payload: { ...here, path, destination } });
    },

    /** A paste of a copy, or Duplicate: one `path_copy` of `path` into the folder `destination`; the host names the copy when the name is taken. */
    copyEntry(path: string, destination: string) {
      const here = explorerTarget();
      if (!here) return diagnostic("path_copy: no focused checkout");
      ui().setExplorerNotice(null);
      dispatch({ schema_version: 2, kind: "path_copy", payload: { ...here, path, destination } });
    },

    /** Files and folders dropped from the file manager into the folder `destination` (docs/UI_BEHAVIOR.md, Explorer). */
    importDropped(destination: string, dropped: CapturedDrop) {
      const here = explorerTarget();
      if (!here) return diagnostic("path_import: no focused checkout");
      void submitDrop(here, destination, dropped);
    },

    /** Opens the trash confirmation; nothing is dispatched until it is confirmed. */
    requestTrash(path: string, name: string, isDirectory: boolean, selectAfter: string, inode: string | null) {
      const target = explorerTarget();
      if (!target) return diagnostic("path_trash: no focused checkout");
      ui().setPendingTrash({ path, name, isDirectory, selectAfter, inode, target });
    },

    confirmTrash() {
      const pending = ui().pendingTrash;
      ui().setPendingTrash(null);
      if (!pending) return;
      dispatch({
        schema_version: 2,
        kind: "path_trash",
        payload: {
          ...pending.target,
          path: pending.path,
          select_after: pending.selectAfter,
          inode: pending.inode,
        },
      });
    },

    cancelTrash() {
      ui().setPendingTrash(null);
    },

    /** One keystroke's contents for one document; the core keeps the draft and the dirty flag. */
    updateDraft(tabId: string, contents: string) {
      dispatch({ schema_version: 2, kind: "file_draft", payload: { tab_id: tabId, contents_utf8: contents } });
    },

    /**
     * A save of the showing document, or of the named tab: the expected
     * modification time is the one the core read when it opened the file, so a
     * disk change since then makes the save a conflict rather than a silent
     * overwrite (B5).
     */
    saveFile,

    /** A document's Markdown mode and wrap choice; the core persists both. */
    setFileView(tabId: string, live: boolean, wrap: boolean) {
      dispatch({ schema_version: 2, kind: "file_view", payload: { tab_id: tabId, markdown_live: live, wrap } });
    },

    /** "reload" reads the disk contents; "keep_editing" accepts the disk
     * timestamp so the next save overwrites. */
    resolveConflict(tabId: string, action: "reload" | "keep_editing") {
      dispatch({ schema_version: 2, kind: "file_conflict", payload: { tab_id: tabId, action } });
    },

    /** ⌘F for one display, by default the active one: only its editor opens its find panel. */
    requestEditorFind(displayId?: string) {
      ui().requestEditorFind(displayId ?? activeDisplayNow()?.display.id ?? null);
    },

    /** `step` 0 searches and keeps the current match; +1 and -1 move (core `PaneFindPayload`). */
    find(paneId: string, term: string, step: -1 | 0 | 1) {
      dispatch({
        schema_version: 2,
        kind: "pane_find",
        payload: { pane_id: paneId, term, case_sensitive: false, whole_word: false, regex: false, step },
      });
    },
  };
}
