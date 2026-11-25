import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createActions } from "./actions";
import { watchDeskFiles } from "./deskFiles";
import { watchDeskOpens } from "./deskOpens";
import { DESK_ROOM_CLOSED, activeTab, type DeskTab } from "./deskRoom";
import type { Checkout, DeskOpen, DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// Expected answers are docs/ARCHITECTURE.md and docs/delivery.md, Desk: a file
// Desk's agent opens with `hide file open` waits in the `desk` section's
// `opens`; a window that shows Desk opens it in the tab panel as a click
// would, holds it with the room's `desk_files` set and sends
// `desk_open_taken`; a window on another screen takes nothing, so Desk never
// pulls the operator away.

const checkout: Checkout = {
  id: "repo", workspace_id: "w-repo", path: "/repo", label: "repo", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, agent_scope: undefined as never, next_tab_label: "Tab 2",
  tabs: [], active_tab_id: null, strip: [],
} as unknown as Checkout;
const rest = {
  navigator: { focused_device_id: "local", focused_checkout_id: "repo", focused_workspace_id: "w-repo", devices: [{ id: "local", kind: "local" }], workspaces: [{ id: "w-repo", label: "repo", device_id: "local", path: "/repo", registered: true, temporary: false, pinned: false, inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: undefined as never, checkouts: [checkout] }] },
  ui_state: {},
} as unknown as SnapshotRest;
const deskWith = (opens: DeskOpen[]) => ({ settings: { enabled: true }, opens }) as unknown as DeskSection;
const plan: DeskOpen = { id: 1, device_id: "local", root: "/repo", path: "/repo/plan.md" };
const planTab: DeskTab = { kind: "file", device_id: "local", root: "/repo", path: "/repo/plan.md" };

let sent: Parameters<DispatchFn>[0][] = [];
const actions = createActions((event) => {
  sent.push(event);
  return true;
});
const events = () => sent.map((event) => [event.kind, event.payload]);

// The App's two watchers (App.tsx).
let unwatch: (() => void)[] = [];
beforeEach(() => {
  useShellStore.setState({ rest, desk: deskWith([]), agents: [], connection: "live" });
  useUiStore.setState({ screen: { kind: "desk" }, deskRoom: DESK_ROOM_CLOSED, deskReturn: null });
  unwatch = [
    watchDeskFiles((files) => actions.deskFiles(files)),
    watchDeskOpens((entry) => actions.deskOpen({ kind: "file", device_id: entry.device_id, root: entry.root, path: entry.path }), (id) => actions.deskOpenTaken(id)),
  ];
  sent = [];
});
afterEach(() => {
  for (const stop of unwatch) stop();
  useUiStore.setState({ deskRoom: DESK_ROOM_CLOSED });
});

describe("a file Desk's agent opens", () => {
  it("opens in the tab panel of a window showing Desk, is held, and is taken once", () => {
    useShellStore.setState({ desk: deskWith([plan]) });

    expect(activeTab(useUiStore.getState().deskRoom.panel)).toEqual(planTab);
    expect(events()).toEqual([
      ["desk_files", { files: [{ workspace_id: "w-repo", checkout_id: "repo", device_id: "local", path: "/repo/plan.md" }] }],
      ["desk_open_taken", { id: 1 }],
    ]);

    // The core still lists it until the taken event lands: nothing is sent again.
    sent = [];
    useShellStore.setState({ desk: deskWith([plan]) });
    useShellStore.setState({ desk: deskWith([]) });
    expect(sent).toEqual([]);
  });

  it("waits while the window shows another screen, and opens when Desk comes on screen", () => {
    useUiStore.setState({ screen: { kind: "workspace" } });
    useShellStore.setState({ desk: deskWith([plan]) });
    expect(sent).toEqual([]);
    expect(useUiStore.getState().screen).toEqual({ kind: "workspace" });

    useUiStore.setState({ screen: { kind: "desk" } });
    expect(activeTab(useUiStore.getState().deskRoom.panel)).toEqual(planTab);
    expect(events().at(-1)).toEqual(["desk_open_taken", { id: 1 }]);
  });

  it("takes nothing while the connection is down, and starts over on the next one", () => {
    useShellStore.setState({ desk: deskWith([plan]) });
    sent = [];
    useShellStore.setState({ connection: "reconnecting" as never });
    useShellStore.setState({ desk: deskWith([plan]) });
    expect(sent).toEqual([]);

    // A restarted core numbers its opens again: id 1 is a new open.
    const next: DeskOpen = { ...plan, path: "/repo/next.md" };
    useShellStore.setState({ connection: "live", desk: deskWith([next]) });
    expect(events().at(-1)).toEqual(["desk_open_taken", { id: 1 }]);
    expect(activeTab(useUiStore.getState().deskRoom.panel)).toEqual({ ...planTab, path: "/repo/next.md" });
  });
});
