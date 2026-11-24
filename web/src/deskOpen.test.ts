import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createActions } from "./actions";
import { watchDeskFiles } from "./deskFiles";
import { DESK_ROOM_CLOSED, activeTab, closeSide, closeTab, panelPlace, tabKey, type DeskTab } from "./deskRoom";
import type { Checkout, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-2-ui D-05, D-22-D-24, B16, B19, B20, B26,
// B35-B39: what a press in Desk opens goes to the tab panel, beside the
// session open beside Desk or at the window's right edge, and a file is its
// checkout's document hided holds for this window from the room's whole file
// set (`desk_files`), sent on each change and each connect, with no checkout
// brought in front.

const checkout = (id: string, path: string, pane: string): Checkout => ({
  id, workspace_id: `w-${id}`, path, label: id, branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: undefined as never, next_tab_label: "Tab 2",
  tabs: [{ id: `t-${id}`, workspace_id: `w-${id}`, checkout_id: id, label: "Tab 1", empty: false, delegated: false, panes: [{ id: pane, device_id: "local", device_pane_id: pane, cwd: path, engine_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: `t-${id}`, strip: [],
});
const workspace = (one: Checkout) => ({ id: one.workspace_id, label: one.label, device_id: "local", path: one.path, registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: undefined as never, checkouts: [one] });
const rest = {
  navigator: { focused_device_id: "local", focused_checkout_id: "front", focused_workspace_id: "w-front", devices: [{ id: "local", kind: "local" }], workspaces: [workspace(checkout("front", "/front", "p-front")), workspace(checkout("repo", "/repo", "p-session"))] },
  ui_state: {},
} as unknown as SnapshotRest;
const desk = { settings: { enabled: true } } as never;
const mission: DeskTab = { kind: "mission", mission: "m-39", title: "1.3.0" };
const pr: DeskTab = { kind: "output", output: { id: "pr:2911", kind: "pr", label: "#2911", thumb: null, seen: true, pr_state: "mergeable", device_id: null, checkout: null, path: null, url: "https://github.com/o/r/pull/2911" } };

let sent: Parameters<DispatchFn>[0][] = [];
const actions = createActions((event) => {
  sent.push(event);
  return true;
});
const kinds = () => sent.map((event) => event.kind);

// The App's watcher of the room (App.tsx), on from a closed room.
let unwatch = () => {};
beforeEach(() => {
  useShellStore.setState({ rest, desk, agents: [], connection: "live" });
  useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, deskReturn: null });
  unwatch = watchDeskFiles((files) => actions.deskFiles(files));
  sent = [];
});
afterEach(() => {
  unwatch();
  useUiStore.setState({ deskRoom: DESK_ROOM_CLOSED });
});
const file = (path: string): DeskTab => ({ kind: "file", device_id: "local", root: "/repo", path });
const held = (path: string) => ({ workspace_id: "w-repo", checkout_id: "repo", device_id: "local", path });
const sets = () => sent.map((event) => (event.payload as { files: unknown[] }).files);

describe("what Desk opens in its tab panel", () => {
  it("opens a file beside the open session as one desk_files set, bringing no checkout in front", () => {
    actions.openDeskSession("p-session");
    sent = [];
    actions.deskOpen(file("/repo/web/token.ts"), { preview: true });
    const room = useUiStore.getState().deskRoom;
    expect(panelPlace(room)).toBe("beside_session");
    expect(activeTab(room.panel)).toEqual(file("/repo/web/token.ts"));
    expect(sent.map((event) => [event.kind, event.payload])).toEqual([["desk_files", { files: [held("/repo/web/token.ts")] }]]);
    // Desk stays on screen.
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
  });

  it("replaces the preview file in one set without the replaced file, and lets a closed tab's file go (B36)", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(file("/repo/a.ts"), { preview: true });
    sent = [];
    actions.deskOpen(file("/repo/b.ts"), { preview: true });
    expect(sets()).toEqual([[held("/repo/b.ts")]]);
    sent = [];
    // Bringing an open tab to the front holds nothing new.
    actions.deskOpen(file("/repo/b.ts"));
    expect(sent).toEqual([]);
    useUiStore.getState().setDeskRoom((now) => closeTab(now, tabKey(file("/repo/b.ts"))));
    expect(sets()).toEqual([[]]);
  });

  it("opens a second file view beside the one in front with ⌘↵, the same file with no event, and a side's ✕ closes only that view (D-23, B37)", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(file("/repo/a.ts"));
    sent = [];
    actions.deskOpen(file("/repo/a.ts"), { beside: true });
    let panel = useUiStore.getState().deskRoom.panel!;
    expect([panel.active, panel.beside]).toEqual([tabKey(file("/repo/a.ts")), tabKey(file("/repo/a.ts"))]);
    expect(sent).toEqual([]);
    useUiStore.getState().setDeskRoom((now) => closeSide(now, "beside"));
    expect(useUiStore.getState().deskRoom.panel?.beside).toBeNull();
    actions.deskOpen(file("/repo/b.ts"), { beside: true });
    panel = useUiStore.getState().deskRoom.panel!;
    expect([panel.active, panel.beside]).toEqual([tabKey(file("/repo/a.ts")), tabKey(file("/repo/b.ts"))]);
    expect(sets()).toEqual([[held("/repo/a.ts"), held("/repo/b.ts")]]);
    sent = [];
    useUiStore.getState().setDeskRoom((now) => closeSide(now, "front"));
    panel = useUiStore.getState().deskRoom.panel!;
    expect([panel.active, panel.beside, panel.tabs.length]).toEqual([tabKey(file("/repo/b.ts")), null, 2]);
    expect(sent).toEqual([]);
  });

  it("opens a file, a PR or a mission at the window's right edge when no session is open, and opens no session for it", () => {
    actions.deskOpen({ kind: "file", device_id: "local", root: "/front", path: "/front/a.ts" });
    actions.deskOpen(pr);
    actions.deskOpen(mission);
    const room = useUiStore.getState().deskRoom;
    expect(room.session).toBeNull();
    expect(panelPlace(room)).toBe("window_edge");
    expect(room.panel?.tabs).toHaveLength(3);
    expect(activeTab(room.panel)).toEqual(mission);
    expect(kinds()).toEqual(["desk_files"]);
  });

  it("opens the Explorer of the session's checkout in the tool column with no event and closes it on a second press (B35)", () => {
    actions.openDeskSession("p-session");
    sent = [];
    actions.deskToggleExplorer();
    expect(useUiStore.getState().deskRoom.panel?.explorer).toBe(true);
    expect(sent).toEqual([]);
    actions.deskToggleExplorer();
    expect(useUiStore.getState().deskRoom.panel).toBeNull();
  });

  it("lets every file of the room go in one set however the screen leaves Desk (B26)", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(file("/repo/a.ts"));
    actions.deskOpen(file("/repo/b.ts"));
    sent = [];
    // Not through `setScreen`: a direct screen change leaves Desk too.
    useUiStore.setState({ screen: { kind: "workspace" } });
    expect(useUiStore.getState().deskRoom).toEqual(DESK_ROOM_CLOSED);
    expect(sets()).toEqual([[]]);
  });

  it("sends the room's files again each time the connection comes live, once the catalog names their checkouts", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(file("/repo/a.ts"));
    sent = [];
    // A new connection holds nothing: a reload, a quit or a dropped socket let it go.
    useShellStore.setState({ connection: "reconnecting", rest: null });
    useShellStore.setState({ connection: "live" });
    expect(sent).toEqual([]);
    useShellStore.setState({ rest });
    expect(sets()).toEqual([[held("/repo/a.ts")]]);
    sent = [];
    // A file whose checkout leaves the catalog is let go, and held again once it is back.
    const listed = rest.navigator!.workspaces!;
    useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator!, workspaces: listed.slice(0, 1) } } as SnapshotRest });
    useShellStore.setState({ rest });
    expect(sets()).toEqual([[], [held("/repo/a.ts")]]);
  });

  it("starts the room closed the next time it is entered (B26), and ⌘J brings back the session it left for (B7)", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(mission);
    actions.deskTerminal();
    // The Agent tab comes in front (openAgent's own work); leaving Desk closes its right side.
    useUiStore.getState().setScreen({ kind: "workspace" });
    expect(useUiStore.getState().deskRoom).toEqual(DESK_ROOM_CLOSED);
    expect(useUiStore.getState().deskReturn).toBe("p-session");
    useShellStore.setState({ focusedPaneId: "p-session" });
    actions.deskTerminal();
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
    expect(useUiStore.getState().deskRoom.session).toBe("p-session");
    // The round trip is spent: the pane no longer goes back to Desk with ⌘J (review m2).
    expect(useUiStore.getState().deskReturn).toBeNull();
    useUiStore.getState().setScreen({ kind: "workspace" });
    actions.openDesk();
    expect(useUiStore.getState().deskRoom).toEqual(DESK_ROOM_CLOSED);
  });

  it("opens a sidebar row's session beside Desk while Desk is in front, and its tab on a ⌘-click or from any other caller (B3, B5, review O1)", () => {
    actions.openSidebarRow("p-session");
    expect(useUiStore.getState().deskRoom.session).toBe("p-session");
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
    // ⌥ number, a notification or another computer open the agent's tab, never beside Desk.
    actions.deskCloseSession();
    actions.openListedAgent("p-session");
    expect(useUiStore.getState().deskRoom.session).toBeNull();
    expect(actions.rowOpensBesideDesk({ tab: true })).toBe(false);
    useUiStore.getState().setScreen({ kind: "workspace" });
    expect(actions.rowOpensBesideDesk()).toBe(false);
  });
});
