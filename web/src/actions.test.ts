// @vitest-environment jsdom
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { clearProbeCache } from "./terminalLinkProvider";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { frontDeviceId } from "./devices";
import { overviewDeviceOf } from "./overviewTasks";
import { frontTool } from "./toolCells";
import { workspaceKey } from "./viewLayout";
import type { DispatchFn } from "./ws";
import { FindBar } from "./Overlays";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Checkout, SnapshotRest } from "./snapshot";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { attachTerminal, terminalFor } from "./terminals";
import { installKeyboardOwner, keyboardCommandOwner, noteCommandDelivered, noteKeyboardOwner, type KeyboardOwner } from "./viewFocus";
import { emptyScope } from "../test/legacyAgentScope";

// jsdom has no canvas. Keep real actions and terminals while answering
// xterm's canvas capability probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
afterEach(() => useUiStore.getState().setViewFocusRequest(null));

it.each([
  { arrival: "before", folded: false, input: "terminal" },
  { arrival: "after", folded: false, input: "terminal" },
  { arrival: "after", folded: true, input: "terminal" },
  { arrival: "after", folded: true, input: "tool" },
  { arrival: "after", folded: false, input: "popup" },
])("returns a hidden native page's keyboard without replacing newer input ($arrival, folded=$folded, input=$input)", async ({ arrival, folded, input }) => {
  const saved = useShellStore.getState();
  const savedUi = useUiStore.getState();
  const previousHost = window.hideHost;
  const previousStyle = document.documentElement.getAttribute("style");
  document.documentElement.style.setProperty("--text-terminal-base", "14px");
  document.documentElement.style.setProperty("--font-mono", "monospace");
  let nativeFocus: (event: import("./host").BrowserHostEvent) => void = () => {};
  const workspace = { device_id: "local", path: "/repo" };
  const scope = emptyScope();
  scope.closes["p"] = { decision: { action: "close" }, subtree: null, subtree_all: null, stop_work: { rows: [], unknown: null } };
  const checkout: Checkout = {
    id: "c", workspace_id: "w", path: "/repo", label: "repo", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: scope, next_tab_label: "Tab 2",
    tabs: [{ id: "t", workspace_id: "w", checkout_id: "c", label: "Tab 1", empty: false, delegated: false, panes: [{ id: "p", device_id: "local", device_pane_id: "p", cwd: "/repo", engine_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: "t", strip: [],
  };
  const rest: SnapshotRest = {
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", agent_scope: scope, devices: [], workspaces: [{ id: "w", label: "repo", device_id: "local", path: "/repo", registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: scope, checkouts: [checkout] }] },
    workspace_view: { ...workspace, open: true, tray: { tab_id: "t1", shown: true, count: 0 }, tool: folded ? "explorer" : null, document_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", layout: { active_area: "a1", limits: { areas: 8, depth: 4, displays: 32 }, display_count: 1, root: { area: { id: "a1", active: "b", displays: [{ id: "b", tab_id: null, path: "", committed: null, preview: false, reason: null, kind: "browser", state: "open", label: "page", url: "https://page.test/" }] } } } },
    ui_state: {},
  };
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }));
  const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1024);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Object.assign(window, { hideHost: { kind: "electron", browser: { onEvent: (listener: typeof nativeFocus) => { nativeFocus = listener; return () => {}; }, sync() {}, command() {} } } });
  document.body.innerHTML = '<div data-workspace-screen="c"><div data-pane-view="p"></div><aside data-lane="explorer"><input data-live-tool /></aside></div><div role="dialog"><input data-popup /></div><div data-screen></div>';
  const root = createRoot(document.querySelector("[data-screen]")!);
  const sent: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { sent.push(event); return true; });
  useShellStore.setState({ rest, connection: "live", focusedPaneId: "p", agents: [], editor: null });
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
  let detach: (() => void) | undefined;
  let remove: (() => void) | undefined;
  try {
    detach = attachTerminal("p", document.querySelector("[data-pane-view]")!, (event) => { sent.push(event); return true; }, actions, 1);
    const terminal = terminalFor("p")!.textarea!;
    remove = installKeyboardOwner();
    await act(async () => root.render(createElement(TooltipProvider, null, createElement(WorkspaceScreen, { actions }))));
    expect(useUiStore.getState().drawnLane.mode).not.toBe("closed");
    act(() => terminal.focus());
    // Reopening the lane requested native focus, but its report has not yet
    // reached the shell when the next menu command closes the lane.
    const focus = () => act(() => nativeFocus({ kind: "focus", workspace: workspaceKey(workspace), id: "b" }));
    if (arrival === "before") focus();
    act(() => {
      if (folded) actions.setWorkspaceView({ documents: false });
      else actions.closeLane();
      noteCommandDelivered();
    });
    if (arrival === "after") focus();
    expect(keyboardCommandOwner()).toMatchObject({ kind: arrival === "after" ? "view" : "pane", workspace: "c" });
    const destination = input === "terminal" ? terminal : document.querySelector<HTMLInputElement>(input === "tool" ? "[data-live-tool]" : "[data-popup]")!;
    if (input !== "terminal") act(() => destination.focus());
    await act(async () => useShellStore.setState({ rest: { ...rest, workspace_view: { ...rest.workspace_view!, ...(folded ? { tray: { ...rest.workspace_view!.tray, shown: false } } : { open: false }) } } }));
    // The terminal remained the shell's active element throughout native
    // focus. Returning it cannot depend on another DOM focus event.
    expect(document.activeElement).toBe(destination);
    act(() => actions.closeFocused());
    expect(sent.filter((event) => event.kind === "close_pane")).toEqual(input === "terminal" ? [{ schema_version: 2, kind: "close_pane", payload: { pane_id: "p", confirmed: false } }] : []);
    expect(keyboardCommandOwner()).toEqual(input === "terminal" ? { kind: "pane", workspace: "c", paneId: "p" } : input === "tool" ? { kind: "tool", workspace: "c" } : { kind: "none" });
  } finally {
    await act(async () => root.unmount());
    remove?.();
    detach?.();
    Object.assign(window, { hideHost: previousHost });
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    document.body.innerHTML = "";
    vi.unstubAllGlobals();
    width.mockRestore();
    if (previousStyle === null) document.documentElement.removeAttribute("style");
    else document.documentElement.setAttribute("style", previousStyle);
  }
});

