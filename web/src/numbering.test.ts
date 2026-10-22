import { projectRows, projectListNumbers } from "../test/legacyAgentScope";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { describe, expect, it } from "vitest";
import type { TreeRow } from "./agentRow";
import { numberedTarget } from "./keyboard";
import { agentListOrder, numberedAgents, numberedDevices, sidebarAgentNumbers, sidebarNumbers, taskListOrder } from "./numbering";
import type { TaskCardScope } from "./agentScope";

import type { AgentRow, Checkout, SnapshotRest, Workspace } from "./snapshot";

function checkoutWith(tabs: string[], extra: { id: string; kind: "file" | "diff" }[] = []): Checkout {
  return { agent_scope: emptyScope(),
    id: "c1",
    workspace_id: "w1",
    strip: [
      ...tabs.map((id) => ({ id: `herdr:${id}`, kind: "herdr" as const, source_id: id, label: id, preview: false })),
      ...extra.map((row) => ({ id: `${row.kind}:${row.id}`, kind: row.kind, source_id: row.id, label: row.id, preview: false })),
    ],
    tabs: tabs.map((id) => ({ id, label: id, panes: [] })),
    active_tab_id: tabs[0] ?? null,
  } as unknown as Checkout;
}

function agent(paneId: string, extra: Partial<AgentRow> = {}): AgentRow {
  return legacyAgentRow({ id: paneId, pane_id: paneId, identity_label: paneId, agent_kind: "claude", mark: "going", group: "working", status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false, ...extra });
}

it("numbers grouped roots in physical tab order, skipping children and duplicate raised occurrences", () => {
  const agents = [agent("a", { group: "needs_you", lineage_child_pane_ids: ["child"], lineage_collapsed: false }), agent("b"), agent("c"), agent("single"), agent("child", { delegated: true, lineage_parent_pane_id: "a" })];
  const checkout = checkoutWith(["t1", "t2", "t3"]);
  for (const [index, panes] of [[0, ["a", "b", "child"]], [1, ["c"]], [2, ["single"]]] as const) {
    checkout.tabs[index]!.panes = panes.map((id) => ({ id })) as Checkout["tabs"][number]["panes"];
    checkout.tabs[index]!.sidebar = { manual_name: null, number: index + 1, named_by_operator: false };
  }
  const rest = legacyRest({ navigator: { focused_device_id: "local", devices: [{ id: "local", kind: "local" }], workspaces: [{ id: "w1", device_id: "local", expanded: true, checkouts: [checkout] }] }, status: { remote: [] } } as unknown as SnapshotRest, agents);
  const placed = rest.navigator!.workspaces![0]!.checkouts[0]!;
  // The core can order roots differently from physical tabs; lineage must stay with its root.
  const refs = ["c", "a", "child", "b", "single"].map((pane_id) => ({ pane_id, occurrence: 0, depth: pane_id === "child" ? 1 : 0, descendants: pane_id === "a" ? 1 : 0 }));
  placed.agent_scope.sidebar_tree.rows = refs;
  placed.agent_scope.sidebar_tree.visible_rows = refs;
  const state = { rest, agents };
  expect([...numberedAgents(agentListOrder(state)).values()]).toEqual(["a", "b", "c", "single"]);
  const keycaps = sidebarAgentNumbers(state, []);
  expect(keycaps("a", null)).toBe(1);
  expect(keycaps("a", "c1")).toBeNull();
  expect(keycaps("b", "c1")).toBe(2);
  expect(keycaps("child", "c1")).toBeNull();
});

