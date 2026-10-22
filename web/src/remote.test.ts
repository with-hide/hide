import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { initializeInterfaceI18n } from "./i18n/instance";
import { deviceCatalogLine, herdrPaneId, remoteTabView, remoteTargetOfPane, remoteView, supportsRemotePurpose } from "./remote";
import { queueBuffer, tabBufferKey } from "./buffers";
import type { Device, EditorSnapshot, RemoteSession, RemoteStatus, SnapshotRest, Tab, Workspace } from "./snapshot";
import { useShellStore, type DaemonInfo } from "./store";
import { draftExported, unstoredDeviceDrafts } from "./settings";
import { useUiStore } from "./ui";
import { noteKeyboardOwner } from "./viewFocus";
import { frontCheckout } from "./snapshot";
import type { WorkspaceView } from "./workspace";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

const LOCAL_PANE = "w1:p1";
const PANE_A = "remote:studio:pane:w9:p1";
const PANE_B = "remote:studio:pane:w9:p2";

function tab(id: string, paneIds: string[], working: string[] = []): Tab {
  return {
    id,
    workspace_id: null,
    checkout_id: null,
    label: id.split(":").pop() ?? id,
    empty: false,
    delegated: false,
    panes: paneIds.map((paneId) => ({
      id: paneId,
      herdr_label: paneId,
      terminal_title: null,
      cwd: "/home/remote/app",
      status_code: "idle",
      requires_close_confirmation: working.includes(paneId),
      requires_close_status_check: false,
      identity_label: null,
    })),
  };
}

function session(overrides: Partial<RemoteSession> = {}): RemoteSession {
  const workspace: Workspace = { agent_scope: emptyScope(),
    id: "remote:studio:workspace:w9",
    label: "app",
    path: "/home/remote/app",
    device_id: "studio",
    remote_target_id: "studio",
    registered: false,
    temporary: false,
    pinned: false,
    checkouts: [
      { agent_scope: emptyScope(),
        id: "remote:studio:checkout:w9",
        workspace_id: "remote:studio:workspace:w9",
        label: "app",
        path: "/home/remote/app",
        branch: null,
        purpose: null,
        is_worktree: false,
        exists: true,
        has_panes: true,
        pull_request: null,
        tabs: [tab("remote:studio:tab:w9:t1", [PANE_A, PANE_B], [PANE_B]), tab("remote:studio:tab:w9:t2", ["remote:studio:pane:w9:p3"])],
        active_tab_id: null,
        strip: [],
        next_tab_label: "3",
      },
    ],
    inactive_checkouts: { expanded: false, checkout_ids: [] },
  };
  return {
    workspaces: [workspace],
    agents: [],
    active_tab_ids: { "remote:studio:checkout:w9": "remote:studio:tab:w9:t1" },
    focused_workspace_id: "remote:studio:workspace:w9",
    focused_checkout_id: "remote:studio:checkout:w9",
    focused_tab_id: "remote:studio:tab:w9:t1",
    focused_pane_id: PANE_A,
    pane_layouts: [
      {
        workspace_id: "remote:studio:workspace:w9",
        tab_id: "remote:studio:tab:w9:t1",
        focused_pane_id: PANE_A,
        zoomed: false,
        frames: [
          { pane_id: PANE_A, x: 0, y: 0, width: 0.5, height: 1 },
          { pane_id: PANE_B, x: 0.5, y: 0, width: 0.5, height: 1 },
        ],
      },
    ],
    ...overrides,
  };
}

const STUDIO: Device = { agent_scope: emptyScope(), id: "studio", label: "Studio Mac", kind: "remote", state: "ready", message: null, ssh_alias: "studio", agent_count: 0, test: null };

