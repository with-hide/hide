// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { DESK_ROOM_CLOSED, activeTab } from "./deskRoom";
import { clientI18n } from "./i18n/translator";
import { Palette } from "./Palette";
import type { Checkout, DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-2-ui D-23, B37, B38: ⌘P on Desk searches the
// session's checkout or every project, ↵ opens the file in the tab panel as a
// preview, ⌘↵ keeps it, and the Explorer column is left as it was.

Element.prototype.scrollIntoView = () => {};

const checkout = (id: string, path: string, pane: string): Checkout => ({
  id, workspace_id: `w-${id}`, path, label: id, branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: undefined as never, next_tab_label: "Tab 2",
  tabs: [{ id: `t-${id}`, workspace_id: `w-${id}`, checkout_id: id, label: "Tab 1", empty: false, delegated: false, panes: [{ id: pane, device_id: "local", device_pane_id: pane, cwd: path, engine_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: `t-${id}`, strip: [],
});
const workspace = (one: Checkout, label: string) => ({ id: one.workspace_id, label, device_id: "local", path: one.path, registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: undefined as never, checkouts: [one] });
const rest = {
  navigator: { focused_device_id: "local", focused_checkout_id: "ide", focused_workspace_id: "w-ide", devices: [{ id: "local", kind: "local" }], workspaces: [workspace(checkout("ide", "/ide", "p-ide"), "herdr-ide"), workspace(checkout("atorni", "/atorni", "p-atorni"), "atorni-ax")] },
  ui_state: {},
} as unknown as SnapshotRest;
const file = (root: string, relative: string) => ({ device_id: "local", root_path: root, path: `${root}/${relative}`, relative_path: relative });
const desk = {
  settings: { enabled: true },
  conversation: { generation: 1, dropped: 0, items: [{ kind: "user", id: 1, text: "", at_ms: 1, mentions: [{ kind: "project", id: "/ide", name: "herdr-ide" }] }] },
} as unknown as DeskSection;

describe("⌘P on Desk", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let sent: Parameters<DispatchFn>[0][];

  beforeEach(async () => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    sent = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("en"); });
    useShellStore.setState({
      rest, desk, agents: [],
      fileIndexAll: { query: "", files: [file("/atorni", "src/token-guard.ts"), file("/ide", "web/src/token.ts")], truncated: false, indexing: false, unavailable: [] },
    });
    useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, overlay: "file_palette" });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
  });

  const open = async () => {
    const actions = createActions((event) => { sent.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><Palette actions={actions} /></TooltipProvider>));
  };
  const press = async (init: KeyboardEventInit) => act(async () => {
    document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...init }));
  });
  const rows = () => [...document.querySelectorAll("[data-palette-row]")].map((row) => row.textContent?.replace(/\s+/gu, " ").replace(/↵$/u, "").trim());

  it("with no session open searches every project, the project Desk named last first, each result led by its project", async () => {
    await open();
    expect(rows()).toEqual(["herdr-ide › token.tsweb/src", "atorni-ax › token-guard.tssrc"]);
    expect([...document.querySelectorAll("[data-palette-group]")].map((group) => group.getAttribute("data-palette-group"))).toEqual(["named", "other"]);
  });

  it("opens the highlighted file in the tab panel as the preview on ↵ and to the side on ⌘↵, leaving the Explorer column alone", async () => {
    await open();
    await press({});
    let room = useUiStore.getState().deskRoom;
    expect(activeTab(room.panel)).toEqual({ kind: "file", device_id: "local", root: "/ide", path: "/ide/web/src/token.ts" });
    expect(room.panel?.preview).toBe("file:local:/ide:/ide/web/src/token.ts");
    expect(room.panel?.explorer).toBe(false);
    expect(sent.at(-1)?.payload).toMatchObject({ path: "/ide/web/src/token.ts", preview: true });
    expect(useUiStore.getState().overlay).toBe("none");

    useUiStore.setState({ overlay: "file_palette" });
    await open();
    await press({ metaKey: true, ctrlKey: true });
    room = useUiStore.getState().deskRoom;
    // The same file again, to the side as the app's Open to the side asks the core for it: a pinned second view.
    expect(room.panel?.tabs).toHaveLength(1);
    expect(room.panel?.preview).toBeNull();
    expect(sent.at(-1)?.payload).toMatchObject({ path: "/ide/web/src/token.ts", preview: false, beside: true });
  });

  it("searches only the checkout of the session open beside Desk", async () => {
    useUiStore.setState({ deskRoom: { ...DESK_ROOM_CLOSED, session: "p-atorni" } });
    await open();
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 150)); });
    expect(sent.map((event) => event.kind)).toEqual(["file_index"]);
    expect(sent[0]!.payload).toMatchObject({ root: "/atorni" });
  });

  it("is the app's own palette on every other screen", async () => {
    useUiStore.setState({ screen: { kind: "workspace" } });
    await open();
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 150)); });
    // The front checkout is asked, not every project.
    expect(sent.map((event) => event.kind)).toEqual(["file_index"]);
    expect(sent[0]!.payload).toMatchObject({ root: "/ide" });
  });
});