describe("numbering (electron-digit-shortcuts-hints D-02)", () => {
  it("numbers the device rail's tiles, the window's machine first, and ⌘n selects by them", () => {
    const devices = [
      { id: "mini", kind: "remote", label: "mini" },
      { id: "local", kind: "local", label: "This Mac" },
      { id: "box", kind: "remote", label: "build-box" },
    ];
    const rest = { navigator: { focused_device_id: "local", devices } } as unknown as SnapshotRest;
    expect([...numberedDevices(rest.navigator!.devices)]).toEqual([[1, "local"], [2, "mini"], [3, "box"]]);
    expect(numberedTarget("devices", 2, { rest, agents: [] })).toBe("mini");
    expect(numberedTarget("devices", 4, { rest, agents: [] })).toBeNull();
    // Before the core names its machine, no tile has an id to select.
    expect([...numberedDevices(undefined)]).toEqual([]);
  });

  it("numbers the Agents list's drawn rows top to bottom, folded children left out", () => {
    const rows: TreeRow[] = [
      { agent: agent("p1", { lineage_collapsed: false, lineage_child_pane_ids: ["p2"] }), device: null, depth: 0, descendants: 1 },
      { agent: agent("p2", { delegated: true }), device: null, depth: 1, descendants: 0 },
      { agent: agent("p3"), device: "mini", depth: 0, descendants: 0 },
    ];
    expect([...numberedAgents(rows)]).toEqual([
      [1, "p1"],
      [2, "p2"],
      [3, "p3"],
    ]);
    expect(numberedAgents([]).size).toBe(0);
  });

  it("numbers only the device in front's Agents list, so another device's agents hold no number (quick device-rail-badges B3)", () => {
    const devices = [
      { id: "local", label: "This Mac", kind: "local" },
      { id: "mini", label: "mini", kind: "remote" },
    ];
    const project = (id: string, device_id: string, pane: string) => ({ id, device_id, expanded: true, checkouts: [{ id: `${id}-checkout`, tabs: [{ id: `${id}-tab`, panes: [{ id: pane }] }] }] });
    const status = { remote: [{ target_id: "mini", state: "connected", session: { agents: [agent("r1", { group: "needs_you" })], workspaces: [project("remote", "mini", "r1")] } }] };
    const local = [agent("l1")];
    const at = (front: string) => [...numberedAgents(agentListOrder({ rest: legacyRest({ navigator: { focused_device_id: front, devices, workspaces: [project("local-project", "local", "l1")] }, status } as unknown as SnapshotRest, local), agents: local })).values()];
    expect(at("local")).toEqual(["l1"]);
    expect(at("mini")).toEqual(["r1"]);
  });

  it("selects what holds the number, and nothing past the end", () => {
    const parent = agent("p1", { lineage_child_pane_ids: ["p2"], lineage_collapsed: true });
    const child = agent("p2", { delegated: true, lineage_parent_pane_id: "p1" });
    const other = agent("p3", { group: "needs_you" });
    const checkout = checkoutWith(["t1", "t2"]);
    checkout.tabs[0]!.panes = ["p1", "p2", "p3"].map((id) => ({ id })) as Checkout["tabs"][number]["panes"];
    const rest = {
      navigator: { focused_checkout_id: "c1", focused_workspace_id: "w1", focused_device_id: "local", devices: [{ id: "local", kind: "local", label: "This Mac" }], workspaces: [{ id: "w1", label: "w", device_id: "local", expanded: true, checkouts: [checkout] }] },
      status: { remote: [] },
    } as unknown as SnapshotRest;
    const state = { rest: legacyRest(rest, [parent, child, other]), agents: [parent, child, other] };
    // Needs You draws first; the folded child is not a row, so p1 is second and 3 is empty.
    expect(numberedTarget("agents", 1, state)).toBe("p3");
    expect(numberedTarget("agents", 2, state)).toBe("p1");
    expect(numberedTarget("agents", 3, state)).toBeNull();
    expect(numberedTarget("agents", 1, { rest: null, agents: [] })).toBeNull();
  });

  it("shows each agent's Agents-list number once in Projects: its raised row, else its own checkout's row", () => {
    const checkout = (id: string, panes: string[]) => ({ id, tabs: [{ id: `${id}-t`, panes: panes.map((pane) => ({ id: pane })) }] });
    const workspaces = [{ agent_scope: emptyScope(), id: "w", device_id: "local", pinned: false, checkouts: [checkout("main", ["ask", "parent"]), checkout("wt", ["child"])] } as unknown as Workspace];
    const listed = [agent("ask", { group: "needs_you" }), agent("parent"), agent("child")].map((row) => ({ agent: row, device: null }));
    const rows = projectRows(workspaces, {}, listed);
    const numbered = new Map([[1, "ask"], [2, "parent"], [3, "child"]] as const);
    const numberOfPane = projectListNumbers(numbered, rows, workspaces);
    // Raised: the number sits there, and not again on its tree row.
    expect(numberOfPane("ask", null)).toBe(1);
    expect(numberOfPane("ask", "main")).toBeNull();
    // Not raised: only the checkout that owns the pane shows it, so a child
    // drawn under its parent in another checkout carries none there.
    expect(numberOfPane("parent", null)).toBeNull();
    expect(numberOfPane("parent", "main")).toBe(2);
    expect(numberOfPane("child", "main")).toBeNull();
    expect(numberOfPane("child", "wt")).toBe(3);
  });

  it("leaves a raised agent folded past its section's cap numbered on its tree row until the section opens", () => {
    const panes = ["d1", "d2", "d3", "d4"];
    const workspaces = [{ agent_scope: emptyScope(), id: "w", device_id: "local", pinned: false, checkouts: [{ id: "main", tabs: [{ id: "t", panes: panes.map((id) => ({ id })) }] }] } as unknown as Workspace];
    const listed = panes.map((pane) => ({ agent: agent(pane, { group: "done" }), device: null }));
    const numbered = new Map([[4, "d4"]] as const);
    const folded = projectListNumbers(numbered, projectRows(workspaces, {}, listed), workspaces);
    expect(folded("d4", null)).toBeNull();
    expect(folded("d4", "main")).toBe(4);
    const opened = projectListNumbers(numbered, projectRows(workspaces, {}, listed, null, ["done"]), workspaces);
    expect(opened("d4", null)).toBe(4);
    expect(opened("d4", "main")).toBeNull();
  });
});


