// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { DESK_ROOM_CLOSED, activeTab, openTab, type DeskRoomWindow } from "./deskRoom";
import { clientI18n } from "./i18n/translator";
import { Palette } from "./Palette";
import type { Checkout, DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-2-ui D-23, B37, B38: ⌘P on Desk searches the
// session's checkout or every project, ↵ opens the file in the tab panel as a
// preview, ⌘↵ (Ctrl+Enter off macOS) opens it to the side in the panel, and the Explorer
// column is left as it was.

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

/** A room whose tab panel already shows a file, so ⌘↵ has one to open the second view beside. */
const showing: DeskRoomWindow = openTab(DESK_ROOM_CLOSED, { kind: "file", device_id: "local", root: "/atorni", path: "/atorni/src/token-guard.ts" });

describe("⌘P on Desk", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let sent: Parameters<DispatchFn>[0][];
  const all = (files: ReturnType<typeof file>[], extra: Partial<NonNullable<ReturnType<typeof useShellStore.getState>["fileIndexAll"]>> = {}) =>
    ({ query: "", files, more: false, index_truncated: false, indexing: false, unavailable: [], ...extra });

  beforeEach(async () => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.useFakeTimers();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    sent = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("en"); });
    useShellStore.setState({ rest, desk, agents: [], fileIndexAll: null });
    useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, overlay: "file_palette" });
    system("MacIntel");
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  /** Opens the palette and lets the debounced request go out, as the core would then answer it with `answer`. */
  const open = async (answer?: ReturnType<typeof all>) => {
    const actions = createActions((event) => { sent.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><Palette actions={actions} /></TooltipProvider>));
    await act(async () => { vi.advanceTimersByTime(150); });
    if (answer) await act(async () => useShellStore.setState({ fileIndexAll: answer }));
  };
  /** The system the keys are read as: ⌘ on macOS, Ctrl elsewhere (`keySystem`). */
  const system = (platform: "MacIntel" | "Win32") => Object.defineProperty(navigator, "platform", { value: platform, configurable: true });
  const press = async (init: KeyboardEventInit) => act(async () => {
    document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...init }));
  });
  const rows = () => [...document.querySelectorAll("[data-palette-row]")].map((row) => row.textContent?.replace(/\s+/gu, " ").replace(/↵$/u, "").trim());
  const footer = () => document.querySelector("[data-palette-footer]")?.textContent ?? "";
  const two = () => all([file("/atorni", "src/token-guard.ts"), file("/ide", "web/src/token.ts")]);

  it("with no session open searches every project, the project Desk named last first, each result led by its project", async () => {
    await open(two());
    expect(sent.map((event) => event.kind)).toEqual(["file_index_all"]);
    // The project Desk named last is asked for first.
    expect(sent[0]!.payload).toMatchObject({ prefer: { device_id: "local", root_path: "/ide" } });
    expect(rows()).toEqual(["herdr-ide › token.tsweb/src", "atorni-ax › token-guard.tssrc"]);
    expect([...document.querySelectorAll("[data-palette-group]")].map((group) => group.getAttribute("data-palette-group"))).toEqual(["named", "other"]);
  });

  it("opens the highlighted file in the tab panel as the preview on ↵ and to the side on ⌘↵, leaving the Explorer column alone", async () => {
    await open(two());
    await press({});
    let room = useUiStore.getState().deskRoom;
    expect(activeTab(room.panel)).toEqual({ kind: "file", device_id: "local", root: "/ide", path: "/ide/web/src/token.ts" });
    expect(room.panel?.preview).toBe("file:local:/ide:/ide/web/src/token.ts");
    expect(room.panel?.explorer).toBe(false);
    expect(useUiStore.getState().overlay).toBe("none");

    useUiStore.setState({ overlay: "file_palette" });
    await open(two());
    await press({ metaKey: true });
    room = useUiStore.getState().deskRoom;
    // The same file again, as a second view the window holds: the file in front stays and no Workspace's layout gets a view.
    expect(room.panel?.beside).toBe("file:local:/ide:/ide/web/src/token.ts");
    expect(room.panel?.preview).toBeNull();
    expect(sent.some((event) => event.kind === "file_open")).toBe(false);
  });

  it("opens a file to the side on the system's own command key: ⌘ on macOS, Ctrl elsewhere, and the other key alone is a plain ↵", async () => {
    const sides = () => useUiStore.getState().deskRoom.panel?.beside != null;
    for (const [platform, held, other] of [["MacIntel", { metaKey: true }, { ctrlKey: true }], ["Win32", { ctrlKey: true }, { metaKey: true }]] as const) {
      system(platform);
      for (const [init, beside] of [[held, true], [other, false]] as const) {
        useUiStore.setState({ overlay: "file_palette", deskRoom: showing });
        await open(two());
        await press(init);
        expect(sides()).toBe(beside);
        await act(async () => root.unmount());
        root = createRoot(container);
      }
    }
  });

  it("tells a project whose files could not be listed, and a cut list, in the footer", async () => {
    await open(all([file("/ide", "web/src/token.ts")], { more: true, unavailable: [{ device_id: "away", root_path: "/x", code: "unavailable" }] }));
    expect(footer()).toContain("Showing the first 80 matches");
    expect(footer()).toContain("1 projects could not be listed");
    expect(document.querySelector('[data-palette-state="empty"]')).toBeNull();
  });

  it("opens the file of the highlighted row when two devices hold the same path", async () => {
    useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote" }] }, status: { remote: [{ target_id: "mini", session: { workspaces: [{ ...workspace(checkout("ide2", "/ide", "p-mini"), "herdr-ide"), device_id: "mini" }] } }] } } as unknown as SnapshotRest });
    await open(all([file("/ide", "a.ts"), { ...file("/ide", "a.ts"), device_id: "mini" }]));
    // Move the highlight to the second row, which is the device's.
    await act(async () => document.querySelector("[data-palette-input]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
    await press({});
    expect(activeTab(useUiStore.getState().deskRoom.panel)).toMatchObject({ device_id: "mini", path: "/ide/a.ts" });
  });

  it("does not list an answer left from an earlier open", async () => {
    useShellStore.setState({ fileIndexAll: two() });
    await open();
    expect(rows()).toEqual([]);
    await act(async () => useShellStore.setState({ fileIndexAll: all([file("/ide", "x.ts")]) }));
    expect(rows()).toHaveLength(1);
  });

  it("keeps the app's own palette showing the checkout's cached list when it is opened again off Desk", async () => {
    useUiStore.setState({ screen: { kind: "workspace" } });
    useShellStore.setState({ fileIndex: { device_id: "local", root_path: "/ide", query: "", files: [{ path: "/ide/a.ts", relative_path: "a.ts" }], truncated: false, indexing: false, unavailable: null } });
    await open();
    expect(rows()).toEqual(["a.ts"]);
  });

  it("searches only the checkout of the session open beside Desk", async () => {
    useUiStore.setState({ deskRoom: { ...DESK_ROOM_CLOSED, session: "p-atorni" } });
    await open();
    expect(sent.map((event) => event.kind)).toEqual(["file_index"]);
    expect(sent[0]!.payload).toMatchObject({ root: "/atorni" });
  });

  it("is the app's own palette on every other screen", async () => {
    useUiStore.setState({ screen: { kind: "workspace" } });
    await open();
    // The front checkout is asked, not every project.
    expect(sent.map((event) => event.kind)).toEqual(["file_index"]);
    expect(sent[0]!.payload).toMatchObject({ root: "/ide" });
  });
});
