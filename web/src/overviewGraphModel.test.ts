// Overview redesign B3-B8: session identity, verified lineage and bounded rendering.
import { describe, expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { graphProjectId, initialGraphExpansion, overviewGraphFamily, overviewGraphModel, overviewProjectGraphCounts } from "./overviewGraphModel";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, Checkout, OverviewGraphMetadata, OverviewGraphParty, Workspace } from "./snapshot";

function agent(pane: string, extra: Partial<AgentRow> = {}) {
  return legacyAgentRow({ id: pane, pane_id: pane, identity_label: pane, agent_kind: "claude", session_id: `native-${pane}`, group: "working", activity: "working", ...extra });
}
function project(id: string, agents: AgentRow[], device = "local"): BoardProject {
  const checkout: Checkout = { id: `${id}-main`, workspace_id: id, label: "main", path: `/fixture/${id}`, branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: agents.length > 0, pull_request: null, tabs: [], active_tab_id: null, next_tab_label: "Tab 1", strip: [], agent_scope: emptyScope() };
  const workspace: Workspace = { id, label: id, device_id: device, path: `/fixture/${id}`, registered: true, temporary: false, pinned: false, checkouts: [checkout], inactive_checkouts: { expanded: false, checkout_ids: [] }, agent_scope: { ...emptyScope(), members: agents.map((row) => ({ pane_id: row.pane_id, project_id: id, checkout_id: checkout.id })) } };
  return { workspace, agents, device: device === "local" ? null : "same display name" };
}
const party = (pane: string, session = `digest-${pane}`, device = "local"): OverviewGraphParty => ({ device_id: device, pane_id: pane, session });
const letter = (id: string, sender: OverviewGraphParty, recipient: OverviewGraphParty, kind = "request", at = 10) => ({ id, sender, recipient, kind, at_unix_ms: at });
function metadata(parties: OverviewGraphParty[], letters: OverviewGraphMetadata["letters"] = []): OverviewGraphMetadata {
  return { available: true, limit: 2000, retained_count: letters.length, truncated: false, retention_ms: 86400000, letters, sessions: parties.map((row) => ({ device_id: row.device_id, pane_id: row.pane_id, session_digest: row.session, launch_model: null })) };
}