describe("which pages the keyboard follows", () => {
  const workspace = { device_id: "local", path: "/repo" };
  const opened = () => {
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    const requestIds = () => sent.filter((event) => event.kind === "browser_open").map((event) => (event.payload as { request_id?: string }).request_id);
    return { actions, requestIds };
  };

  it("names a page the operator opens and waits for the display the core answers with", () => {
    const { actions, requestIds } = opened();
    actions.openBrowser("https://a.test/", workspace, undefined, undefined, "pane-1");
    actions.openBrowser("https://pr.test/", workspace);
    const [fromLink, fromRow] = requestIds();
    expect(fromLink).toMatch(/^follow:/);
    expect(fromRow).toMatch(/^follow:/);
    expect(fromRow).not.toBe(fromLink);
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: workspaceKey(workspace), browserOpen: fromRow });
  });

  it("leaves the keyboard alone for a New tab and a page's own new tab", () => {
    const { actions, requestIds } = opened();
    actions.openBrowser("", workspace, "a1");
    actions.openBrowser("https://b.test/", workspace, undefined, "page-1");
    expect(requestIds()).toEqual([undefined, undefined]);
    expect(useUiStore.getState().viewFocusRequest).toBeNull();
  });
});

// In another computer's screen a ⌘-click or Open in default browser on that
// computer's loopback, LAN or local name would reach nothing: the showing
// window's default browser opens only public addresses. Such a link opens
// as a page in the Workspace in front, which reaches it through that
// computer (docs/BROWSER_DISPLAYS.md, A page in another computer's screen).
describe("a link of another computer's screen", () => {
  it("opens that computer's local address as a page in front, and a public one in the default browser", () => {
    const saved = useShellStore.getState();
    const savedUi = useUiStore.getState();
    const opened = vi.fn();
    vi.stubGlobal("hideRemote", { bridge: 1, socket: {}, clipboardWrite: async () => true, onMenuCommand: () => {}, onVisible: () => {} });
    vi.spyOn(window, "open").mockImplementation((...args) => { opened(...args); return null; });
    try {
      useShellStore.setState({ rest: { workspace_view: { device_id: "mini", path: "/web", layout: null } } as never });
      useUiStore.setState({ screen: { kind: "workspace" } as never });
      const sent: Parameters<DispatchFn>[0][] = [];
      const actions = createActions((event) => { sent.push(event); return true; });
      for (const url of ["http://localhost:5173/", "http://192.168.0.4:8080/", "http://printer.local/"]) actions.openLink(url, true);
      expect(sent.filter((event) => event.kind === "browser_open").map((event) => (event.payload as { url: string; workspace: unknown }).url)).toEqual(["http://localhost:5173/", "http://192.168.0.4:8080/", "http://printer.local/"]);
      expect(opened).not.toHaveBeenCalled();
      actions.openLink("https://github.com/with-hide/hide", true);
      expect(opened).toHaveBeenCalledExactlyOnceWith("https://github.com/with-hide/hide", "_blank", "noopener,noreferrer");
    } finally {
      useShellStore.setState(saved, true);
      useUiStore.setState(savedUi, true);
      vi.restoreAllMocks();
      vi.unstubAllGlobals();
    }
  });
});

