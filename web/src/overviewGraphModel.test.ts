// Overview redesign B3-B8 and PRD overview-main-map B4, B10, B13, B16-B18:
// session identity, verified lineage and teams, pairs of correspondence and
// bounded rendering.
import { describe, expect, it } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { graphProjectId, overviewGraphModel, overviewGraphTeam, overviewProjectGraphCounts } from "./overviewGraphModel";
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
type Model = ReturnType<typeof overviewGraphModel>;
const pane = (model: Model, id: string) => model.allAgents.find((row) => row.id === id)!.agent.pane_id;
const idOf = (model: Model, paneId: string) => model.allAgents.find((row) => row.agent.pane_id === paneId)!.id;
const lineage = (model: Model) => [...model.parents].map(([child, parent]) => [pane(model, parent), pane(model, child)]).sort();
const team = (model: Model, paneId: string) => overviewGraphTeam(model.allAgents.find((row) => row.agent.pane_id === paneId)!, model).map(({ row, depth }) => [row.agent.pane_id, depth]);

describe("current sessions in the Overview graph", () => {
  it("joins letters and Herdr input between the same two sessions into one pair with directions", () => {
    const p = project("one", [agent("a"), agent("b")]);
    const facts = metadata([party("a"), party("b")], [letter("mail", party("a"), party("b"))]);
    facts.direct_inputs = [
      { id: "direct-1", sender: party("a"), recipient: party("b"), at_unix_ms: 20 },
      { id: "direct-2", sender: party("a"), recipient: party("b"), at_unix_ms: 30 },
      { id: "direct-3", sender: party("b"), recipient: party("a"), at_unix_ms: 40 },
      { id: "old-session", sender: party("a", "previous-digest"), recipient: party("b"), at_unix_ms: 50 },
      { id: "other-server-device", sender: party("a", "digest-a", "other-server-device"), recipient: party("b"), at_unix_ms: 60 },
    ];
    const graph = overviewGraphModel([p], facts, null);
    expect(graph.communication).toHaveLength(1);
    const [pair] = graph.communication;
    expect(pair).toMatchObject({ letters: 1, direct: 3, latest: 40 });
    const a = idOf(graph, "a");
    const b = idOf(graph, "b");
    expect(pair!.directions.find((row) => row.from === a)).toEqual({ from: a, to: b, letters: 1, direct: 2 });
    expect(pair!.directions.find((row) => row.from === b)).toEqual({ from: b, to: a, letters: 0, direct: 1 });
    expect(graph.parents.size).toBe(0);
    expect(overviewGraphModel([p], { ...facts, direct_inputs: [] }, null).structure).toBe(graph.structure);
    expect(overviewGraphModel([p], { ...facts, sessions: [] }, null).communication).toEqual([]);
    expect(overviewGraphModel([p], { ...facts, available: false }, null).communication).toMatchObject([{ letters: 0, direct: 3 }]);
    // One direction only: the pair points from its sender.
    const one = overviewGraphModel([p], metadata([party("a"), party("b")], [letter("x", party("b"), party("a"))]), null).communication[0]!;
    expect([one.source, one.target]).toEqual([b, a]);
  });

  it("shares one newest-record cap across letters and direct inputs", () => {
    const p = project("one", [agent("a"), agent("b")]);
    const facts = metadata([party("a"), party("b")], Array.from({ length: 2000 }, (_, index) => letter(`letter-${index}`, party("a"), party("b"), "request", index)));
    facts.direct_inputs = [{ id: "direct", sender: party("a"), recipient: party("b"), at_unix_ms: 2000 }];
    expect(overviewGraphModel([p], facts, null).communication).toMatchObject([{ letters: 1999, direct: 1 }]);
  });

  it("keeps a pair whose session is outside the drawn budget, for the chosen session's counterparts", () => {
    const big = project("big", Array.from({ length: 400 }, (_, i) => agent(`p${String(i).padStart(3, "0")}`)));
    const other = project("other", [agent("o")]);
    const facts = metadata([...big.agents.map((row) => party(row.pane_id)), party("o")], [letter("far", party("p399"), party("o"))]);
    const graph = overviewGraphModel([big, other], facts, null);
    expect(graph.visible.some((row) => row.agent.pane_id === "p399")).toBe(false);
    expect(graph.communication).toMatchObject([{ letters: 1, source: idOf(graph, "p399"), target: idOf(graph, "o") }]);
  });

  it("keeps core-verified lineage in both cross-device directions and lists each team in tree order", () => {
    const local = project("local", [agent("local-parent"), agent("local-child", { lineage_parent_pane_id: "remote:mini:pane:parent" }), agent("local-grandchild", { lineage_parent_pane_id: "remote:mini:pane:child" })]);
    const remote = project("remote", [agent("remote:mini:pane:parent", { device_id: "mini" }), agent("remote:mini:pane:child", { device_id: "mini", lineage_parent_pane_id: "local-parent" }), agent("remote:mini:pane:fork", { device_id: "mini", forked_from: { pane_id: "local-parent", label: "source" } })], "mini");
    const graph = overviewGraphModel([local, remote], undefined, null);
    expect(lineage(graph)).toEqual([["local-parent", "remote:mini:pane:child"], ["remote:mini:pane:child", "local-grandchild"], ["remote:mini:pane:parent", "local-child"]]);
    expect(team(graph, "local-parent")).toEqual([["remote:mini:pane:child", 1], ["local-grandchild", 2]]);
    expect(team(graph, "remote:mini:pane:parent")).toEqual([["local-child", 1]]);
    expect(graph.teams.get(idOf(graph, "local-grandchild"))).toBe(idOf(graph, "local-parent"));
    expect(graph.teams.get(idOf(graph, "remote:mini:pane:fork"))).toBe(idOf(graph, "remote:mini:pane:fork"));
  });

  it.each(["another-device", "duplicate-native-sessions"])("rejects ambiguous plain parents: %s", (ambiguity) => {
    const first = project("first", [agent("parent")]);
    const second = project("second", [agent("parent", { device_id: ambiguity === "another-device" ? "mini" : "local", session_id: "other-native-session" })], ambiguity === "another-device" ? "mini" : "local");
    const child = project("child", [agent("child", { device_id: ambiguity === "another-device" ? "third" : "local", lineage_parent_pane_id: "parent" })], ambiguity === "another-device" ? "third" : "local");
    const graph = overviewGraphModel([first, second, child], undefined, null);
    expect(graph.parents.size).toBe(0);
    for (const row of graph.agents) expect(overviewGraphTeam(row, graph)).toEqual([]);
    expect(graph.roots.size).toBe(3);
  });

  it("lists direct children and all current descendants without forks or another device's reused pane", () => {
    const p = project("family", [agent("parent"), ...["a", "b", "c", "d"].map((pane) => agent(pane, { lineage_parent_pane_id: "parent" })), agent("grandchild", { lineage_parent_pane_id: "a" }), agent("fork", { forked_from: { pane_id: "parent", label: "source" } })]);
    const other = project("remote", [agent("parent", { device_id: "remote" }), agent("remote-child", { device_id: "remote", lineage_parent_pane_id: "parent" })], "remote");
    const graph = overviewGraphModel([p, other], undefined, null);
    const parent = graph.agents.find((row) => row.agent.pane_id === "parent" && row.deviceId === "local")!;
    expect(overviewGraphTeam(parent, graph).map(({ row, depth }) => [row.agent.pane_id, depth])).toEqual([["a", 1], ["grandchild", 2], ["b", 1], ["c", 1], ["d", 1]]);
    // The drawn tree follows the same parents: a grandchild sits outside its own parent.
    expect(graph.layoutParents.get(idOf(graph, "grandchild"))).toBe(idOf(graph, "a"));
    expect(graph.layoutParents.get(parent.id)).toBeNull();
  });

  it("keeps multiple roots and uses only proven parents, never a fork origin or registration outside the core scope", () => {
    const p = project("one", [agent("a"), agent("b", { lineage_parent_pane_id: "a" }), agent("fork", { forked_from: { pane_id: "a", label: "source" } })]);
    p.agents.push(agent("historical"));
    const graph = overviewGraphModel([p], undefined, null);
    expect(graph.agents.map((row) => row.agent.pane_id).sort()).toEqual(["a", "b", "fork"]);
    expect(graph.projects[0]?.roots).toBe(2);
    expect(overviewProjectGraphCounts([p], undefined).get(graphProjectId(p))).toEqual({ total: 3, roots: 2, children: 1 });
    expect(lineage(graph)).toEqual([["a", "b"]]);
  });

  it("does not attach past correspondence to a reused pane, a different device or an unproven native session", () => {
    const a = project("one", [agent("same")]);
    const b = project("two", [agent("other")]);
    const facts = metadata([party("same", "new"), party("other")], [letter("old", party("same", "old"), party("other")), letter("wrong-device", party("same", "new", "remote"), party("other")), letter("current", party("same", "new"), party("other"))]);
    expect(overviewGraphModel([a, b], facts, null).communication.map((row) => row.letters)).toEqual([1]);
    expect(overviewGraphModel([a, b], { ...facts, sessions: [] }, null).communication).toEqual([]);
    expect(graphProjectId(project("one", [], "remote-1"))).not.toBe(graphProjectId(project("one", [], "remote-2")));
  });

  it("keeps topology stable when status, labels, correspondence, filters or activity order refresh", () => {
    const first = project("one", [agent("b"), agent("a")]);
    const second = project("two", [agent("c", { mark: "turn" })]);
    const before = overviewGraphModel([first, second], metadata([party("a"), party("b"), party("c")]), null, { focus: "all", includeResting: true });
    const changed = project("one", [agent("a", { identity_label: "새 상태", group: "needs_you", demand: "question", mark: "turn" }), agent("b", { group: "seen", mark: "quiet" })]);
    const after = overviewGraphModel([second, changed], metadata([party("a"), party("b"), party("c")], [letter("new", party("a"), party("c"))]), null, { focus: "all", includeResting: true });
    expect(after.structure).toBe(before.structure);
    expect(after.visible.find((row) => row.agent.pane_id === "a")?.agent.mark).toBe("turn");
    // B10: a filter dims sessions; it never removes them from the map.
    const filtered = overviewGraphModel([first, second], metadata([party("a"), party("b"), party("c")]), null, { focus: "turn", includeResting: true });
    expect(filtered.structure).toBe(before.structure);
    expect(filtered.agents.map((row) => row.agent.pane_id)).toEqual(["c"]);
    expect([...filtered.context].map((id) => pane(filtered, id)).sort()).toEqual(["a", "b"]);
  });

  it("summarizes each project's graph sessions by state, including sessions the budget does not draw", () => {
    const p = project("mixed", [agent("ask", { mark: "turn" }), agent("work", { mark: "going" }), agent("wait", { mark: "waiting" }), agent("read", { mark: "unread" }), agent("done", { mark: "done" }), agent("idle", { mark: "quiet" }), agent("sleep", { mark: "asleep" })]);
    const graph = overviewGraphModel([p], undefined, null);
    expect(graph.summaries).toMatchObject([{ id: graphProjectId(p), sessions: 6, turn: 1, working: 2, results: 2, admitted: true }]);
    expect(overviewGraphModel([p], undefined, null, { focus: "all", includeResting: true }).summaries[0]?.sessions).toBe(7);
  });

  it("counts project names against the cap and retains every session in the fallback list", () => {
    const p = project("large", Array.from({ length: 400 }, (_, i) => agent(`p${i}`)));
    const graph = overviewGraphModel([p], undefined, null);
    expect(graph.limited).toBe(true);
    // B18 retains representative sessions and an access slot when a project is too large.
    expect(graph.visible).toHaveLength(6);
    expect(graph.projects[0]?.revealSlot).not.toBeNull();
    expect(graph.agents).toHaveLength(400);
    const many = Array.from({ length: 401 }, (_, i) => project(`project${String(i).padStart(3, "0")}`, [agent(`pane${i}`)]));
    const bounded = overviewGraphModel(many, undefined, null);
    // Every admitted name retains one member position within 400 nodes.
    expect(bounded.projects).toHaveLength(200);
    expect(bounded.projects.length + bounded.visible.length).toBe(400);
    expect(bounded.agents).toHaveLength(401);
    expect(bounded.limited).toBe(true);
    expect(bounded.summaries.filter((row) => !row.admitted)).toHaveLength(201);
    const counts = overviewProjectGraphCounts(many, undefined);
    expect(counts.size).toBe(401);
    expect(counts.get(graphProjectId(many[400]!))).toEqual({ total: 1, roots: 1, children: 0 });
  });

  it("reveals a chosen session in its project's reserved slot without moving another project's members", () => {
    const a = project("a", Array.from({ length: 390 }, (_, i) => agent(`a${i}`)));
    const b = project("b", Array.from({ length: 40 }, (_, i) => agent(`b${i}`)));
    const all = overviewGraphModel([a, b], undefined, null);
    const selected = idOf(all, "b39");
    const chosen = overviewGraphModel([a, b], undefined, null, { focus: "all", includeResting: false, selected });
    expect(all.visible.some((row) => row.id === selected)).toBe(false);
    expect(chosen.visible.some((row) => row.id === selected)).toBe(true);
    expect(chosen.projects.find((group) => group.project.workspace.id === "b")?.revealed?.id).toBe(selected);
    expect(chosen.structure).toBe(all.structure);
    expect(chosen.projects.length + chosen.visible.length).toBeLessThanOrEqual(400);
  });

  it("protects the entry session before applying the 400-node graph budget", () => {
    const projects = Array.from({ length: 401 }, (_, i) => project(`project${String(i).padStart(3, "0")}`, [agent(`pane${i}`)]));
    const all = overviewGraphModel(projects, undefined, null);
    const current = idOf(all, "pane400");
    expect(all.projects.some((group) => group.id === all.allAgents.find((row) => row.id === current)!.projectId)).toBe(false);
    const protectedGraph = overviewGraphModel(projects, undefined, null, { focus: "all", includeResting: false, pinned: [current] });
    expect(protectedGraph.visible.map((row) => row.id)).toContain(current);
    expect(protectedGraph.visible.length + protectedGraph.projects.length).toBeLessThanOrEqual(400);
    expect(protectedGraph.agents).toHaveLength(401);
    // The selected session's project is admitted the same way.
    const opened = overviewGraphModel(projects, undefined, null, { focus: "all", includeResting: false, selected: current });
    expect(opened.visible.map((row) => row.id)).toContain(current);
  });

  it("retains current and a different selected member simultaneously at the admission boundary", () => {
    const projects = Array.from({ length: 200 }, (_, i) => project(`project${String(i).padStart(3, "0")}`, [agent(`current${i}`), agent(`other${i}`)]));
    projects[0] = project("project000", [agent("current0"), agent("other0"), agent("selected0")]);
    const all = overviewGraphModel(projects, undefined, null);
    const current = idOf(all, "current0");
    const selected = idOf(all, "selected0");
    const options = { focus: "all" as const, includeResting: false, pinned: [current] };
    const before = overviewGraphModel(projects, undefined, null, options);
    const after = overviewGraphModel(projects, undefined, null, { ...options, selected });
    expect(after.visible.map((row) => row.id)).toEqual(expect.arrayContaining([current, selected]));
    expect(after.structure).toBe(before.structure);
    expect(after.projects.length + after.visible.length).toBeLessThanOrEqual(400);
    expect(after.agents).toHaveLength(401);
  });
});