it("numbers a plain-folder row even when its project expansion is false", () => {
  // Literal core scope membership, independent of the retired test projector.
  const parent = agent("folder-pane");
  // The core lists every root in both: `rows` numbers them, `visible_rows` draws them with any opened children.
  const root = {pane_id: parent.pane_id, occurrence: 0, depth: 0, descendants: 0};
  const scope = { ...emptyScope(), sidebar_tree: { ...emptyScope().sidebar_tree, rows: [root], visible_rows: [root] } };
  const folder = { ...checkoutWith(["folder-tab"]), id: "folder-checkout", agent_scope: scope };
  const project = { id: "folder", device_id: "local", is_git: false, expanded: false, pinned: false, checkouts: [folder], agent_scope: scope, inactive_checkouts: {expanded: false, checkout_ids: []} } as unknown as Workspace;
  const rest = { navigator: {focused_device_id: "local", workspaces: [project], devices: [{id: "local", kind: "local", agent_scope: scope}]}, status: {remote: []} } as unknown as SnapshotRest;
  expect([...numberedAgents(agentListOrder({rest, agents: [parent]}))]).toEqual([[1, "folder-pane"]]);
  expect(numberedTarget("agents", 1, {rest, agents: [parent]})).toBe("folder-pane");
});

it("numbers a project's checkouts in the order the sidebar draws them, with no group of its own for what Hide AI made", () => {
  const rows = ["main", "hide", "feature"].map((id) => agent(`${id}-pane`));
  const checkout = (id: string, row: AgentRow) => {
    const root = { pane_id: row.pane_id, occurrence: 0, depth: 0, descendants: 0 };
    const scope = { ...emptyScope(), sidebar_tree: { ...emptyScope().sidebar_tree, rows: [root], visible_rows: [root] } };
    return { ...checkoutWith([`${id}-tab`]), id, path: `/repo/${id}`, exists: true, is_primary: id === "main", agent_scope: scope };
  };
  const checkouts = ["main", "hide", "feature"].map((id, index) => checkout(id, rows[index]!));
  const project = { id: "p", device_id: "local", is_git: true, expanded: true, pinned: false, checkouts, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] } } as unknown as Workspace;
  const rest = { navigator: { focused_device_id: "local", workspaces: [project], devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], hide_ai_checkouts: [{ device_id: "local", path: "/repo/hide", task: null }] }, status: { remote: [] } } as unknown as SnapshotRest;
  expect([...numberedAgents(agentListOrder({ rest, agents: rows }))]).toEqual([[1, "main-pane"], [2, "hide-pane"], [3, "feature-pane"]]);
});