function rest(focusedDevice: string, state = "connected"): SnapshotRest {
  return {
    navigator: {
      focused_device_id: focusedDevice,
      focused_checkout_id: "c-local",
      devices: [{ ...STUDIO, id: "local", label: "This Mac", kind: "local", state: "local", ssh_alias: null }, STUDIO],
      workspaces: [
        { agent_scope: emptyScope(),
          id: "p-local",
          label: "local",
          path: "/Users/example/app",
          device_id: "local",
          registered: true,
          temporary: false,
          pinned: false,
          inactive_checkouts: { expanded: false, checkout_ids: [] },
          checkouts: [
            { agent_scope: emptyScope(),
              id: "c-local",
              workspace_id: "p-local",
              label: "app",
              path: "/Users/example/app",
              branch: "main",
              purpose: null,
              is_worktree: false,
              exists: true,
              has_panes: true,
              pull_request: null,
              tabs: [tab("w1:t1", [LOCAL_PANE])],
              active_tab_id: "w1:t1",
              strip: [],
              next_tab_label: "2",
            },
          ],
        },
      ],
    },
    // What typing into the remote pane leaves in the core's terminal pane.
    terminal: { pane_id: PANE_A, panes: [] },
    status: { remote: [{ target_id: "studio", state, message: null, herdr_version: "0.9.1", session: session() }] },
  };
}

/** The device Workspace in front, its one area showing a device file. */
function deviceViews(views: boolean): WorkspaceView {
  const notes = { id: "d2", tab_id: "file:studio:/home/remote/app/notes.md", path: "/home/remote/app/notes.md", label: "notes.md", kind: "file" as const, committed: null, preview: false, state: "open" as const, reason: null };
  return {
    device_id: "studio",
    path: "/home/remote/app",
    open: views,
    documents: views,
    tool: null,
    
    pinned: true,
    document_width: null,
    detail_width: null,
    explorer_width: null,
    panel_width: null,
    lane_called: 0, lane_calls: 0, lane_called_side: "view",
    layout: { root: { area: { id: "a1", active: "d2", displays: [notes] } }, active_area: "a1", limits: { areas: 6, depth: 3, displays: 64 }, display_count: 1 },
  };
}

function seed(value: SnapshotRest) {
  noteKeyboardOwner({ kind: "none" });
  useShellStore.setState({ rest: null, agents: [], focusedPaneId: null });
  useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 1, rest: legacyRest(value, []) } });
}

function recorder() {
  const sent: { kind: string; payload: Record<string, unknown> }[] = [];
  const actions = createActions((event) => {
    sent.push(event as { kind: string; payload: Record<string, unknown> });
    return true;
  });
  return { sent, actions };
}

describe("remote view", () => {
  it("keeps a core-selected draft or empty group visible instead of the host's live tab", () => {
    const host = session();
    const front = { device_id: "studio", path: "/home/remote/app", agent_layout: { root: { area: { id: "a1", active: "draft-1", displays: [{ id: "draft-1" }] } }, active_area: "a1", canvases: {}, drafts: ["draft-1"] } } as WorkspaceView;
    expect(remoteView(host, front)?.tab).toBeNull();
    expect(remoteView(host, front)?.focusedPaneId).toBeNull();
    front.agent_layout!.root = { area: { id: "a1", active: null, displays: [] } };
    expect(remoteView(host, front)?.tab).toBeNull();
    expect(remoteView(host)?.tab).not.toBeNull();
  });

  it("draws the tab the host focused, which is the one the core attaches", () => {
    const view = remoteView(session());
    expect(view?.tab?.id).toBe("remote:studio:tab:w9:t1");
    expect(view?.layout?.frames).toHaveLength(2);
    expect(view?.focusedPaneId).toBe(PANE_A);
  });

  it("falls back to the focused workspace's active tab and the layout's focused pane", () => {
    const view = remoteView(session({ focused_tab_id: null, focused_pane_id: "remote:studio:pane:w9:gone" }));
    expect(view?.tab?.id).toBe("remote:studio:tab:w9:t1");
    expect(view?.focusedPaneId).toBe(PANE_A);
  });

  it("draws another Agent area's tab of a node with its own panes and layout, the keyboard left on the front tab", () => {
    const second = { workspace_id: "remote:studio:workspace:w9", tab_id: "remote:studio:tab:w9:t2", focused_pane_id: "remote:studio:pane:w9:p3", zoomed: false, frames: [{ pane_id: "remote:studio:pane:w9:p3", x: 0, y: 0, width: 1, height: 1 }] };
    const host = session({ pane_layouts: [...session().pane_layouts, second] });
    const front = remoteView(host)!;
    expect(remoteTabView(host, front, null)).toBe(front);
    expect(remoteTabView(host, front, "remote:studio:tab:w9:t1")).toBe(front);
    const beside = remoteTabView(host, front, "remote:studio:tab:w9:t2");
    expect(beside.tab?.panes.map((pane) => pane.id)).toEqual(["remote:studio:pane:w9:p3"]);
    expect(beside.layout).toEqual(second);
    expect(beside.focusedPaneId).toBeNull();
  });

  it("names a pane's host by its scoped id and never a similar target", () => {
    const value: SnapshotRest = { status: { remote: [{ target_id: "studio", state: "connected", message: null, herdr_version: null }, { target_id: "studio2", state: "connected", message: null, herdr_version: null }] } };
    expect(remoteTargetOfPane(value, PANE_A)).toBe("studio");
    expect(remoteTargetOfPane(value, "remote:studio2:pane:w1:p1")).toBe("studio2");
    expect(remoteTargetOfPane(value, LOCAL_PANE)).toBeNull();
  });

  it("copies a pane by the id its own Herdr knows it by, without the device scope", () => {
    expect(herdrPaneId("w9J:p52")).toBe("w9J:p52");
    expect(herdrPaneId("remote:mini:pane:w9J:p52")).toBe("w9J:p52");
    expect(herdrPaneId("remote:studio2:pane:w1:pB")).toBe("w1:pB");
  });

  it("stores a remote purpose only from Herdr 0.9.1", () => {
    expect(supportsRemotePurpose("0.9.1")).toBe(true);
    expect(supportsRemotePurpose("v0.10.0")).toBe(true);
    expect(supportsRemotePurpose("0.9.0")).toBe(false);
    expect(supportsRemotePurpose(null)).toBe(false);
  });
});

