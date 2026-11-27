// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createActions } from "./actions";
import { deskOpenSend, type DeskOpen } from "./deskDesk";
import { DESK_ROOM_CLOSED, deskShown, panelPlace } from "./deskRoom";
import type { Checkout, DeskOutput, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";
import type { DispatchFn } from "./ws";

// Expected answers are PRD desk-own-desk D-03, D-12, D-15, B10, B11, B14,
// B16, B17, B34, B37-B39 and PRD desk-2-ui B7, B26: what a press in Desk
// opens is one `desk_view_open`, which the core puts in front of Desk's own
// desk, and the desk shows, beside the session open beside Desk or at the
// window's right edge, with no checkout brought in front.

const checkout = (id: string, path: string, pane: string): Checkout => ({
  id, workspace_id: `w-${id}`, path, label: id, branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: undefined as never, next_tab_label: "Tab 2",
  tabs: [{ id: `t-${id}`, workspace_id: `w-${id}`, checkout_id: id, label: "Tab 1", empty: false, delegated: false, panes: [{ id: pane, device_id: "local", device_pane_id: pane, cwd: path, engine_label: null, terminal_title: null, identity_label: null, status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false }] }], active_tab_id: `t-${id}`, strip: [],
});
const workspace = (one: Checkout, pulls: { number: number; url: string }[] = []) => ({ id: one.workspace_id, label: one.label, device_id: "local", path: one.path, registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: undefined as never, checkouts: [one], pull_requests: pulls });
const layout = { root: { area: { id: "a1", active: "d1", displays: [{ id: "d1" }] } }, active_area: "a1", display_count: 1, limits: { areas: 6, depth: 3, displays: 32 } };
const rest = {
  navigator: { focused_device_id: "local", focused_checkout_id: "front", focused_workspace_id: "w-front", devices: [{ id: "local", kind: "local" }], workspaces: [workspace(checkout("front", "/front", "p-front")), workspace(checkout("repo", "/repo", "p-session"), [{ number: 2911, url: "https://github.com/o/r/pull/2911" }])] },
  ui_state: {},
  desk_view: { device_id: "local", path: "hide:desk", layout, opened: 0 },
} as unknown as SnapshotRest;
const desk = { settings: { enabled: true } } as never;
const mission: DeskOpen = { kind: "card", card: { kind: "mission", mission: "m-39" } };
const output = (fields: Partial<DeskOutput>): DeskOutput => ({ id: "d4", kind: "doc", label: "plan.md", thumb: null, seen: false, pr_state: null, device_id: "local", checkout: "/repo", path: "/repo/plan.md", url: null, ...fields });

let sent: Parameters<DispatchFn>[0][] = [];
const actions = createActions((event) => {
  sent.push(event);
  return true;
});
const opens = () => sent.filter((event) => event.kind === "desk_view_open").map((event) => event.payload);
const shown = () => deskShown(useUiStore.getState().deskRoom, 1);

beforeEach(() => {
  useShellStore.setState({ rest, desk, agents: [], connection: "live", deskJobs: null });
  useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, deskReturn: null, paneConversation: null });
  sent = [];
});
afterEach(() => {
  useUiStore.setState({ deskRoom: DESK_ROOM_CLOSED, paneConversation: null });
});
const file = (path: string): DeskOpen => ({ kind: "file", device_id: "local", root: "/repo", path });

