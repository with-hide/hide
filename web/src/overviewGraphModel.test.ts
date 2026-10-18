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

  it("aggregates real directions and kinds into a collapsed project endpoint, and restores session endpoints when expanded", () => {
    const a = project("one", [agent("a"), agent("child", { lineage_parent_pane_id: "a" })]);
    const b = project("two", [agent("b")]);
    const facts = metadata([party("a"), party("child"), party("b")], [letter("1", party("a"), party("b")), letter("2", party("child"), party("b"), "progress", 20), letter("3", party("b"), party("a"), "report", 30)]);
    const folded = overviewGraphModel([a, b], facts, new Set(), null);
    expect(folded.communication).toHaveLength(1);
    expect(folded.communication[0]).toMatchObject({ count: 3, latest: 30, kinds: { request: 1, progress: 1, report: 1 } });
    expect(folded.communication[0]?.directions.map((row) => row.count).sort()).toEqual([1, 2]);
    const open = overviewGraphModel([a, b], facts, new Set([graphProjectId(a)]), null);
    expect(open.communication).toHaveLength(2);
    expect(open.communication.every((row) => row.source !== graphProjectId(a) && row.target !== graphProjectId(a))).toBe(true);
  });

  it("keeps topology stable when status, labels, correspondence or activity order refresh", () => {
    const first = project("one", [agent("b"), agent("a")]);
    const second = project("two", [agent("c")]);
    const expanded = new Set([graphProjectId(first)]);
    const before = overviewGraphModel([first, second], metadata([party("a"), party("b"), party("c")]), expanded, null);
    const changed = project("one", [agent("a", { identity_label: "새 상태", group: "needs_you", demand: "question" }), agent("b", { group: "seen" })]);
    const after = overviewGraphModel([second, changed], metadata([party("a"), party("b"), party("c")], [letter("new", party("a"), party("c"))]), expanded, null);
    expect(after.structure).toBe(before.structure);
    expect(after.visible.find((row) => row.agent.pane_id === "a")?.bucket).toBe("turn");
  });

  it("counts project groups against the cap and retains every session in the fallback list", () => {
    const p = project("large", Array.from({ length: 400 }, (_, i) => agent(`p${i}`)));
    const graph = overviewGraphModel([p], undefined, new Set([graphProjectId(p)]), null);
    expect(graph.limited).toBe(true);
    expect(graph.visible).toHaveLength(0);
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

  it("opens the chosen project, then the recent checkout, without using device display labels as identity", () => {
    const a = project("one", [agent("a")]);
    const b = project("two", [agent("b")]);
    expect([...initialGraphExpansion([a, b], "one", ["two-main"])]).toEqual([graphProjectId(a)]);
    expect([...initialGraphExpansion([a, b], null, ["two-main"])]).toEqual([graphProjectId(b)]);
    expect(graphProjectId(project("one", [], "remote-1"))).not.toBe(graphProjectId(project("one", [], "remote-2")));
  });
});
