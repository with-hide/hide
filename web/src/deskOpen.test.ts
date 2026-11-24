import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createActions } from "./actions";
import { DESK_ROOM_CLOSED, activeTab, panelPlace, type DeskTab } from "./deskRoom";
import type { Checkout, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-2-ui D-05, D-22-D-24, B16, B19, B20, B35-B39:
// what a press in Desk opens goes to the tab panel, beside the session open
// beside Desk or at the window's right edge, and a file is the app's own file
// view of its checkout, which the core brings in front without leaving Desk.

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

beforeEach(() => {
  sent = [];
  useShellStore.setState({ rest, desk, agents: [] });
  useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, deskReturn: null });
});
afterEach(() => useUiStore.setState({ deskRoom: DESK_ROOM_CLOSED }));

describe("what Desk opens in its tab panel", () => {
  it("opens a file beside the open session, its checkout brought in front and the file placed by the core as the app's own file view", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen({ kind: "file", device_id: "local", root: "/repo", path: "/repo/web/token.ts" }, { preview: true });
    const room = useUiStore.getState().deskRoom;
    expect(panelPlace(room)).toBe("beside_session");
    expect(activeTab(room.panel)).toEqual({ kind: "file", device_id: "local", root: "/repo", path: "/repo/web/token.ts" });
    expect(kinds()).toEqual(["focus_checkout", "file_open"]);
    expect(sent[1]!.payload).toEqual({ path: "/repo/web/token.ts", workspace_id: "w-repo", checkout_id: "repo", preview: true });
    // Desk stays on screen.
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });
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
    // The front checkout needs no focus; only the file is opened.
    expect(kinds()).toEqual(["file_open"]);
  });

  it("opens the Explorer of the session's checkout in the tool column and closes it on a second press (B35)", () => {
    actions.openDeskSession("p-session");
    actions.deskToggleExplorer();
    expect(useUiStore.getState().deskRoom.panel?.explorer).toBe(true);
    expect(kinds()).toEqual(["focus_checkout"]);
    expect(sent[0]!.payload).toEqual({ workspace_id: "w-repo", checkout_id: "repo" });
    actions.deskToggleExplorer();
    expect(useUiStore.getState().deskRoom.panel).toBeNull();
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
    useUiStore.getState().setScreen({ kind: "workspace" });
    actions.openDesk();
    expect(useUiStore.getState().deskRoom).toEqual(DESK_ROOM_CLOSED);
  });
});