// Overview focus B2-B8, B17-B18: projection uses authoritative marks, not bucket.
describe("Overview focus", () => {
  const panes = (model: Model) => model.agents.map((row) => row.agent.pane_id);
  it("keeps idle sessions, Seen demands, sleeping asks and unknown, with operator-first list order", () => {
    const p = project("focus", [agent("rest", { mark: "quiet" }), agent("sleep", { mark: "asleep" }), agent("unknown", { mark: "unknown" }), agent("result", { mark: "unread" }), agent("worker", { mark: "waiting" }), agent("seen-ask", { mark: "turn", group: "seen" }), agent("sleep-ask", { mark: "turn" })]);
    const all = overviewGraphModel([p], undefined, null);
    expect(panes(all)).toEqual(["seen-ask", "sleep-ask", "worker", "result", "unknown", "rest"]);
    expect(all.counts).toEqual({ all: 6, turn: 2, working: 1, results: 1 });
    for (const [focus, expected] of [["turn", ["seen-ask", "sleep-ask"]], ["working", ["worker"]], ["results", ["result"]]] as const) {
      expect(panes(overviewGraphModel([p], undefined, null, { focus, includeResting: false }))).toEqual(expected);
    }
    expect(panes(overviewGraphModel([p], undefined, null, { focus: "all", includeResting: true })).slice(-2)).toEqual(["rest", "sleep"]);
  });
  it("retains resting ancestors as context without increasing matching counts", () => {
    const p = project("family", [agent("root", { mark: "asleep" }), agent("middle", { mark: "quiet", lineage_parent_pane_id: "root" }), agent("child", { mark: "going", lineage_parent_pane_id: "middle" })]);
    const graph = overviewGraphModel([p], undefined, null, { focus: "working", includeResting: false });
    expect(panes(graph)).toEqual(["child"]);
    expect(graph.visible.map((row) => row.agent.pane_id).sort()).toEqual(["child", "middle", "root"]);
    expect([...graph.context].map((id) => pane(graph, id)).sort()).toEqual(["middle", "root"]);
    expect(graph.counts.working).toBe(1);
    expect(graph.layoutParents.get(idOf(graph, "child"))).toBe(idOf(graph, "middle"));
    expect(graph.layoutParents.get(idOf(graph, "middle"))).toBe(idOf(graph, "root"));
    expect(graph.quietBranches.size).toBe(0);
  });
  it("does not spend graph capacity on empty or sleeping-only projects", () => {
    const projects = Array.from({ length: 450 }, (_, i) => project(`empty${i}`, []));
    projects.push(project("rest", [agent("rest", { mark: "asleep" })]));
    const active = project("active", [agent("active")]);
    projects.push(active);
    const graph = overviewGraphModel(projects, undefined, null);
    expect(graph.projects.map((group) => group.id)).toEqual([graphProjectId(active)]);
    expect(graph.visible).toHaveLength(1);
    expect(graph.summaries.map((row) => row.id)).toEqual([graphProjectId(active)]);
    expect(graph.limited).toBe(false);
  });
  it("offers folds only for completely sleeping descendant branches, and releases a fold when a child becomes idle", () => {
    const rows = [agent("root"), agent("quiet", { mark: "asleep", lineage_parent_pane_id: "root" }), agent("leaf", { mark: "asleep", lineage_parent_pane_id: "quiet" })];
    const p = project("family", rows);
    const options = { focus: "all" as const, includeResting: true };
    const before = overviewGraphModel([p], undefined, null, options);
    const root = idOf(before, "root");
    expect(before.quietBranches.get(root)).toBe(2);
    const folded = overviewGraphModel([p], undefined, null, { ...options, folded: new Set([root]) });
    expect(folded.visible.map((row) => row.agent.pane_id)).toEqual(["root"]);
    expect(folded.agents).toHaveLength(3);
    const changed = project("family", rows.map((row) => row.pane_id === "leaf" ? { ...row, mark: "quiet" } : row));
    const active = overviewGraphModel([changed], undefined, null, { ...options, folded: new Set([root]) });
    expect(active.folded.size).toBe(0);
    expect(active.visible).toHaveLength(3);
  });
  it("shows idle-only projects by default and restores sleeping sessions on request", () => {
    const idle = project("idle", [agent("idle", { mark: "quiet" })]);
    const sleeping = project("sleeping", [agent("sleeping", { mark: "asleep" })]);
    const projects = [sleeping, idle];
    const graph = overviewGraphModel(projects, undefined, null);
    expect(graph.projects.map((group) => group.project.workspace.id)).toEqual(["idle"]);
    expect(panes(graph)).toEqual(["idle"]);
    expect(graph.visible.map((row) => row.agent.pane_id)).toEqual(["idle"]);
    const restored = overviewGraphModel(projects, undefined, null, { focus: "all", includeResting: true });
    expect(panes(restored)).toEqual(["idle", "sleeping"]);
    expect(restored.projects.map((group) => group.project.workspace.id).sort()).toEqual(["idle", "sleeping"]);
  });
  it("keeps the map when no session matches the filter, without mutating input", () => {
    const projects = [project("empty", []), project("rest", [agent("sleep", { mark: "asleep" })]), project("busy", [agent("busy")])];
    const before = JSON.stringify(projects);
    const graph = overviewGraphModel(projects, undefined, null, { focus: "turn", includeResting: false });
    expect(graph.agents).toEqual([]);
    expect(graph.projects.map((group) => group.project.workspace.id)).toEqual(["busy"]);
    expect(overviewGraphModel(projects, undefined, null, { focus: "all", includeResting: true }).projects).toHaveLength(2);
    expect(JSON.stringify(projects)).toBe(before);
  });
  it("terminates cyclic lineage, never offers an unsafe cyclic fold, and gives the cycle one team", () => {
    const p = project("cycle", [agent("a", { mark: "quiet", lineage_parent_pane_id: "b" }), agent("b", { mark: "quiet", lineage_parent_pane_id: "a" })]);
    const graph = overviewGraphModel([p], undefined, null, { focus: "all", includeResting: true });
    expect(graph.visible).toHaveLength(2);
    expect(graph.quietBranches.size).toBe(0);
    expect(new Set(graph.allAgents.map((row) => graph.teams.get(row.id))).size).toBe(1);
  });
});
