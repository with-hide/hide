import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { beforeEach, describe, expect, it } from "vitest";
import type { EditorDocumentSnapshot } from "./snapshot";
import { DIAGNOSTIC_CAP, LISTING_CAP, useShellStore } from "./store";

describe("snapshot merge", () => {
  beforeEach(() => {
    useShellStore.setState({
      connection: "connecting",
      revision: 0,
      terminalSequence: null,
      terminalEpoch: null,
      rest: null,
      usage: null,
      agents: [],
      focusedPaneId: null,
      herdrState: null,
      find: null,
      listings: {},
      pathRefusal: null,
      diagnostics: [],
      diagnosticsDropped: 0,
      viewGeneration: 0,
      refused: false,
    });
  });

  it("revisions usage independently and resets it on a fresh snapshot", () => {
    const store = useShellStore.getState();
    const usage = { nodes: [{ node: "local", agents: [], history: null }] };
    store.applyFrame({ type: "snapshot", payload: { revision: 1, usage, rest: { ui_state: { sidebar_width: 292 } } } });
    expect(useShellStore.getState().usage).toBe(usage);
    const rest = useShellStore.getState().rest;
    const changed = { nodes: [{ node: "changed", agents: [], history: null }] };
    store.applyFrame({ type: "delta", payload: { revision: 2, usage: changed } });
    expect(useShellStore.getState().rest).toBe(rest);
    const retained = useShellStore.getState().usage;
    store.applyFrame({ type: "delta", payload: { revision: 2, usage: structuredClone(changed) } });
    expect(useShellStore.getState().usage).toBe(retained);
    store.applyFrame({ type: "delta", payload: { revision: 3, rest: { ui_state: { sidebar_width: 300 } } } });
    expect(useShellStore.getState().usage).toBe(retained);
    store.applyFrame({ type: "delta", payload: { revision: 1, usage } });
    expect(useShellStore.getState().usage).toBe(retained);
    expect(useShellStore.getState().revision).toBe(3);
    store.applyFrame({ type: "delta", payload: { revision: 4, usage: { nodes: [] } } });
    expect(useShellStore.getState().usage?.nodes).toEqual([]);
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest: {} } });
    expect(useShellStore.getState().usage).toBeNull();
  });

  it("names each device's agent rows by their device and keeps their identity across frames", () => {
    const store = useShellStore.getState();
    const device = (agents: unknown[]) => ({ target_id: "hq3mini", state: "connected", message: null, herdr_version: null, session: { agents } });
    const row = { id: "a", pane_id: "remote:hq3mini:pane:w1:p4", agent_kind: "claude", session_id: "s-1" };
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest: { status: { remote: [device([row])] } } } as never });
    const stamped = useShellStore.getState().rest?.status?.remote?.[0]?.session?.agents[0];
    expect(stamped?.device_id).toBe("hq3mini");
    store.applyFrame({ type: "delta", payload: { revision: 2, rest: { status: { remote: [device([{ ...row }])] } } } as never });
    expect(useShellStore.getState().rest?.status?.remote?.[0]?.session?.agents[0]).toBe(stamped);
  });

  it("advances the view generation only on a self-contained snapshot", () => {
    const store = useShellStore.getState();
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest: {} } });
    store.applyFrame({ type: "delta", payload: { revision: 2 } });
    store.applyFrame({ type: "delta", payload: { revision: 3, rest: { focused: { pane_id: "p2" } } } });
    expect(useShellStore.getState().viewGeneration).toBe(1);
    store.applyFrame({ type: "snapshot", payload: { revision: 4, rest: {} } });
    expect(useShellStore.getState().viewGeneration).toBe(2);
  });

  it("drops the front Workspace's view when a delta no longer carries one", () => {
    const store = useShellStore.getState();
    const view = { device_id: "local", path: "/h/hide", open: true, documents: true, tool: "explorer", pinned: false, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0 };
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest: { workspace_view: view } as never } });
    expect(useShellStore.getState().rest?.workspace_view?.path).toBe("/h/hide");
    store.applyFrame({ type: "delta", payload: { revision: 2, rest: { focused: { pane_id: "p2" } } } });
    expect(useShellStore.getState().rest?.workspace_view).toBeUndefined();
  });

  it("merges negotiated semantic fields, retains omitted lists and clears explicit values", () => {
    const store = useShellStore.getState();
    const view = { device_id: "local", path: "/fixture", open: true, documents: true, tool: null };
    const workspaces = [{ id: "a", checkouts: [] }, { id: "b", checkouts: [] }];
    const remote = [{ target_id: "device", session: { workspaces } }];
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest_format: "semantic-rest-v1", rest: {
      workspace_view: view, navigator: { workspaces, agents: [], focused_checkout_id: "a" },
      status: { remote, last_error: null }, terminal: { panes: [{ pane_id: "p1" }] },
    } as never } });
    const initial = useShellStore.getState().rest;
    store.applyFrame({ type: "delta", payload: { revision: 2, rest_format: "semantic-rest-v1", rest: {
      navigator: { focused_checkout_id: "b" }, terminal: { pane_id: "p1" },
    } as never } });
    const selected = useShellStore.getState().rest;
    expect(selected?.navigator?.workspaces).toBe(initial?.navigator?.workspaces);
    expect(selected?.status?.remote).toBe(initial?.status?.remote);
    expect(selected?.terminal?.panes).toBe(initial?.terminal?.panes);
    expect(selected?.workspace_view).toBe(initial?.workspace_view);
    store.applyFrame({ type: "delta", payload: { revision: 3, rest_format: "semantic-rest-v1", rest: {
      workspace_view: null, navigator: { workspaces: [workspaces[1], workspaces[0]] },
      terminal: { panes: [] }, status: { remote: [] },
    } as never } });
    expect(useShellStore.getState().rest?.navigator?.workspaces?.map(row => row.id)).toEqual(["b", "a"]);
    expect(useShellStore.getState().rest?.workspace_view).toBeNull();
    expect(useShellStore.getState().rest?.terminal?.panes).toEqual([]);
    expect(useShellStore.getState().rest?.status?.remote).toEqual([]);
    store.applyFrame({ type: "snapshot", payload: { revision: 1, rest_format: "semantic-rest-v1", rest: {} } });
    expect(useShellStore.getState().rest?.navigator).toBeUndefined();
  });

  it("keeps the newest diagnostics under the cap and counts the rest", () => {
    const store = useShellStore.getState();
    for (let i = 0; i < DIAGNOSTIC_CAP + 5; i += 1) store.noteDiagnostic(`d${i}`);
    const state = useShellStore.getState();
    expect(state.diagnostics).toHaveLength(DIAGNOSTIC_CAP);
    expect(state.diagnostics[0]).toBe("d5");
    expect(state.diagnosticsDropped).toBe(5);
  });

  it("replaces the store on a full snapshot", () => {
    useShellStore.getState().applyFrame({
      type: "snapshot",
      payload: {
        revision: 3,
        rest: {
          navigator: {
            agents: [
              legacyAgentRow({
                id: "a",
                pane_id: "p1",
                identity_label: "codex",
                agent_kind: "codex",
                mark: "turn",
                group: "needs_you",
                status_code: "question",
                changed_at_unix_ms: null,
                emphasized: true,
                unread: true,
              }),
            ],
          },
          focused: { pane_id: "p1" },
          status: { herdr: { state: "unconfigured" } },
        },
      },
    });
    const state = useShellStore.getState();
    expect(state.agents).toHaveLength(1);
    expect(state.focusedPaneId).toBe("p1");
    expect(state.herdrState).toBe("unconfigured");
  });

  it("draws unknown enum rows without stalling", () => {
    useShellStore.getState().applyFrame({
      type: "snapshot",
      payload: {
        rest: {
          navigator: {
            agents: [
              legacyAgentRow({
                id: "a",
                pane_id: "p1",
                identity_label: "x",
                agent_kind: "x",
                mark: "turn",
                group: "brand_new",
                status_code: "unknown",
                changed_at_unix_ms: null,
                emphasized: false,
                unread: false,
              }),
            ],
          },
        },
      },
    });
    const state = useShellStore.getState();
    expect(state.agents[0]?.unknown).toBe(true);
    // A state Hide cannot read draws no mark rather than a `!` (status-marks B16).
    expect(state.agents[0]?.mark).toBe("unknown");
    expect(state.diagnostics[0]).toContain("brand_new");
  });

  it("draws a mark this shell does not know as no mark, and logs it", () => {
    useShellStore.getState().applyFrame({
      type: "snapshot",
      payload: { rest: { navigator: { agents: [legacyAgentRow({ id: "a", pane_id: "p1", identity_label: "x", agent_kind: "x", mark: "sparkle" as never, group: "working", status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false })] } } },
    });
    const state = useShellStore.getState();
    expect([state.agents[0]?.mark, state.agents[0]?.status_code]).toEqual(["unknown", "unknown"]);
    expect(state.diagnostics.some((line) => line.includes("mark=sparkle"))).toBe(true);
  });

  it("keeps a sleeping agent's moon and a waiting agent's dashed ring (status-marks D-21, D-25)", () => {
    useShellStore.getState().applyFrame({
      type: "snapshot",
      payload: { rest: { navigator: { agents: [
        legacyAgentRow({ id: "a", pane_id: "p1", identity_label: "x", agent_kind: "x", mark: "asleep", group: "seen", status_code: "sleeping", changed_at_unix_ms: null, emphasized: false, unread: false }),
        legacyAgentRow({ id: "b", pane_id: "p2", identity_label: "y", agent_kind: "x", mark: "waiting", group: "working", status_code: "waiting", wait: "checks", changed_at_unix_ms: null, emphasized: false, unread: false }),
      ] } } },
    });
    const state = useShellStore.getState();
    expect(state.agents.map((agent) => [agent.mark, agent.status_code])).toEqual([["asleep", "sleeping"], ["waiting", "waiting"]]);
  });

  it("returns a terminal frame's chunks without storing them and keeps its cursor and the hub's epoch", () => {
    const chunks = useShellStore.getState().applyFrame({
      type: "terminal",
      payload: {
        chunks: [{ pane_id: "p1", bytes_base64: "YQ==" }],
        terminal_epoch: "00112233aabbccdd",
        terminal_sequence: 7,
      },
    });
    expect(chunks).toHaveLength(1);
    expect(useShellStore.getState().agents).toEqual([]);
    expect(useShellStore.getState().terminalSequence).toBe(7);
    expect(useShellStore.getState().terminalEpoch).toBe("00112233aabbccdd");
    // A delta keeps the cursor; a whole snapshot clears it.
    useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 4 } });
    expect(useShellStore.getState().terminalSequence).toBe(7);
    expect(useShellStore.getState().terminalEpoch).toBe("00112233aabbccdd");
    useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 5, rest: {} as never } });
    expect(useShellStore.getState().terminalSequence).toBeNull();
    expect(useShellStore.getState().terminalEpoch).toBeNull();
  });

  it("keeps untouched workspace rows by reference across a delta", () => {
    const workspace = { agent_scope: emptyScope(),
      id: "w1",
      label: "hide",
      path: "/h/hide",
      device_id: "local",
      registered: true,
      temporary: false,
      pinned: false,
      checkouts: [],
      inactive_checkouts: { expanded: false, checkout_ids: [] },
    };
    const store = useShellStore.getState();
    store.applyFrame({
      type: "snapshot",
      payload: { revision: 1, rest: { navigator: { workspaces: [workspace], agents: [] } } },
    });
    const before = useShellStore.getState().rest?.navigator?.workspaces?.[0];
    store.applyFrame({
      type: "delta",
      payload: {
        revision: 2,
        rest: {
          navigator: {
            workspaces: [{ ...workspace }],
            agents: [
              legacyAgentRow({
                id: "a",
                pane_id: "p1",
                identity_label: "codex",
                agent_kind: "codex",
                mark: "turn",
                group: "working",
                status_code: "working",
                changed_at_unix_ms: null,
                emphasized: false,
                unread: false,
              }),
            ],
          },
        },
      },
    });
    const after = useShellStore.getState().rest?.navigator?.workspaces?.[0];
    expect(after).toBe(before);
    expect(useShellStore.getState().agents).toHaveLength(1);
  });

  it("stores hided's path refusal frames", () => {
    const store = useShellStore.getState();
    store.applyFrame({
      type: "path_refused",
      payload: { kind: "create_workspace", path: "/etc", reason: "outside_home" },
    });
    expect(useShellStore.getState().pathRefusal?.reason).toBe("outside_home");
    store.clearPathRefusal();
    expect(useShellStore.getState().pathRefusal).toBeNull();
    store.applyFrame({ type: "error", message: "client json: boom" });
    expect(useShellStore.getState().diagnostics.at(-1)).toContain("boom");
  });

  it("re-reads a watched folder only for the device it shows", () => {
    const store = useShellStore.getState();
    useShellStore.setState({
      rest: { navigator: { focused_device_id: "mac" } } as never,
      listings: { "/r/src": { kind: "file_list", root_path: "/r/src", entries: [], truncated: false } as never },
      folderChanges: {},
    });
    store.applyFrame({ type: "directory_changed", payload: { path: "/r/src", device_id: "local" } });
    expect(useShellStore.getState().listings["/r/src"]?.stale).toBeUndefined();
    store.applyFrame({ type: "directory_changed", payload: { path: "/r/src", device_id: "mac" } });
    expect(useShellStore.getState().listings["/r/src"]?.stale).toBe(true);
    expect(useShellStore.getState().folderChanges["/r/src"]).toBe(1);
  });

  it("keeps a changed folder's rows until its re-read lands", () => {
    const store = useShellStore.getState();
    const entry = { name: "a.ts", path: "/r/src/a.ts", is_directory: false };
    useShellStore.setState({
      rest: { navigator: { focused_device_id: "local", devices: [{ id: "local", kind: "local" }] } } as never,
      listings: { "/r/src": { kind: "file_list", root_path: "/r/src", entries: [entry], truncated: false } as never },
      folderChanges: {},
    });
    store.applyFrame({ type: "directory_changed", payload: { path: "/r/src" } });
    expect(useShellStore.getState().listings["/r/src"]).toMatchObject({ entries: [entry], stale: true });
    store.applyFrame({ type: "directory_list", payload: { kind: "file_list", root_path: "/r/src", entries: [], truncated: false } });
    expect(useShellStore.getState().listings["/r/src"]).toEqual({ kind: "file_list", root_path: "/r/src", entries: [], truncated: false });
  });

  it("keeps the newest LISTING_CAP folders and drops the oldest", () => {
    const store = useShellStore.getState();
    for (let i = 0; i < LISTING_CAP + 2; i += 1) {
      store.applyFrame({
        type: "directory_list",
        payload: { kind: "file_list", root_path: `/repo/f${i}`, entries: [], truncated: false },
      });
    }
    const paths = Object.keys(useShellStore.getState().listings);
    expect(paths).toHaveLength(LISTING_CAP);
    expect(paths[0]).toBe("/repo/f2");
    expect(paths.at(-1)).toBe(`/repo/f${LISTING_CAP + 1}`);
  });

  it("records a listing whose kind it does not know instead of guessing a flow", () => {
    const store = useShellStore.getState();
    store.applyFrame({
      type: "directory_list",
      payload: { root_path: "/h", entries: [], truncated: false },
    });
    const state = useShellStore.getState();
    expect(state.listings).toEqual({});
    expect(state.diagnostics.at(-1)).toContain("directory_list without a known kind=none");
  });
});