describe("Find while an agent answers its own search request", () => {
  it.each([false, true])("follows the answer only while the keyboard stays at its origin (moved: %s)", async (moved) => {
    const { act, createElement } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { installKeyboardOwner, keyboardCommandOwner, keyboardOwner, noteKeyboardOwner } = await import("./viewFocus");
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    useUiStore.setState({ overlay: "none", agentFindRequest: null, screen: { kind: "workspace" } });
    useShellStore.setState({ rest: { navigator: { agents: [{ pane_id: "pane-1", own_find: true }] } } as never, focusedPaneId: "pane-1", find: null });
    document.body.innerHTML = '<div data-workspace-screen="c"><div data-pane-view="pane-1"><textarea></textarea></div><div data-view-area-id="a1"><input data-document /></div></div><button role="menu">Find</button><div data-find-host></div>';
    const remove = installKeyboardOwner();
    const root = createRoot(document.querySelector("[data-find-host]")!);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    try {
      act(() => root.render(createElement(TooltipProvider, null, createElement(FindBar, { actions }))));
      document.querySelector("textarea")!.focus();
      // A menu borrows focus; its command still belongs to the pane.
      document.querySelector<HTMLButtonElement>('[role="menu"]')!.focus();
      expect(keyboardCommandOwner()).toEqual({ kind: "pane", workspace: "c", paneId: "pane-1" });
      expect(keyboardOwner()).toEqual({ kind: "none" });
      act(() => actions.openFind());
      const pending = useUiStore.getState().agentFindRequest!;
      expect(sent).toEqual([{ schema_version: 2, kind: "pane_find_open", payload: { pane_id: "pane-1", request_id: pending.id } }]);
      expect(document.querySelector("[data-find-bar]")).toBeNull();
      const documentInput = document.querySelector<HTMLInputElement>("[data-document]")!;
      if (moved) documentInput.focus();
      act(() => useShellStore.setState({ find: { pane_id: "pane-1", term: "", index: 0, total: 0, opened: { request_id: pending.id, route: "bar" } } as never }));
      expect(useUiStore.getState().agentFindRequest).toBeNull();
      if (moved) {
        expect(document.querySelector("[data-find-bar]")).toBeNull();
        expect(document.activeElement).toBe(documentInput);
      } else {
        expect(document.querySelector("[data-find-bar] input")).not.toBeNull();
        expect(document.activeElement).toBe(document.querySelector("[data-find-bar] input"));
      }
    } finally {
      act(() => root.unmount());
      remove();
      noteKeyboardOwner({ kind: "none" });
      document.body.innerHTML = "";
      useUiStore.setState({ overlay: "none", agentFindRequest: null });
      useShellStore.setState({ rest: null, focusedPaneId: null, find: null });
      vi.unstubAllGlobals();
    }
  });
});

// The MacBook's window after the core moved to the Mac mini: its own checkout
// is in the node's session, and the path a pane prints is the MacBook's.
describe("a terminal link to a file of this window's own checkout", () => {
  const checkout = { id: "remote:mbp:checkout:1", workspace_id: "remote:mbp:project:1", path: "/work/proj-m", exists: true };
  const rest = {
    navigator: { devices: [{ id: "core", kind: "local" }, { id: "mbp", kind: "remote", dials_in: true }], workspaces: [] },
    status: { remote: [{ target_id: "mbp", session: { workspaces: [{ id: checkout.workspace_id, device_id: "mbp", path: checkout.path, checkouts: [checkout] }] } }] },
  } as never;
  const revealed = async (hash: string) => {
    window.location.hash = hash;
    const sent: Parameters<DispatchFn>[0][] = [];
    createActions((event) => { sent.push(event); return true; }).openTerminalPath({ real: "/work/proj-m/README.md", kind: "file" }, null, null, false);
    await vi.waitFor(() => expect(sent.some((event) => event.kind === "reveal_path")).toBe(true));
    return sent.find((event) => event.kind === "reveal_path")?.payload;
  };

  afterEach(() => {
    window.location.hash = "";
    clearProbeCache();
    Reflect.deleteProperty(window, "hideHost");
    useShellStore.setState({ rest: null });
  });

  it("names the node, so the core's machine never judges the path against its own checkouts", async () => {
    Object.assign(window, { hideHost: { kind: "electron", probePaths: async (paths: string[]) => paths.map((path) => ({ real: path, kind: "directory" })), openPath: vi.fn() } });
    useShellStore.setState({ rest });
    expect(await revealed("#token=t&node=mbp")).toMatchObject({ path: "/work/proj-m/README.md", workspace_id: checkout.workspace_id, checkout_id: checkout.id, device_id: "mbp" });
  });

  it("names no device for a checkout of the core's own machine", async () => {
    Object.assign(window, { hideHost: { kind: "electron", probePaths: async (paths: string[]) => paths.map((path) => ({ real: path, kind: "directory" })), openPath: vi.fn() } });
    const own = { ...checkout, id: "c1", workspace_id: "w1" };
    useShellStore.setState({ rest: { navigator: { devices: [{ id: "core", kind: "local" }], workspaces: [{ id: "w1", device_id: "core", path: own.path, checkouts: [own] }] }, status: { remote: [] } } as never });
    const payload = await revealed("#token=t");
    expect(payload).toMatchObject({ checkout_id: "c1" });
    expect(payload).not.toHaveProperty("device_id");
  });
});