describe("commands with an SSH device selected", () => {
  beforeEach(() => {
    useUiStore.setState({ notice: null, pendingClose: null });
  });

  it("splits, zooms and focuses the host's exact pane and sends nothing local", () => {
    seed(rest("studio"));
    expect(useShellStore.getState().focusedPaneId).toBe(PANE_A);
    const { sent, actions } = recorder();
    actions.split("right");
    actions.toggleZoom();
    actions.focusPane(PANE_B);
    actions.focusTab("remote:studio:tab:w9:t2");
    actions.createTab();
    expect(sent.map((event) => event.kind)).toEqual(["remote_control", "remote_control", "remote_control", "remote_control", "remote_control"]);
    expect(sent.map((event) => event.payload.target_id)).toEqual(["studio", "studio", "studio", "studio", "studio"]);
    expect(sent[0]?.payload).toMatchObject({ action: "split_pane", pane_id: PANE_A, direction: "right", cwd: "/home/remote/app" });
    expect(sent[1]?.payload).toMatchObject({ action: "toggle_pane_zoom", pane_id: PANE_A });
    expect(sent[2]?.payload).toMatchObject({ action: "focus_pane", pane_id: PANE_B });
    expect(sent[3]?.payload).toMatchObject({ action: "focus_tab", tab_id: "remote:studio:tab:w9:t2" });
    expect(sent[4]?.payload).toMatchObject({ action: "create_tab", workspace_id: "remote:studio:workspace:w9", checkout_id: "remote:studio:checkout:w9", cwd: "/home/remote/app", label: "3" });
    expect(new Set(sent.map((event) => event.payload.request_id)).size).toBe(5);
  });

  it("asks before closing working remote work and closes it on that host after confirming", () => {
    seed(rest("studio"));
    const { sent, actions } = recorder();
    actions.closePane(PANE_B);
    expect(sent).toHaveLength(0);
    expect(useUiStore.getState().pendingClose).toMatchObject({ kind: "pane", id: PANE_B, targetId: "studio" });
    actions.confirmClose();
    expect(sent[0]).toMatchObject({ kind: "remote_control", payload: { target_id: "studio", action: "close_pane", pane_id: PANE_B, confirmed: true } });
  });

  it("sends nothing to a device that is not connected and says so", () => {
    seed(rest("studio", "stale"));
    const { sent, actions } = recorder();
    actions.split("down");
    actions.closeTab("remote:studio:tab:w9:t1");
    expect(sent).toHaveLength(0);
    expect(useUiStore.getState().notice?.text).toContain("Studio Mac is not connected");
  });

  it("closes the device Workspace's active view with the close chord while its View area owns the keyboard, connected or not", () => {
    for (const state of ["connected", "stale"]) {
      useUiStore.setState({ notice: null, pendingClose: null });
      seed({ ...rest("studio", state), workspace_view: deviceViews(true) });
      const { sent, actions } = recorder();
      useUiStore.setState({ screen: { kind: "workspace" }, drawnLane: { mode: "docked", view: "documents", tool: null, stacked: false } });
      noteKeyboardOwner({ kind: "view", workspace: frontCheckout(useShellStore.getState().rest)!.id, areaId: "a1" });
      actions.closeFocused();
      expect(sent).toHaveLength(1);
      expect(sent[0]).toMatchObject({
        kind: "view_layout",
        payload: { action: "close", display_id: "d2", workspace: { device_id: "studio", path: "/home/remote/app" } },
      });
      expect(useUiStore.getState().notice).toBeNull();
      expect(useUiStore.getState().pendingClose).toBeNull();
    }
  });

  it("closes the device's Herdr tab only when its explicit control names it", () => {
    seed({ ...rest("studio"), workspace_view: deviceViews(false) });
    const { sent, actions } = recorder();
    actions.closeTab("remote:studio:tab:w9:t1");
    expect(sent).toHaveLength(0);
    expect(useUiStore.getState().pendingClose).toMatchObject({ kind: "tab", id: "remote:studio:tab:w9:t1", targetId: "studio" });
  });

  it("opens the device pane's find bar, which the core answers through the device's Herdr", () => {
    seed(rest("studio"));
    const { sent, actions } = recorder();
    actions.openFind();
    expect(sent).toHaveLength(0);
    expect(useUiStore.getState().overlay).toBe("find");
    expect(useUiStore.getState().notice).toBeNull();
  });

  it("reorders the device's own strip, naming its checkout", () => {
    seed(rest("studio"));
    const { sent, actions } = recorder();
    actions.reorderTab("herdr:remote:studio:tab:w9:t2", 0);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ kind: "reorder_tab", payload: { tab_id: "herdr:remote:studio:tab:w9:t2", to_index: 0 } });
    expect(String(sent[0]?.payload.checkout_id)).toMatch(/^remote:studio:/);
  });

  it("splits this machine's pane again once this machine is selected", () => {
    const local = rest("local");
    seed({ ...local, terminal: { pane_id: LOCAL_PANE, panes: [] } });
    const { sent, actions } = recorder();
    actions.split("right");
    expect(sent[0]).toMatchObject({ kind: "create_pane", payload: { tab_id: "w1:t1", direction: "right" } });
  });
});

