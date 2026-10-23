// @vitest-environment jsdom
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { clearProbeCache } from "./terminalLinkProvider";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";
import type { DispatchFn } from "./ws";
import { FindBar } from "./Overlays";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Checkout, SnapshotRest } from "./snapshot";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { WorkspaceScreen } from "./WorkspaceScreen";
import { attachTerminal, terminalFor } from "./terminals";
import { installKeyboardOwner, keyboardCommandOwner, noteCommandDelivered } from "./viewFocus";
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
    tabs: [{ id: "t", workspace_id: "w", checkout_id: "c", label: "Tab 1", empty: false, delegated: false, panes: [{ id: "p", cwd: "/repo", herdr_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: "t", strip: [],
  };
  const rest: SnapshotRest = {
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", agent_scope: scope, devices: [], workspaces: [{ id: "w", label: "repo", device_id: "local", path: "/repo", registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: scope, checkouts: [checkout] }] },
    workspace_view: { ...workspace, open: true, documents: true, tool: folded ? "explorer" : null, pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", layout: { active_area: "a1", limits: { areas: 8, depth: 4, displays: 32 }, display_count: 1, root: { area: { id: "a1", active: "b", displays: [{ id: "b", tab_id: null, path: "", committed: null, preview: false, reason: null, kind: "browser", state: "open", label: "page", url: "https://page.test/" }] } } } },
    ui_state: { detail: { items: [], shown: false } },
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
    await act(async () => useShellStore.setState({ rest: { ...rest, workspace_view: { ...rest.workspace_view!, ...(folded ? { documents: false } : { open: false }) } } }));
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

describe("input after a confirmed View close", () => {
  const workspace = { device_id: "local", path: "/repo" };
  const frame = (ids: string[], selected: string) => ({ ...workspace, open: true, documents: true, tool: "explorer", layout: { active_area: "a1", display_count: ids.length, root: { area: { id: "a1", active: selected, displays: ids.map((id) => ({ id, kind: "browser", state: "open", label: id, url: "", tab_id: null })) } } } });
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
    useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "docked", view: "documents", tool: null, stacked: false } });
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
    Object.assign(window, { hideHost: { kind: "electron", platform: "darwin", onCommand: (listener: typeof command) => { command = listener; return () => {}; }, reportBindings: vi.fn(), reportLanguage: vi.fn() } });
    useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "docked", view: "documents", tool: null, stacked: false } });
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