// PRD outside-path-in-hide: a file outside every checkout opens in Hide from
// a desktop window on the core's machine, by the grant its host gives it.
describe("a terminal link to a file outside every checkout", () => {
  const own = { id: "c1", workspace_id: "w1", path: "/repo", exists: true };
  const grant = { id: "0123456789abcdef0123456789abcdef", name: "shot.png", kind: "image" as const };
  const host = (granted: typeof grant | null) => {
    const external = { open: vi.fn(async () => granted), read: vi.fn(), describe: vi.fn(), live: vi.fn() };
    const openPath = vi.fn();
    Object.assign(window, { hideHost: { kind: "electron", probePaths: async (paths: string[]) => paths.map((path) => ({ real: path, kind: "directory" })), openPath, external } });
    return { external, openPath };
  };
  const clicked = (link = "/tmp/shot.png", node = "#token=t") => {
    window.location.hash = node;
    useShellStore.setState({ rest: { navigator: { devices: [{ id: "core", kind: "local" }, { id: "mbp", kind: "remote", dials_in: true }], workspaces: [{ id: "w1", device_id: "core", path: own.path, checkouts: [own] }] }, status: { remote: [] }, workspace_view: { device_id: "core", path: own.path } } as never });
    useUiStore.setState({ screen: { kind: "workspace" } } as never);
    const sent: Parameters<DispatchFn>[0][] = [];
    createActions((event) => { sent.push(event); return true; }).openTerminalPath({ real: link, kind: "file" }, 3, null, false, "auto", "pane-1");
    return sent;
  };

  afterEach(() => {
    window.location.hash = "";
    clearProbeCache();
    Reflect.deleteProperty(window, "hideHost");
    useShellStore.setState({ rest: null });
  });

  it("opens it as a tab of the Workspace in front named by its grant alone, the pressed pane its owner (B1, D-05, D-54)", async () => {
    const { external, openPath } = host(grant);
    const sent = clicked();
    await vi.waitFor(() => expect(sent.some((event) => event.kind === "external_open")).toBe(true));
    expect(external.open).toHaveBeenCalledWith("/tmp/shot.png", "auto");
    const open = sent.find((event) => event.kind === "external_open")!;
    expect(open.payload).toEqual({ grant: grant.id, name: "shot.png", kind: "image", request_id: expect.stringMatching(/^follow:/), workspace: { device_id: "core", path: "/repo" }, opener_pane: "pane-1" });
    expect(JSON.stringify(sent)).not.toContain("/tmp");
    expect(openPath).not.toHaveBeenCalled();
  });

  it("sends nothing to the core when the host hands the file to the system instead (B7, B8, B17)", async () => {
    const { external } = host(null);
    const sent = clicked("/tmp/song.mp3");
    await vi.waitFor(() => expect(external.open).toHaveBeenCalled());
    await Promise.resolve();
    expect(sent.some((event) => event.kind === "external_open")).toBe(false);
  });

  it("hands it to the system from a window on another machine, which opens no outside file (D-09)", async () => {
    const { external, openPath } = host(grant);
    clicked("/tmp/shot.png", "#token=t&node=mbp");
    await vi.waitFor(() => expect(openPath).toHaveBeenCalledWith("/tmp/shot.png"));
    expect(external.open).not.toHaveBeenCalled();
  });
});