describe("what Desk opens on its own desk", () => {
  it("opens a file of a checkout beside the open session as one event naming that checkout, bringing no checkout in front (B10, B37, B38)", () => {
    actions.openDeskSession("p-session");
    sent = [];
    actions.deskOpen(file("/repo/web/token.ts"), { preview: true });
    actions.deskOpen(file("/repo/web/a.ts"), { beside: true });
    expect(opens()).toEqual([
      { kind: "file", workspace_id: "w-repo", checkout_id: "repo", device_id: "local", path: "/repo/web/token.ts", preview: true, beside: false },
      { kind: "file", workspace_id: "w-repo", checkout_id: "repo", device_id: "local", path: "/repo/web/a.ts", preview: false, beside: true },
    ]);
    expect(sent.map((event) => event.kind)).toEqual(["desk_view_open", "desk_view_open"]);
    expect([shown(), panelPlace(useUiStore.getState().deskRoom)]).toEqual([true, "beside_session"]);
  });

  it("opens a mission or a pull request as its card at the window's right edge when no session is open, and opens no session for it (B10, B39)", () => {
    actions.deskOpen(mission);
    actions.deskOpen({ kind: "output", output: output({ id: "pr:2911", kind: "pr", label: "#2911", pr_state: "mergeable", device_id: null, checkout: null, path: null, url: "https://github.com/o/r/pull/2911" }) });
    expect(opens()).toEqual([{ kind: "card", card: { kind: "mission", mission: "m-39" } }, { kind: "card", card: { kind: "pull_request", workspace_id: "w-repo", number: 2911 } }]);
    expect(useUiStore.getState().deskRoom.session).toBeNull();
    expect([shown(), panelPlace(useUiStore.getState().deskRoom)]).toEqual([true, "window_edge"]);
  });

  it("opens a View a session listed through the core, which marks it seen, and a file an answer named by its checkout (D-12, B11, B25)", () => {
    const listed = output({});
    useShellStore.setState({ deskJobs: { jobs: [{ id: "j1", outputs: [listed] }], workers: [], named: [], records: [], moved: [] } as never });
    actions.deskOpen({ kind: "output", output: listed });
    actions.deskOpen({ kind: "output", output: output({ id: "answer:1", device_id: null, checkout: null, path: "/repo/docs/x.md" }) });
    expect(opens()).toEqual([
      { kind: "output", device_id: "local", checkout: "/repo", id: "d4", request_id: expect.stringMatching(/^follow:/) },
      { kind: "file", workspace_id: "w-repo", checkout_id: "repo", device_id: "local", path: "/repo/docs/x.md", preview: false, beside: false },
    ]);
  });

  // Operator request 2026-11-27 (mission-109): the core places a session's View on Desk's desk under a display id the
  // window cannot know ahead, so the open names itself and the keyboard follows the display its receipt names.
  it("names a session's View it opens, and the keyboard waits for the display the core's receipt names on Desk's desk", () => {
    const listed = output({});
    useShellStore.setState({ deskJobs: { jobs: [{ id: "j1", outputs: [listed] }], workers: [], named: [], records: [], moved: [] } as never });
    actions.deskOpen({ kind: "output", output: listed });
    const request = opens()[0]!.request_id as string;
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: workspaceKey({ device_id: "local", path: "hide:desk" }), browserOpen: request });
  });

  it("opens nothing, and says so only in the log, for a file outside every checkout or while Desk is off", () => {
    actions.deskOpen({ kind: "file", device_id: "local", root: "/elsewhere", path: "/elsewhere/x.md" });
    expect(deskOpenSend(rest, { kind: "file", device_id: "local", root: "/elsewhere", path: "/elsewhere/x.md" }, () => false)).toEqual({ refused: "desk file: its checkout is not registered" });
    useShellStore.setState({ desk: { settings: { enabled: false } } as never });
    actions.deskOpen(file("/repo/a.ts"));
    expect(opens()).toEqual([]);
    expect(shown()).toBe(false);
  });

  it("opens the Explorer of the session's checkout in the desk's tool column with no event and closes it on a second press (B37)", () => {
    actions.openDeskSession("p-session");
    sent = [];
    actions.deskToggleExplorer();
    expect([useUiStore.getState().deskRoom.desk, useUiStore.getState().deskRoom.explorer]).toEqual([true, true]);
    actions.deskToggleExplorer();
    expect(useUiStore.getState().deskRoom.explorer).toBe(false);
    expect(sent).toEqual([]);
  });

  it("keeps the desk shown through a session change, and hides it with its tabs kept (B13, B18)", () => {
    actions.deskOpen(mission);
    expect(shown()).toBe(true);
    actions.openDeskSession("p-session");
    expect(shown()).toBe(true);
    actions.deskHideDesk();
    expect(shown()).toBe(false);
    // Opening anything again shows the desk with every tab it holds (B9).
    actions.deskOpen(mission);
    expect(shown()).toBe(true);
  });

  it("folds the desk and keeps its session while ⌘J toggles the pane it left for in place (B26, pane conversation B10)", () => {
    actions.openDeskSession("p-session");
    actions.deskOpen(mission);
    actions.deskTerminal();
    // The Agent tab comes in front (openAgent's own work); leaving Desk folds its desk and keeps the session.
    useUiStore.getState().setScreen({ kind: "workspace" });
    expect(useUiStore.getState().deskRoom).toEqual({ ...DESK_ROOM_CLOSED, session: "p-session" });
    expect(useUiStore.getState().deskReturn).toBe("p-session");
    // Pane conversation D-04/B10 replaces the old Desk round trip: the
    // same chord switches this pane without reopening Desk's right side.
    useShellStore.setState({ focusedPaneId: "p-session", agents: [{ pane_id: "p-session", thread_capability: "ready", thread_identity: "session-one" }] as never });
    actions.deskTerminal();
    expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });
    expect(useUiStore.getState().deskRoom).toEqual({ ...DESK_ROOM_CLOSED, session: "p-session" });
    expect(useUiStore.getState().paneConversation).toEqual({ paneId: "p-session", session: "session-one" });
    actions.deskTerminal();
    expect(useUiStore.getState().paneConversation).toBeNull();
    actions.openDesk();
    expect(useUiStore.getState().deskRoom).toEqual({ ...DESK_ROOM_CLOSED, session: "p-session" });
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
