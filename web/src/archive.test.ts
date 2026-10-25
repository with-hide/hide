import { describe, expect, it } from "vitest";
import { archivedChildrenOf, archivedRows, archiveOffer, archiveWorking } from "./archive";
import { paneMenuItems } from "./PaneRelations";
import type { AgentRow, ArchivedMember, ArchivedSession, PaneRow, SnapshotRest, Workspace } from "./snapshot";
import { taskList, taskSections, taskFoldKey } from "./taskList";

const LOCAL = "dev-local";
const REMOTE = "dev-remote";

const rest = (over: Partial<NonNullable<SnapshotRest["navigator"]>> = {}, remote: "connected" | "disconnected" = "connected"): SnapshotRest =>
  ({
    navigator: { devices: [{ id: LOCAL, kind: "local" }, { id: REMOTE, kind: "remote" }], ...over },
    status: { remote: [{ target_id: REMOTE, state: remote }] },
  }) as unknown as SnapshotRest;

const agent = (pane: string, over: Partial<AgentRow> = {}): AgentRow =>
  ({ pane_id: pane, session_id: `s-${pane}`, device_id: LOCAL, agent_kind: "claude", identity_label: pane, state: { subtree: "idle" }, ...over }) as unknown as AgentRow;

const member = (session: string, over: Partial<ArchivedMember> = {}): ArchivedMember => ({
  session: { device_id: LOCAL, agent: "claude", session_id: session },
  agent_kind: "claude",
  title: session,
  depth: 0,
  parent_session_id: null,
  last_line: null,
  pulls: [],
  resume: "archived",
  reason: null,
  ...over,
});

const archived = (id: string, over: Partial<ArchivedSession> = {}): ArchivedSession => ({
  id,
  device_id: LOCAL,
  project_root: "/repo",
  project_label: "repo",
  checkout_path: "/repo",
  phase: "archived",
  listed: true,
  archived_at_unix_ms: 1,
  reason: null,
  members: [member(`${id}-root`)],
  ...over,
});

describe("where 세션 보관 is offered", () => {
  it("offers it on a root whose conversation Hide knows", () => {
    expect(archiveOffer(rest(), agent("p1"))).toEqual({ kind: "offered", unavailable: null });
  });

  it("offers nothing on Hide AI", () => {
    expect(archiveOffer(rest(), agent("p1", { hide_ai: { kind: "secretary" } } as unknown as Partial<AgentRow>))).toEqual({ kind: "hidden" });
  });

  it("draws it disabled with why on a child, an unknown conversation and a gone device", () => {
    expect(archiveOffer(rest(), agent("p2", { lineage_parent_pane_id: "p1" }))).toMatchObject({ kind: "offered", unavailable: expect.any(String) });
    expect(archiveOffer(rest(), agent("p1", { session_id: null } as unknown as Partial<AgentRow>))).toMatchObject({ unavailable: expect.any(String) });
    expect(archiveOffer(rest({}, "disconnected"), agent("p1", { device_id: REMOTE }))).toMatchObject({ unavailable: expect.any(String) });
    expect(archiveOffer(rest({}, "connected"), agent("p1", { device_id: REMOTE }))).toEqual({ kind: "offered", unavailable: null });
  });

  it("puts it below Sleep agent in the pane menu, or first when the agent cannot sleep", () => {
    const offered = { kind: "offered", unavailable: null } as const;
    const ids = (row: object) => paneMenuItems({ id: "w1:p1", children: null, lineage_path: [], ...row } as unknown as PaneRow, "reviewer", offered).map((item) => item.id);
    expect(ids({ sleep_action: { available: true }, fork: { available: true } })).toEqual(["sleep_agent", "archive_session", "fork_agent", "fork_agent_side", "copy_reference", "close_pane"]);
    expect(ids({ fork: { available: true } })).toEqual(["fork_agent", "fork_agent_side", "archive_session", "copy_reference", "close_pane"]);
  });
});

describe("the archive confirmation", () => {
  it("lists the root and the descendants that are working now, and nothing quiet", () => {
    const root = agent("p1", { close_descendant_pane_ids: ["p2", "p3"], state: { subtree: "idle" } } as unknown as Partial<AgentRow>);
    const byPane = new Map([root, agent("p2", { state: { subtree: "working" } } as unknown as Partial<AgentRow>), agent("p3")].map((row) => [row.pane_id, row]));
    expect(archiveWorking(root, byPane).map((row) => row.pane_id)).toEqual(["p2"]);
  });
});

describe("보관됨", () => {
  const project = { id: "w1", device_id: LOCAL, path: "/repo", label: "repo", checkouts: [] } as unknown as Workspace;

  it("lists only this device's rows that stand in the fold", () => {
    const rows = [archived("a1"), archived("a2", { listed: false }), archived("a3", { device_id: REMOTE })];
    expect(archivedRows(rest({ archived_sessions: rows }), LOCAL).map((row) => row.id)).toEqual(["a1"]);
  });

  it("keeps a project whose sessions are all archived, its fold closed until opened", () => {
    const list = taskList({ projects: [] } as never, new Map(), [project], "project", [archived("a1")]);
    expect(list.groups.map((group) => [group.project?.id, group.archived.map((entry) => entry.row.id)])).toEqual([["w1", ["a1"]]]);
    const key = list.groups[0]!.key;
    expect(taskSections(list, "project", new Set()).sections[0]!.archived).toEqual([]);
    expect(taskSections(list, "project", new Set([taskFoldKey("archived", key)])).sections[0]!.archived.map((entry) => entry.row.id)).toEqual(["a1"]);
  });

  it("stands at the very bottom in 상태별", () => {
    const list = taskList({ projects: [] } as never, new Map(), [project], "status", [archived("a1")]);
    expect(list.archived.map((entry) => entry.row.id)).toEqual(["a1"]);
    expect(taskSections(list, "status", new Set([taskFoldKey("archived", "")])).archived).toHaveLength(1);
  });
});

describe("a live root's record after a partial close", () => {
  it("shows the members that closed, and nothing once they resumed", () => {
    const row = archived("a1", { listed: false, phase: "close_failed", members: [member("root"), member("c1", { depth: 1 }), member("c2", { depth: 1, resume: "resumed" })] });
    const found = archivedChildrenOf(rest({ archived_sessions: [row] }), { device_id: LOCAL, agent: "claude", session_id: "root" });
    expect(found?.members.map((one) => one.session.session_id)).toEqual(["c1"]);
    expect(archivedChildrenOf(rest({ archived_sessions: [archived("a2")] }), { device_id: LOCAL, agent: "claude", session_id: "a2-root" })).toBeNull();
  });
});