describe("input after a confirmed View close", () => {
  const workspace = { device_id: "local", path: "/repo" };
  const frame = (ids: string[], selected: string) => ({ ...workspace, open: true, tray: { tab_id: "t1", shown: true, count: 0 }, tool: "explorer", layout: { active_area: "a1", display_count: ids.length, root: { area: { id: "a1", active: selected, displays: ids.map((id) => ({ id, kind: "browser", state: "open", label: id, url: "", tab_id: null })) } } } });
  const setup = async () => {
    vi.stubGlobal("CSS", { escape: (value: string) => value });
    const { installKeyboardOwner } = await import("./viewFocus");
    const remove = installKeyboardOwner();
    const checkout = { id: "c", path: "/repo" };
    const rest = { navigator: { focused_checkout_id: "c", focused_workspace_id: "w", workspaces: [{ id: "w", device_id: "local", checkouts: [checkout] }] }, workspace_view: frame(["d1", "d2"], "d1") } as never;
    useShellStore.setState({ rest, connection: "live", editor: null });
    document.body.innerHTML = '<div data-workspace-screen="c"><div data-view-area-id="a1"><button role="tab" aria-selected="true">next</button><div class="cm-content" tabindex="0" contenteditable="true"></div></div><input data-other /></div>';
    const content = document.querySelector<HTMLElement>(".cm-content")!;
    content.focus();
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    const land = () => useShellStore.setState({ rest: { ...useShellStore.getState().rest, workspace_view: frame(["d2"], "d2") } as never });
    return { remove, actions, content, land, sent };
  };
  afterEach(() => { useShellStore.setState({ rest: null }); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

  it("waits for removal, then follows the replacement content", async () => {
    const { remove, actions, content, land } = await setup();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return 1; });
    try {
      actions.closeView("d1");
      expect(frames).toHaveLength(0);
      content.blur();
      land();
      frames.forEach((callback) => callback(0));
      expect(document.activeElement).toBe(content);
    } finally { remove(); }
  });

  it("keeps newer input when a delayed close finally removes its View", async () => {
    const { remove, actions, land } = await setup();
    const other = document.querySelector<HTMLInputElement>("[data-other]")!;
    try {
      actions.closeView("d1");
      other.focus();
      land();
      expect(document.activeElement).toBe(other);
    } finally { remove(); }
  });

  it("ends its pending focus follow when the save reports a failure", async () => {
    const { remove, actions, content, land } = await setup();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return 1; });
    try {
      actions.closeView("d1");
      useShellStore.setState({ rest: { ...useShellStore.getState().rest, status: { last_error: { occurred_at: 1 } } } as never });
      content.blur();
      land();
      expect(frames).toHaveLength(0);
      expect(document.activeElement).not.toBe(content);
    } finally { remove(); }
  });

  it("keeps View commands in its toolbar while suppressing canvas input ownership", async () => {
    const { remove, actions, content, sent } = await setup();
    const { keyboardOwner, keyboardCommandOwner, noteDrawnViews } = await import("./viewFocus");
    const header = document.createElement("div");
    header.dataset.browserToolbar = "";
    header.innerHTML = "<input />";
    content.parentElement!.append(header);
    useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "overlay", view: "documents", tool: null, stacked: false } });
    noteDrawnViews({ workspace, layout: frame(["d1", "d2"], "d1").layout } as never);
    try {
      header.querySelector("input")!.focus();
      expect(keyboardOwner().kind).toBe("tool");
      expect(keyboardCommandOwner()).toEqual({ kind: "view", workspace: "c", areaId: "a1" });
      actions.newTabFocused();
      expect(sent.at(-1)?.kind).toBe("browser_open");
      actions.closeFocused();
      expect(sent.at(-1)).toMatchObject({ kind: "view_layout", payload: { action: "close", display_id: "d1" } });
    } finally { noteDrawnViews(null); remove(); }
  });

  it("routes a host Find command from a View toolbar to its document", async () => {
    const { remove, actions, content } = await setup();
    const { installKeyboard } = await import("./keyboard");
    const { noteDrawnViews, keyboardOwner } = await import("./viewFocus");
    let command: (id: string) => void = () => {};
    const previousHost = window.hideHost;
    Object.assign(window, { hideHost: { kind: "electron", platform: "darwin", onCommand: (listener: typeof command) => { command = listener; return () => {}; }, reportBindings: vi.fn(), reportLanguage: vi.fn(), reportDesk: vi.fn() } });
    useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "overlay", view: "documents", tool: null, stacked: false } });
    useShellStore.setState({ editor: { active_tab_id: "d1" } as never });
    noteDrawnViews({ workspace, layout: frame(["d1", "d2"], "d1").layout, geometry: { areas: [{ id: "a1" }] } } as never);
    const stopKeyboard = installKeyboard(actions);
    const header = document.createElement("div");
    header.dataset.documentHeader = "";
    header.innerHTML = "<button>Find in document</button>";
    content.parentElement!.append(header);
    try {
      header.querySelector("button")!.focus();
      expect(keyboardOwner().kind).toBe("tool");
      const before = useUiStore.getState().editorFindRequest;
      command("find_in_pane");
      expect(useUiStore.getState().editorFindRequest).toBe(before + 1);
      expect(useUiStore.getState().editorFindDisplay).toBe("d1");
      expect(useUiStore.getState().overlay).not.toBe("find");
    } finally { stopKeyboard(); noteDrawnViews(null); window.hideHost = previousHost; remove(); }
  });

  it("toggles the pane owning the composer while the last core focus still names another pane", async () => {
    const { remove, actions } = await setup();
    useUiStore.setState({ screen: { kind: "workspace" } });
    useShellStore.setState({ focusedPaneId: "w1:p2", agents: [
      { pane_id: "w1:p1", thread_capability: "ready", thread_identity: "one" },
      { pane_id: "w1:p2", thread_capability: "ready", thread_identity: "two" },
    ] as never });
    noteKeyboardOwner({ kind: "pane", workspace: workspace.path, paneId: "w1:p1" });
    try {
      actions.deskTerminal();
      expect(useUiStore.getState().paneConversation).toEqual({ paneId: "w1:p1", session: "one" });
      actions.deskTerminal();
      expect(useUiStore.getState().paneConversation).toBeNull();
    } finally { noteKeyboardOwner({ kind: "none" }); remove(); }
  });

  it("toggles a ready pane with Desk off or on, without returning it to Desk", async () => {
    const { remove, actions } = await setup();
    const { installKeyboard } = await import("./keyboard");
    let command: (id: string) => void = () => {};
    const reportDesk = vi.fn();
    const previousHost = window.hideHost;
    Object.assign(window, { hideHost: { kind: "electron", platform: "darwin", onCommand: (listener: typeof command) => { command = listener; return () => {}; }, reportBindings: vi.fn(), reportLanguage: vi.fn(), reportDesk } });
    const desk = (enabled: boolean) => ({ settings: { enabled, default_agent: null, location: "desk", cap: 8 } }) as never;
    useUiStore.setState({ screen: { kind: "workspace" } });
    // Even a pane opened from Desk uses the general in-place conversation command.
    useShellStore.setState({ agents: [{ pane_id: "w1:p1", thread_capability: "ready", thread_identity: "session-one" }] as never, desk: desk(false), deskJobs: { jobs: [{ id: "job-1", pane_id: "w1:p1" }], workers: [] } as never, focusedPaneId: "w1:p1" });
    const stopKeyboard = installKeyboard(actions);
    try {
      expect(reportDesk).toHaveBeenLastCalledWith(false);
      command("desk_terminal");
      expect(useUiStore.getState().paneConversation).toEqual({ paneId: "w1:p1", session: "session-one" });
      actions.openDesk();
      expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });

      useShellStore.setState({ desk: desk(true) });
      expect(reportDesk).toHaveBeenLastCalledWith(true);
      command("desk_terminal");
      expect(useUiStore.getState().paneConversation).toBeNull();
      expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });
      useShellStore.setState({ agents: [{ pane_id: "w1:p1", thread_capability: "pending" }] as never });
      command("desk_terminal");
      expect(useUiStore.getState().paneConversation).toBeNull();
      useShellStore.setState({ agents: [] });
      command("desk_terminal");
      expect(useUiStore.getState().paneConversation).toBeNull();
    } finally { stopKeyboard(); window.hideHost = previousHost; remove(); }
  });

  it("continues a delayed menu close after automatic focus return", async () => {
    const { remove, actions, content, land } = await setup();
    const { act, createElement } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { useReturnFocus } = await import("./components/ui/layer");
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let handback: (event: Event) => void = () => {};
    function Layer() { handback = useReturnFocus(true); return null; }
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return 1; });
    try {
      document.querySelector<HTMLElement>('[role="tab"]')!.focus();
      act(() => root.render(createElement(Layer)));
      const menu = document.createElement("button");
      menu.setAttribute("role", "menu");
      document.body.append(menu);
      menu.focus();
      actions.closeView("d1");
      act(() => root.unmount());
      menu.remove();
      handback(new Event("focusScopeAutoFocusOnUnmount", { cancelable: true }));
      expect(document.activeElement).toBe(document.querySelector('[role="tab"]'));
      land();
      frames.forEach((callback) => callback(0));
      expect(document.activeElement).toBe(content);
    } finally { remove(); }
  });
});