describe("current sessions in the Overview graph", () => {
  it("keeps Herdr input count and direction separate from letters and delegation", () => {
    const p = project("one", [agent("a"), agent("b")]);
    const facts = metadata([party("a"), party("b")], [letter("mail", party("a"), party("b"))]);
    facts.direct_inputs = [
      { id: "direct-1", sender: party("a"), recipient: party("b"), at_unix_ms: 20 },
      { id: "direct-2", sender: party("a"), recipient: party("b"), at_unix_ms: 30 },
      { id: "direct-3", sender: party("b"), recipient: party("a"), at_unix_ms: 40 },
      { id: "old-session", sender: party("a", "previous-digest"), recipient: party("b"), at_unix_ms: 50 },
      { id: "other-server-device", sender: party("a", "digest-a", "other-server-device"), recipient: party("b"), at_unix_ms: 60 },
    ];
    const graph = overviewGraphModel([p], facts, new Set([graphProjectId(p)]), null);
    const direct = graph.communication.find((row) => row.kind === "herdr_direct")!;
    expect(direct.count).toBe(3);
    expect(direct.latest).toBe(40);
    expect(direct.directions.map((row) => row.count).sort()).toEqual([1, 2]);
    expect(direct.kinds).toEqual({ herdr_direct: 3 });
    expect(graph.communication.find((row) => row.kind === "letters")?.count).toBe(1);
    expect(graph.lineage).toEqual([]);
    expect(overviewGraphModel([p], { ...facts, direct_inputs: [] }, new Set([graphProjectId(p)]), null).structure).toBe(graph.structure);
    expect(overviewGraphModel([p], { ...facts, sessions: [] }, new Set([graphProjectId(p)]), null).communication).toEqual([]);
    const withoutLetters = overviewGraphModel([p], { ...facts, available: false }, new Set([graphProjectId(p)]), null);
    expect(withoutLetters.communication.map((row) => row.kind)).toEqual(["herdr_direct"]);
  });

  it("shares one newest-record cap across letters and direct inputs", () => {
    const p = project("one", [agent("a"), agent("b")]);
    const facts = metadata([party("a"), party("b")], Array.from({ length: 2000 }, (_, index) => letter(`letter-${index}`, party("a"), party("b"), "request", index)));
    facts.direct_inputs = [{ id: "direct", sender: party("a"), recipient: party("b"), at_unix_ms: 2000 }];
    const graph = overviewGraphModel([p], facts, new Set([graphProjectId(p)]), null);
    expect(graph.communication.reduce((count, row) => count + row.count, 0)).toBe(2000);
    expect(graph.communication.find((row) => row.kind === "herdr_direct")?.count).toBe(1);
    expect(graph.communication.find((row) => row.kind === "letters")?.count).toBe(1999);
  });

  it("keeps core-verified lineage in both cross-device directions and counts descendants by current session", () => {
    const local = project("local", [agent("local-parent"), agent("local-child", { lineage_parent_pane_id: "remote:mini:pane:parent" }), agent("local-grandchild", { lineage_parent_pane_id: "remote:mini:pane:child" })]);
    const remote = project("remote", [agent("remote:mini:pane:parent", { device_id: "mini" }), agent("remote:mini:pane:child", { device_id: "mini", lineage_parent_pane_id: "local-parent" }), agent("remote:mini:pane:fork", { device_id: "mini", forked_from: { pane_id: "local-parent", label: "source" } })], "mini");
    const graph = overviewGraphModel([local, remote], undefined, new Set([graphProjectId(local), graphProjectId(remote)]), null);
    const edges = graph.lineage.map((edge) => [graph.agents.find((row) => row.id === edge.from)!.agent.pane_id, graph.agents.find((row) => row.id === edge.to)!.agent.pane_id]);
    expect(edges).toEqual(expect.arrayContaining([["local-parent", "remote:mini:pane:child"], ["remote:mini:pane:parent", "local-child"], ["remote:mini:pane:child", "local-grandchild"]]));
    expect(edges).toHaveLength(3);
    const family = (pane: string) => overviewGraphFamily(graph.agents.find((row) => row.agent.pane_id === pane)!, graph.agents);
    expect(family("local-parent")).toEqual({ direct: 1, descendants: 2 });
    expect(family("remote:mini:pane:parent")).toEqual({ direct: 1, descendants: 1 });
    expect(family("remote:mini:pane:child")).toEqual({ direct: 1, descendants: 1 });
  });

  it.each(["another-device", "duplicate-native-sessions"])("rejects ambiguous plain parents: %s", (ambiguity) => {
    const first = project("first", [agent("parent")]);
    const second = project("second", [agent("parent", { device_id: ambiguity === "another-device" ? "mini" : "local", session_id: "other-native-session" })], ambiguity === "another-device" ? "mini" : "local");
    const child = project("child", [agent("child", { device_id: ambiguity === "another-device" ? "third" : "local", lineage_parent_pane_id: "parent" })], ambiguity === "another-device" ? "third" : "local");
    const graph = overviewGraphModel([first, second, child], undefined, new Set([first, second, child].map(graphProjectId)), null);
    expect(graph.lineage).toEqual([]);
    for (const row of graph.agents) expect(overviewGraphFamily(row, graph.agents)).toEqual({ direct: 0, descendants: 0 });
  });

  it("summarizes direct children and all current descendants without forks or another device's reused pane", () => {
    const p = project("family", [agent("parent"), ...["a", "b", "c", "d"].map((pane) => agent(pane, { lineage_parent_pane_id: "parent" })), agent("grandchild", { lineage_parent_pane_id: "a" }), agent("fork", { forked_from: { pane_id: "parent", label: "source" } })]);
    const other = project("remote", [agent("parent", { device_id: "remote" }), agent("remote-child", { device_id: "remote", lineage_parent_pane_id: "parent" })], "remote");
    const graph = overviewGraphModel([p, other], undefined, new Set([graphProjectId(p), graphProjectId(other)]), null);
    const parent = graph.agents.find((row) => row.agent.pane_id === "parent" && row.deviceId === "local")!;
    expect(overviewGraphFamily(parent, graph.agents)).toEqual({ direct: 4, descendants: 5 });
    expect(overviewGraphFamily(graph.agents.find((row) => row.agent.pane_id === "a")!, graph.agents)).toEqual({ direct: 1, descendants: 1 });
    expect(graph.lineage.filter((edge) => edge.from === parent.id)).toHaveLength(4);
  });

  it("keeps multiple roots and uses only proven parents, never a fork origin or registration outside the core scope", () => {
    const p = project("one", [agent("a"), agent("b", { lineage_parent_pane_id: "a" }), agent("fork", { forked_from: { pane_id: "a", label: "source" } })]);
    p.agents.push(agent("historical"));
    const graph = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null);
    expect(graph.agents.map((row) => row.agent.pane_id).sort()).toEqual(["a", "b", "fork"]);
    expect(graph.projects[0]?.roots).toBe(2);
    expect(overviewProjectGraphCounts([p], undefined).get(graphProjectId(p))).toEqual({ total: 3, roots: 2, children: 1 });
    expect(graph.lineage.map((edge) => [graph.agents.find((row) => row.id === edge.from)?.agent.pane_id, graph.agents.find((row) => row.id === edge.to)?.agent.pane_id])).toEqual([["a", "b"]]);
  });

  it("does not attach past correspondence to a reused pane, a different device or an unproven native session", () => {
    const a = project("one", [agent("same")]);
    const b = project("two", [agent("other")]);
    const facts = metadata([party("same", "new"), party("other")], [letter("old", party("same", "old"), party("other")), letter("wrong-device", party("same", "new", "remote"), party("other")), letter("current", party("same", "new"), party("other"))]);
    const graph = overviewGraphModel([a, b], facts, new Set(), null);
    expect(graph.communication.map((row) => row.count)).toEqual([1]);
    expect(overviewGraphModel([a, b], { ...facts, sessions: [] }, new Set(), null).communication).toEqual([]);
  });

  it("keeps folded endpoint kinds and directions, then reveals any selected session without moving representatives", () => {
    const a = project("one", Array.from({ length: 10 }, (_, i) => agent(`a${i}`)));
    const b = project("two", [agent("b")]);
    const facts = metadata([...a.agents.map((row) => party(row.pane_id)), party("b")], [letter("1", party("a8"), party("b")), letter("2", party("a9"), party("b"), "progress", 20), letter("3", party("b"), party("a8"), "report", 30)]);
    const folded = overviewGraphModel([a, b], facts, new Set(), null);
    expect(folded.communication).toHaveLength(1);
    expect(folded.communication[0]).toMatchObject({ count: 3, latest: 30, kinds: { request: 1, progress: 1, report: 1 } });
    expect(folded.communication[0]?.directions.map((row) => row.count).sort()).toEqual([1, 2]);
    const selected = folded.agents.find((row) => row.agent.pane_id === "a8")!.id;
    const revealed = overviewGraphModel([a, b], facts, new Set(), null, { focus: "all", includeResting: false, selected });
    expect(revealed.visible.some((row) => row.id === selected)).toBe(true);
    expect(revealed.projects[0]?.shown.map((row) => row.id)).toEqual(folded.projects[0]?.shown.map((row) => row.id));
    expect(revealed.structure).toBe(folded.structure);
    expect(revealed.communication).toHaveLength(2);
  });

  it("keeps topology stable when status, labels, correspondence or activity order refresh", () => {
    const first = project("one", [agent("b"), agent("a")]);
    const second = project("two", [agent("c")]);
    const expanded = new Set([graphProjectId(first)]);
    const before = overviewGraphModel([first, second], metadata([party("a"), party("b"), party("c")]), expanded, null, { focus: "all", includeResting: true });
    const changed = project("one", [agent("a", { identity_label: "새 상태", group: "needs_you", demand: "question" }), agent("b", { group: "seen" })]);
    const after = overviewGraphModel([second, changed], metadata([party("a"), party("b"), party("c")], [letter("new", party("a"), party("c"))]), expanded, null, { focus: "all", includeResting: true });
    expect(after.structure).toBe(before.structure);
    expect(after.visible.find((row) => row.agent.pane_id === "a")?.bucket).toBe("turn");
  });

  it("counts project groups against the cap and retains every session in the fallback list", () => {
    const p = project("large", Array.from({ length: 400 }, (_, i) => agent(`p${i}`)));
    const graph = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null);
    expect(graph.limited).toBe(true);
    // B8 retains representative sessions when an oversized expansion is refused.
    expect(graph.visible).toHaveLength(6);
    expect(graph.projects[0]?.expanded).toBe(false);
    expect(graph.agents).toHaveLength(400);
    const many = Array.from({ length: 401 }, (_, i) => project(`project${i}`, [agent(`pane${i}`)]));
    const bounded = overviewGraphModel(many, undefined, new Set(), null);
    expect(bounded.projects).toHaveLength(400);
    expect(bounded.agents).toHaveLength(401);
    expect(bounded.limited).toBe(true);
    const counts = overviewProjectGraphCounts(many, undefined);
    expect(counts.size).toBe(401);
    expect(counts.get(graphProjectId(many[400]!))).toEqual({ total: 1, roots: 1, children: 0 });
  });

  it("protects the entry session before applying the 400-node graph budget", () => {
    const projects = Array.from({ length: 401 }, (_, i) => project(`project${i}`, [agent(`pane${i}`)]));
    const all = overviewGraphModel(projects, undefined, new Set(), null);
    const current = all.allAgents.find((row) => row.agent.pane_id === "pane400")!;
    const protectedGraph = overviewGraphModel(projects, undefined, new Set(), null, { focus: "all", includeResting: false, pinned: [current.id] });
    expect(protectedGraph.visible.map((row) => row.id)).toContain(current.id);
    expect(protectedGraph.visible.length + protectedGraph.projects.length).toBeLessThanOrEqual(400);
    expect(protectedGraph.agents).toHaveLength(401);
  });

  it("opens the chosen project, then the recent checkout, without using device display labels as identity", () => {
    const a = project("one", [agent("a")]);
    const b = project("two", [agent("b")]);
    expect([...initialGraphExpansion([a, b], "one", ["two-main"])]).toEqual([graphProjectId(a)]);
    expect([...initialGraphExpansion([a, b], null, ["two-main"])]).toEqual([graphProjectId(b)]);
    expect(graphProjectId(project("one", [], "remote-1"))).not.toBe(graphProjectId(project("one", [], "remote-2")));
  });
});