describe("⌥n over the 태스크 list (sidebar-task-lens)", () => {
  // The core's cards in its order; the list regroups them by state, puts a waiting task after the working ones and folds the resting one.
  const panes = ["works", "ask", "waits", "rest", "parent", "child"];
  const rows = [agent("works"), agent("ask", { group: "needs_you" }), agent("waits"), agent("rest"), agent("parent", { lineage_child_pane_ids: ["child"] }), agent("child", { delegated: true, lineage_parent_pane_id: "parent" })];
  const card = (root: string, state: TaskCardScope["state"], members = [root]): TaskCardScope => ({ root, state, members, checkouts: ["c1"], recency: "" });
  const cards = [card("works", "working"), card("ask", "needs_you"), card("waits", "waiting"), card("rest", "resting"), card("parent", "working", ["parent", "child"])];
  const checkout = { ...checkoutWith(["t1"]), tabs: [{ id: "t1", label: "t1", panes: panes.map((id) => ({ id })) }] };
  // The legacy projector rebuilds every device scope, so the device's tasks go on after it, the way the core sends them.
  const rest = (lens: "projects" | "tasks") => {
    const built = legacyRest({
      navigator: { focused_device_id: "local", devices: [{ id: "local", kind: "local", agent_scope: emptyScope() }], workspaces: [{ id: "w1", device_id: "local", expanded: true, checkouts: [checkout] }] },
      status: { remote: [] },
      ui_state: { sidebar_lens: lens, task_view: "status" },
    } as unknown as SnapshotRest, rows);
    built.navigator!.devices![0]!.agent_scope = { ...built.navigator!.devices![0]!.agent_scope!, tasks: { cards, needs_you: 1, recency: "" } };
    return built;
  };

  it("numbers the task rows as the list draws them, a child never, and an opened fold's rows after the groups", () => {
    const state = { rest: rest("tasks"), agents: rows };
    expect(taskListOrder(state, [])).toEqual(["ask", "works", "parent", "waits"]);
    expect(taskListOrder(state, ["rest:"])).toEqual(["ask", "works", "parent", "waits", "rest"]);
    expect(numberedTarget("agents", 1, state)).toBe("ask");
    expect(numberedTarget("agents", 4, state)).toBe("waits");
    expect(numberedTarget("agents", 5, state)).toBeNull();
    expect([...sidebarNumbers(state, { raisedOpen: [], taskFoldsOpen: ["rest:"] }).values()]).toEqual(["ask", "works", "parent", "waits", "rest"]);
  });

  it("follows the switch: on 프로젝트 the numbers are the projects tree's", () => {
    const state = { rest: rest("projects"), agents: rows };
    expect([...sidebarNumbers(state, { raisedOpen: [], taskFoldsOpen: ["rest:"] }).values()]).toEqual([...numberedAgents(agentListOrder(state)).values()]);
    expect([...sidebarNumbers(state, { raisedOpen: [], taskFoldsOpen: [] }).values()]).not.toEqual(["ask", "works", "parent", "waits"]);
  });
});