// PRD hide-external-control B44: a tool's cell stands over this window's own
// machine, which the window shows again once the cell goes (disconnect, leave
// or View only), whether the window is a node's or the Overview showed another device.
describe("selecting a tool's cell", () => {
  const TOOL = "c0ffee00c0ffee00";
  afterEach(() => {
    window.location.hash = "";
    useShellStore.setState({ rest: null });
    useUiStore.setState({ screen: null, overviewOpen: false });
  });
  // The tool whose cell is in front, read from what the center shows.
  const cellInFront = () => frontTool(useShellStore.getState().rest, useUiStore.getState().screen)?.connection ?? null;
  // `named` is the device a main page names, as a rail tile or a device's Overview opened as the page leaves it.
  const select = (hash: string, front: string, overview: string | null = null, named?: string) => {
    window.location.hash = hash;
    useShellStore.setState({ rest: { navigator: { focused_device_id: front, devices: [{ id: "core", kind: "local" }, { id: "node-window", kind: "remote" }, { id: "mini", kind: "remote" }], workspaces: [], tools: [{ connection: TOOL, name: "nightly", turn: 0, devices: [] }] }, status: { remote: [] } } as never });
    useUiStore.setState({ screen: { kind: "main", deviceId: named } } as never);
    useUiStore.getState().setOverview({ deviceId: overview });
    const sent: Parameters<DispatchFn>[0][] = [];
    createActions((event) => { sent.push(event); return true; }).focusTool(TOOL);
    return { focus: sent.filter((event) => event.kind === "focus_device").map((event) => (event.payload as { device_id: string }).device_id), overview: useUiStore.getState().overview.deviceId, cell: cellInFront() };
  };
  // The machine the Overview shows, as `useOverviewDevice` reads it, beside the one the rail and the sidebar show once no cell stands.
  const shown = () => {
    const { overview, screen } = useUiStore.getState();
    const now = useShellStore.getState().rest;
    return { overview: overviewDeviceOf(now, overview.deviceId, screen?.kind === "main" ? screen.deviceId : undefined), rail: frontDeviceId(now) };
  };
  // The core takes the focus the window asked for, then the tool leaves Act (a disconnect, a leave, View only) and its cell goes.
  const cellGoes = (focused: string) => {
    const now = useShellStore.getState().rest!;
    useShellStore.setState({ rest: { ...now, navigator: { ...now.navigator!, focused_device_id: focused, tools: [] } } });
  };

  it("brings this window's own machine to the front under the cell, never the core's from a node's window", () => {
    expect(select("#node=node-window", "core"), "a node's window").toEqual({ focus: ["node-window"], overview: null, cell: TOOL });
    expect(select("#node=node-window", "node-window"), "already in front").toEqual({ focus: [], overview: null, cell: TOOL });
    expect(select("", "mini", "mini"), "the core's window on a device's Overview").toEqual({ focus: ["core"], overview: null, cell: TOOL });
  });

  it("leaves the Overview on the machine the rail and the sidebar show once the cell goes, from a device's Overview page", () => {
    for (const [hash, own] of [["", "core"], ["#node=node-window", "node-window"]] as const) {
      expect(select(hash, "mini", null, "mini").focus, hash).toEqual([own]);
      cellGoes(own);
      expect(cellInFront(), hash).toBeNull();
      expect(useUiStore.getState().screen, hash).toEqual({ kind: "main" });
      expect(shown(), hash).toEqual({ overview: own, rail: own });
    }
  });

  it("names this window's own machine for the Overview under the cell, before the core answers the focus", () => {
    select("#node=node-window", "mini", null, "mini");
    expect(shown().overview).toBe("node-window");
  });

  it("drops a device the Overview named while the cell stood once the cell goes", () => {
    select("", "core");
    useUiStore.getState().setOverview({ deviceId: "mini" });
    cellGoes("core");
    expect(shown()).toEqual({ overview: "core", rail: "core" });
  });

  // PRD hide-external-control D-54, B47: the tile shows the tool's Overview as the center's page, never as a layer over
  // the Workspace or as This Mac's panes left in place; a device tile gives the center back.
  it("shows the tool's Overview as the center's page, from a Workspace and from its Overview layer, and a device tile takes the center back", () => {
    for (const layer of [false, true]) {
      select("", "core");
      useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: layer });
      createActions(() => true).focusTool(TOOL);
      const { screen, overviewOpen } = useUiStore.getState();
      expect({ screen: screen?.kind, overviewOpen, cell: cellInFront() }, `layer ${layer}`).toEqual({ screen: "main", overviewOpen: false, cell: TOOL });
    }
    // This Mac's Workspace, an operator's pane in front, is where the center goes back to.
    const now = useShellStore.getState().rest!;
    const checkout = { id: "c", workspace_id: "w", path: "/fixture", label: "fixture", tabs: [{ id: "t", panes: [{ id: "mine" }] }], agent_scope: emptyScope() };
    useShellStore.setState({ rest: { ...now, navigator: { ...now.navigator!, focused_checkout_id: "c", workspaces: [{ id: "w", device_id: "core", checkouts: [checkout] }] }, focused: { pane_id: "mine" }, workspace_view: { device_id: "core", path: "/fixture" } } as never });
    const sent: Parameters<DispatchFn>[0][] = [];
    createActions((event) => { sent.push(event); return true; }).focusDevice("core");
    expect({ screen: useUiStore.getState().screen?.kind, cell: cellInFront() }).toEqual({ screen: "workspace", cell: null });
    expect(sent).toEqual([]);
  });

  it("opens on the tool's Overview page its own session's record in its own panel, and another's on the device's Overview", () => {
    const actions = createActions(() => true);
    select("", "core");
    const now = useShellStore.getState().rest!;
    const cell = { connection: TOOL, name: "nightly", turn: 0, devices: [{ device_id: "core", tasks: { cards: [], needs_you: 0, recency: "" }, members: [{ pane_id: "tool-pane", project_id: "w", checkout_id: "c" }] }] };
    useShellStore.setState({ rest: { ...now, navigator: { ...now.navigator!, tools: [cell] } } as never, agents: [{ pane_id: "tool-pane", session_id: "s-tool" }, { pane_id: "mine", session_id: "s-mine" }] as never });
    actions.openSessionRecord({ device_id: "core", agent: "claude", session_id: "s-tool" });
    expect({ screen: useUiStore.getState().screen, panel: useUiStore.getState().overview.panel?.kind, cell: cellInFront() }).toEqual({ screen: { kind: "main", deviceId: "core", tool: TOOL }, panel: "record", cell: TOOL });
    actions.openSessionRecord({ device_id: "core", agent: "claude", session_id: "s-mine" });
    expect({ screen: useUiStore.getState().screen, cell: cellInFront() }).toEqual({ screen: { kind: "main", deviceId: "core" }, cell: null });
  });

  it("opens the 개요 and 미션 rows of a tool's cell on the tool's own pages, from the page and from a pane it keeps", () => {
    const actions = createActions(() => true);
    select("", "core");
    actions.openMissions();
    expect(useUiStore.getState().screen).toEqual({ kind: "missions", tool: TOOL });
    actions.toggleOverview();
    expect(useUiStore.getState().screen).toEqual({ kind: "main", tool: TOOL });
    // A device's tile from the tool's mission screen shows the device's missions.
    actions.openMissions();
    actions.focusDevice("core");
    expect({ screen: useUiStore.getState().screen, cell: cellInFront() }).toEqual({ screen: { kind: "missions" }, cell: null });
    actions.openMissions();
    expect(useUiStore.getState().screen).toEqual({ kind: "missions" });
  });
});