// Overview focus B2-B8, B17-B18: projection uses authoritative marks, not bucket.
describe("Overview focus", () => {
  const panes = (model: ReturnType<typeof overviewGraphModel>) => model.agents.map((row) => row.agent.pane_id);
  it("keeps idle sessions, Seen demands, sleeping asks and unknown, with operator-first list order", () => {
    const p = project("focus", [agent("rest", { mark: "quiet" }), agent("sleep", { mark: "asleep" }), agent("unknown", { mark: "unknown" }), agent("result", { mark: "unread" }), agent("worker", { mark: "waiting" }), agent("seen-ask", { mark: "turn", group: "seen" }), agent("sleep-ask", { mark: "turn" })]);
    const all = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null);
    expect(panes(all)).toEqual(["seen-ask", "sleep-ask", "worker", "result", "unknown", "rest"]);
    expect(all.counts).toEqual({ all: 6, turn: 2, working: 1, results: 1 });
    for (const [focus, expected] of [["turn", ["seen-ask", "sleep-ask"]], ["working", ["worker"]], ["results", ["result"]]] as const) {
      expect(panes(overviewGraphModel([p], undefined, new Set(), null, { focus, includeResting: false }))).toEqual(expected);
    }
    expect(panes(overviewGraphModel([p], undefined, new Set(), null, { focus: "all", includeResting: true })).slice(-2)).toEqual(["rest", "sleep"]);
  });
  it("retains resting ancestors as context without increasing matching counts", () => {
    const p = project("family", [agent("root", { mark: "asleep" }), agent("middle", { mark: "quiet", lineage_parent_pane_id: "root" }), agent("child", { mark: "going", lineage_parent_pane_id: "middle" })]);
    const graph = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null, { focus: "working", includeResting: false });
    expect(panes(graph)).toEqual(["child"]);
    expect(graph.visible.map((row) => row.agent.pane_id).sort()).toEqual(["child", "middle", "root"]);
    expect(graph.context.size).toBe(2);
    expect(graph.projects[0]?.matches).toBe(1);
    expect(graph.lineage).toHaveLength(2);
    expect(graph.quietBranches.size).toBe(0);
  });
  it("counts collapsed families from the filtered projection without changing Projects totals", () => {
    const p = project("several", [agent("first"), agent("ask", { mark: "turn" }), agent("rest", { mark: "quiet" })]);
    const graph = overviewGraphModel([p], undefined, new Set(), null, { focus: "turn", includeResting: false });
    expect(graph.projects[0]).toMatchObject({ roots: 1, matches: 1, expanded: false });
    expect(graph.projects[0]?.agents).toHaveLength(1);
    expect(overviewProjectGraphCounts([p], undefined).get(graphProjectId(p))).toEqual({ total: 3, roots: 3, children: 0 });
  });
  it("does not spend graph capacity on empty or sleeping-only projects", () => {
    const projects = Array.from({ length: 450 }, (_, i) => project(`empty${i}`, []));
    projects.push(project("rest", [agent("rest", { mark: "asleep" })]));
    const active = project("active", [agent("active")]);
    projects.push(active);
    const graph = overviewGraphModel(projects, undefined, new Set([graphProjectId(active)]), null);
    expect(graph.projects).toHaveLength(1);
    expect(graph.visible).toHaveLength(1);
    expect(graph.omitted).toHaveLength(451);
    expect(graph.limited).toBe(false);
  });
  it("offers folds only for completely sleeping descendant branches, and releases a fold when a child becomes idle", () => {
    const rows = [agent("root"), agent("quiet", { mark: "asleep", lineage_parent_pane_id: "root" }), agent("leaf", { mark: "asleep", lineage_parent_pane_id: "quiet" })];
    const p = project("family", rows);
    const options = { focus: "all" as const, includeResting: true };
    const before = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null, options);
    const root = before.agents.find((row) => row.agent.pane_id === "root")!.id;
    expect(before.quietBranches.get(root)).toBe(2);
    const folded = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null, { ...options, folded: new Set([root]) });
    expect(folded.visible.map((row) => row.agent.pane_id)).toEqual(["root"]);
    expect(folded.agents).toHaveLength(3);
    const changed = project("family", rows.map((row) => row.pane_id === "leaf" ? { ...row, mark: "quiet" } : row));
    const active = overviewGraphModel([changed], undefined, new Set([graphProjectId(changed)]), null, { ...options, folded: new Set([root]) });
    expect(active.folded.size).toBe(0);
    expect(active.visible).toHaveLength(3);
  });
  it("shows idle-only projects by default and restores sleeping sessions on request", () => {
    const idle = project("idle", [agent("idle", { mark: "quiet" })]);
    const sleeping = project("sleeping", [agent("sleeping", { mark: "asleep" })]);
    const projects = [sleeping, idle];
    const expanded = new Set(projects.map(graphProjectId));
    const graph = overviewGraphModel(projects, undefined, expanded, null);
    expect(graph.projects.map((group) => group.project.workspace.id)).toEqual(["idle"]);
    expect(panes(graph)).toEqual(["idle"]);
    expect(graph.visible.map((row) => row.agent.pane_id)).toEqual(["idle"]);
    expect(graph.omitted.map((group) => group.project.workspace.id)).toEqual(["sleeping"]);
    expect([...initialGraphExpansion(projects, null)]).toEqual([graphProjectId(idle)]);
    expect(panes(overviewGraphModel(projects, undefined, expanded, null, { focus: "all", includeResting: true }))).toEqual(["idle", "sleeping"]);
  });
  it("distinguishes empty, resting-only and filtered projects and recovers without mutating input", () => {
    const projects = [project("empty", []), project("rest", [agent("sleep", { mark: "asleep" })]), project("busy", [agent("busy")])];
    const before = JSON.stringify(projects);
    const graph = overviewGraphModel(projects, undefined, new Set(), null, { focus: "turn", includeResting: false });
    expect(graph.omitted.map((group) => [group.project.workspace.id, group.reason])).toEqual([["empty", "empty"], ["rest", "resting"], ["busy", "filtered"]]);
    expect(graph.projects).toEqual([]);
    expect(overviewGraphModel(projects, undefined, new Set(), null, { focus: "all", includeResting: true }).projects).toHaveLength(2);
    expect(JSON.stringify(projects)).toBe(before);
  });
  it("terminates cyclic lineage and never offers an unsafe cyclic fold", () => {
    const p = project("cycle", [agent("a", { mark: "quiet", lineage_parent_pane_id: "b" }), agent("b", { mark: "quiet", lineage_parent_pane_id: "a" })]);
    const graph = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null, { focus: "all", includeResting: true });
    expect(graph.visible).toHaveLength(2);
    expect(graph.quietBranches.size).toBe(0);
  });
});