describe("deviceCatalogLine", () => {
  const device: Device = { agent_scope: emptyScope(), id: "studio", label: "Studio", kind: "remote", state: "ready", message: null, ssh_alias: "studio", herdr_socket_path: null, agent_count: 0 } as Device;
  const status = (state: string, session: RemoteSession | null, catalog?: RemoteStatus["catalog"]): RemoteStatus => ({
    target_id: "studio",
    state,
    message: state === "connected" ? null : "ssh refused",
    herdr_version: null,
    session,
    catalog,
  });
  const line = (value: RemoteStatus | null) => deviceCatalogLine({ device, status: value, session: value?.session ?? null }, [device], english);

  it("names the window's own machine This Mac in a window on a node whose core runs elsewhere", () => {
    vi.stubGlobal("window", { location: { hash: "#token=t&node=studio" } });
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    try {
      expect(line(null)?.text).toBe(english("devices.catalog.loading", { device: "This Mac" }));
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("tells loading, failure, a kept stale list and an empty host apart", () => {
    expect(line(null)?.state).toBe("loading");
    expect(line(status("auth_failed", null))).toMatchObject({ state: "error", text: "Studio is auth failed: ssh refused" });
    expect(line(status("stale", session()))?.state).toBe("stale");
    expect(line(status("connected", { ...session(), workspaces: [] }))?.state).toBe("empty");
  });

  it("never presents projects the helper has not confirmed as confirmed", () => {
    expect(line(status("connected", session(), { state: "resolving", message: null, refused: [] }))?.state).toBe("resolving");
    expect(line(status("connected", session(), { state: "unavailable", message: "Allow the helper in Settings", refused: [] }))?.text).toContain("Allow the helper in Settings");
    expect(line(status("connected", session(), { state: "ready", message: null, refused: [{ path: "/gone", message: "missing" }] }))?.state).toBe("partial");
    expect(line(status("connected", session(), { state: "ready", message: null, refused: [] }))).toBeNull();
  });

  it("words the line in the interface language, a folder count included", () => {
    const refused = (count: number) => status("connected", session(), { state: "ready", message: null, refused: Array.from({ length: count }, (_, index) => ({ path: `/gone${index}`, message: "missing" })) });
    expect(line(refused(1))?.text).toBe("1 folder on Studio could not be read and is listed by workspace.");
    expect(line(refused(2))?.text).toBe("2 folders on Studio could not be read and are listed by workspace.");
    expect(deviceCatalogLine({ device, status: refused(2), session: session() }, [device], korean)?.text).toBe("Studio의 폴더 2개를 읽지 못해 워크스페이스별로 표시합니다.");
    expect(deviceCatalogLine({ device, status: null, session: null }, [device], korean)?.text).toBe("Studio에서 프로젝트를 읽는 중…");
  });
});

describe("removing a device (S5.5 B26, B44)", () => {
  const TAB = "remote:studio:file:a";
  const withTab = () => {
    seed(rest("studio"));
    useShellStore.setState({
      daemon: { host_id: "host-a" } as unknown as DaemonInfo,
      editor: { tabs: [{ id: TAB, checkout_id: "remote:studio:checkout:w9", path: "/home/remote/app/a.ts" }] } as unknown as EditorSnapshot,
      bufferWarnings: new Set<string>(),
    });
  };

  it("sends nothing while a draft of its tabs could not be stored, and names it", async () => {
    withTab();
    const key = tabBufferKey("host-a", useShellStore.getState().rest, { checkout_id: "remote:studio:checkout:w9", path: "/home/remote/app/a.ts" });
    expect(key).not.toBeNull();
    // The tab's last edit is still queued when the removal is asked for, and
    // storing it fails during the flush the removal waits for.
    queueBuffer(key!, "typed", (ok) => useShellStore.getState().noteBufferWarning(TAB, ok === false));
    const { sent, actions } = recorder();
    await expect(actions.removeDevice("studio")).resolves.toEqual(["/home/remote/app/a.ts"]);
    expect(sent.filter((event) => event.kind === "remove_device")).toEqual([]);
  });

  it("is sent once every draft of its tabs is stored", async () => {
    withTab();
    const { sent, actions } = recorder();
    await expect(actions.removeDevice("studio")).resolves.toEqual([]);
    expect(sent.filter((event) => event.kind === "remove_device")).toEqual([{ kind: "remove_device", payload: { device_id: "studio" }, schema_version: 2 }]);
  });
});

describe("an exported draft and a device removal (S5.5 B26, B44)", () => {
  it("releases the hold for the exact text exported, and holds again after a newer edit", () => {
    const tabs = [{ id: "t", checkout_id: "remote:studio:checkout:w9", path: "/home/remote/app/a.ts" }];
    const unstored = new Set(["t"]);
    let current = "typed";
    const exported = new Map([["t", "typed"]]);
    expect(unstoredDeviceDrafts("studio", tabs, unstored, draftExported(exported, () => current))).toEqual([]);
    current = "typed more";
    expect(unstoredDeviceDrafts("studio", tabs, unstored, draftExported(exported, () => current))).toEqual(["/home/remote/app/a.ts"]);
    expect(unstoredDeviceDrafts("studio", tabs, unstored)).toEqual(["/home/remote/app/a.ts"]);
  });
});