describe("a pane command pressed without pointing at a pane", () => {
  const checkout = {
    id: "c", workspace_id: "w", path: "/fixture", label: "fixture", active_tab_id: "t", agent_scope: emptyScope(),
    tabs: [{ id: "t", panes: [{ id: "left", cwd: "/fixture/left" }, { id: "right", cwd: "/fixture/right" }] }, { id: "behind", panes: [{ id: "elsewhere", cwd: "/fixture/elsewhere" }] }],
  };
  // The core still names the left pane: its answer to a click on another pane is on its way.
  const rest = { navigator: { focused_checkout_id: "c", focused_workspace_id: "w", workspaces: [{ id: "w", device_id: "local", checkouts: [checkout] }] }, focused: { pane_id: "left" } } as never;
  const pressed = (owner: KeyboardOwner) => {
    useShellStore.setState({ rest, connection: "live", focusedPaneId: "left" });
    noteKeyboardOwner(owner);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    actions.toggleZoom();
    actions.split("right");
    return sent.map((event) => [event.kind, event.kind === "create_pane" ? event.payload.cwd : event.payload.pane_id]);
  };
  afterEach(() => {
    noteKeyboardOwner({ kind: "none" });
    useShellStore.setState({ rest: null, focusedPaneId: null });
  });

  it("acts on the pane the operator clicked before the core's answer arrives", () => {
    expect(pressed({ kind: "pane", workspace: "c", paneId: "right" })).toEqual([["toggle_zoom", "right"], ["create_pane", "/fixture/right"]]);
  });

  it.each<KeyboardOwner>([{ kind: "none" }, { kind: "pane", workspace: "c", paneId: "elsewhere" }])("acts on the core's focused pane with the keyboard on no pane of the tab on screen ($kind)", (owner) => {
    expect(pressed(owner)).toEqual([["toggle_zoom", "left"], ["create_pane", "/fixture/left"]]);
  });
});