describe("the documents section", () => {
  const doc = (path: string, contents: string): EditorDocumentSnapshot => ({
    path,
    language: null,
    document_kind: "text",
    contents_utf8: contents,
    revision: null,
    dirty: false,
    readonly_reason: null,
    conflict: null,
    save: null,
  });

  beforeEach(() => {
    useShellStore.setState({ documents: {}, editor: null, changes: null });
  });

  it("replaces the map with a snapshot's section, and empties it when a snapshot has none", () => {
    const store = useShellStore.getState();
    store.applyFrame({ type: "snapshot", payload: { rest: {}, documents: { visible: ["a", "b"], changed: [{ tab_id: "a", document: doc("/a", "1") }, { tab_id: "b", document: doc("/b", "1") }] } } });
    store.applyFrame({ type: "snapshot", payload: { rest: {}, documents: { visible: ["c"], changed: [{ tab_id: "c", document: doc("/c", "1") }] } } });
    expect(Object.keys(useShellStore.getState().documents)).toEqual(["c"]);
    store.applyFrame({ type: "snapshot", payload: { rest: {} } });
    expect(useShellStore.getState().documents).toEqual({});
  });

  it("keeps a still-visible document, overwrites a changed one and drops one no longer visible", () => {
    const store = useShellStore.getState();
    const a = doc("/a", "1");
    store.applyFrame({ type: "snapshot", payload: { rest: {}, documents: { visible: ["a", "b"], changed: [{ tab_id: "a", document: a }, { tab_id: "b", document: doc("/b", "1") }] } } });
    const c = doc("/c", "2");
    store.applyFrame({ type: "delta", payload: { documents: { visible: ["a", "c"], changed: [{ tab_id: "c", document: c }] } } });
    const documents = useShellStore.getState().documents;
    expect(Object.keys(documents).sort()).toEqual(["a", "c"]);
    expect(documents.a).toBe(a);
    expect(documents.c).toBe(c);
    const edited = doc("/a", "typed");
    store.applyFrame({ type: "delta", payload: { documents: { visible: ["a", "c"], changed: [{ tab_id: "a", document: edited }] } } });
    expect(useShellStore.getState().documents.a).toBe(edited);
    expect(useShellStore.getState().documents.c).toBe(c);
  });

  it("keeps the map, object for object, through a delta without the section or one that changes nothing", () => {
    const store = useShellStore.getState();
    store.applyFrame({ type: "snapshot", payload: { rest: {}, documents: { visible: ["a"], changed: [{ tab_id: "a", document: doc("/a", "1") }] } } });
    const before = useShellStore.getState().documents;
    store.applyFrame({ type: "delta", payload: { revision: 2, changes: null, rest: { focused: { pane_id: "p2" } } } });
    expect(useShellStore.getState().documents).toBe(before);
    store.applyFrame({ type: "delta", payload: { documents: { visible: ["a"], changed: [] } } });
    expect(useShellStore.getState().documents).toBe(before);
  });
});