// The mirror of remote-look on this machine (docs/status-model.md, Hide owns the read axis): a look chosen here counts
// only while a window of this machine shows its own screen, so the window tells which one it shows. The action that
// changes it tells it after its own events, on the same socket, so the core never reads a look the action passes through.
describe("which machine's screen this window shows", () => {
  const mini = "mini-machine";
  const live = (remoteScreen: string | null) => {
    useShellStore.setState({
      connection: "live",
      rest: {
        navigator: {
          focused_device_id: "core",
          devices: [{ id: "core", kind: "local" }, { id: "ssh-box", kind: "remote" }],
          workspaces: [],
          tools: [],
          peers: [{ key: "k".repeat(64), device_id: mini, name: "mini", opens: "open" }],
        },
        status: { remote: [] },
      } as never,
    });
    useUiStore.setState({ screen: { kind: "main" }, remoteScreen } as never);
  };
  const told = (sent: Parameters<DispatchFn>[0][]) => sent.map((event) => event.kind === "window_screen" ? `screen:${(event.payload as { screen: string }).screen}` : event.kind);
  afterEach(() => {
    window.hideHost = undefined;
    useUiStore.setState({ remoteScreen: null });
  });

  it("brings the device a tile chose forward before it says this machine's screen shows again", () => {
    live(mini);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    actions.noteWindowScreen();
    actions.focusDevice("ssh-box");
    expect(told(sent)).toEqual(["screen:computer", "focus_device", "screen:own"]);
    expect(useUiStore.getState().remoteScreen).toBeNull();

    // The device already in front sends no focus, and the screen is told once.
    sent.length = 0;
    actions.showScreen(mini);
    actions.focusDevice("core");
    actions.focusDevice("core");
    expect(told(sent)).toEqual(["screen:computer", "screen:own"]);
  });

  it("says another computer's screen when its tile or its work shows it, and this machine's again from a tool's tile", () => {
    live(null);
    const go = vi.fn();
    window.hideHost = { kind: "electron", platform: "darwin", remoteScreens: { show: vi.fn(), go, onState: () => () => {}, onWorkResult: () => () => {} } } as never;
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    actions.noteWindowScreen();
    actions.selectComputer(mini);
    expect(useUiStore.getState().remoteScreen).toBe(mini);
    actions.focusTool("tool-connection");
    expect(actions.openComputerWork(mini, { op: "show", pane: "p" })).toBe("handed");
    expect(go).toHaveBeenCalledOnce();
    expect(told(sent)).toEqual(["screen:own", "screen:computer", "screen:own", "screen:computer"]);
  });

  it("tells after the action's own events, however the action ends", () => {
    live(mini);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    actions.noteWindowScreen();
    expect(() => actions.showScreen(null, () => {
      actions.openAgent("w1:p1");
      throw new Error("the open failed");
    })).toThrow("the open failed");
    expect(told(sent)).toEqual(["screen:computer", "focus_pane", "screen:own"]);
  });

  it("opens this machine's agent on this machine's own screen when it is asked for from another computer's", () => {
    live(mini);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    actions.noteWindowScreen();
    actions.openAgent("w1:p1");
    expect(useUiStore.getState().remoteScreen).toBeNull();
    actions.openAgent("w1:p1");
    expect(told(sent)).toEqual(["screen:computer", "focus_pane", "screen:own", "focus_pane"]);
  });

  it("tells a new connection first, then only what changes, and nothing while none is live", () => {
    live(mini);
    const sent: Parameters<DispatchFn>[0][] = [];
    const actions = createActions((event) => { sent.push(event); return true; });
    // Before the connection's first report a change is the report's to tell.
    actions.focusDevice("core");
    expect(told(sent)).toEqual([]);
    actions.showScreen(mini);
    actions.noteWindowScreen();
    actions.showScreen(mini);
    useShellStore.setState({ connection: "connecting" } as never);
    actions.noteWindowScreen();
    useShellStore.setState({ connection: "live" } as never);
    actions.showScreen(mini);
    actions.noteWindowScreen();
    expect(told(sent)).toEqual(["screen:computer", "screen:computer"]);
  });
});
