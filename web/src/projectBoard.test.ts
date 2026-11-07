import { emptyScope } from "../test/legacyAgentScope";
// What the Overview, the sidebar and the Factory share about a project's
// checkouts: a checkout's Git stage, the layered dependency layout, and the
// day an issue states.

import { describe, expect, it } from "vitest";
import { checkoutAgentRows, issueDate, layerDependencies, stageOf } from "./projectBoard";
import type { AgentRow, Checkout, PrState, PullRequest, Workspace } from "./snapshot";

function pr(badge: PullRequest["badge"], checks: PullRequest["checks"] = "unknown", draft = false, state: PrState = "pending"): PullRequest {
  return { number: 7, title: "PR", url: "https://github.com/acme/project/pull/7", badge, review: null, is_draft: draft, checks, state: badge === "merged" || badge === "closed" ? badge : state };
}

/** `merged` is work that landed (Git and the link record agree); `inBase` is only Git finding HEAD in the base, as a checkout with no commits of its own does. */
function checkout(id: string, options: { changed?: number; ahead?: number; pr?: PullRequest; worktree?: boolean; panes?: string[]; merged?: boolean; inBase?: boolean; task?: string; closes?: string[]; behind?: number } = {}): Checkout {
  const worktree = options.worktree ?? true;
  return { agent_scope: emptyScope(),
    id,
    workspace_id: "project",
    label: id,
    path: `/fixture/${id}`,
    branch: id,
    purpose: null,
    is_worktree: worktree,
    exists: true,
    has_panes: (options.panes ?? []).length > 0,
    worktree: { merged: options.merged || options.inBase ? true : null, is_main: !worktree, behind_upstream: options.behind ?? null } as Checkout["worktree"],
    landed: options.merged ?? false,
    pull_request: options.pr ?? null,
    issue: null,
    task_key: options.task ?? null,
    closes_task_keys: options.closes ?? [],
    changed_file_count: options.changed ?? 0,
    ahead: options.ahead ?? 0,
    tabs: [{ id: `${id}:tab`, workspace_id: "project", checkout_id: id, label: "Tab 1", empty: false, delegated: false, panes: (options.panes ?? []).map((pane) => ({ id: pane })) as never }],
    active_tab_id: `${id}:tab`,
    strip: [],
    next_tab_label: "Tab 2",
  };
}


describe("the Git stage", () => {
  it("is done, then review, else in progress; agents and the issue's state never move it", () => {
    expect(stageOf(checkout("a"))).toBe("working");
    expect(stageOf(checkout("a", { changed: 1 }))).toBe("working");
    expect(stageOf(checkout("a", { ahead: 1 }))).toBe("working");
    expect(stageOf(checkout("a", { changed: 3, pr: pr("open") }))).toBe("review");
    expect(stageOf(checkout("a", { changed: 3, pr: pr("review") }))).toBe("review");
    expect(stageOf(checkout("a", { changed: 3, pr: pr("merged") }))).toBe("done");
    expect(stageOf(checkout("a", { changed: 3, pr: pr("closed") }))).toBe("working");
    expect(stageOf(checkout("a", { merged: true }))).toBe("done");
    // Git finds a checkout with no commits of its own in its base too; only landed work is done.
    expect(stageOf(checkout("a", { inBase: true }))).toBe("working");
  });
});

describe("the layered dependency layout", () => {
  it("puts a node after the longest chain of its blockers, orders a column by the rows it hangs from, keeps unrelated nodes apart, and drops the arrow that closes a cycle", () => {
    type Node = { id: string; after: string[] };
    const nodes: Node[] = [
      { id: "a", after: [] },
      { id: "b", after: [] },
      { id: "lone", after: [] },
      { id: "c", after: ["b"] },
      { id: "d", after: ["a"] },
      { id: "x", after: ["y"] },
      { id: "y", after: ["x"] },
    ];
    const byId = new Map(nodes.map((node) => [node.id, node]));
    const graph = layerDependencies(nodes, (node) => node.id, (node) => node.after.map((id) => byId.get(id)!));
    // `d` hangs from `a` on row 0 and `c` from `b` on row 1, so `d` comes first in its column.
    expect(graph.layers.map((layer) => layer.map((node) => node.id))).toEqual([["a", "b", "y"], ["d", "c", "x"]]);
    expect(graph.unrelated.map((node) => node.id)).toEqual(["lone"]);
    expect(graph.edges.filter((edge) => [edge.from, edge.to].every((id) => id === "x" || id === "y"))).toHaveLength(1);
  });
});

describe("the day an issue states", () => {
  it("follows the interface language's own calendar words", () => {
    const day = new Date(2026, 8, 27, 12).getTime();
    expect(issueDate(day, "en")).toBe("Sep 27");
    expect(issueDate(day, "ko")).toBe("9월 27일");
  });
});

// with-hide/hide-private#664: a delegation crosses machines, so the rows the core
// draws under an opened parent can be a row of another device. The sidebar
// resolves them against every device's agents, never the parent's device alone.
describe("checkoutAgentRows", () => {
  it("draws a child that works on another device under its parent, one step in", () => {
    const parent = { pane_id: "root" } as AgentRow;
    const child = { pane_id: "remote:win:pane:child" } as AgentRow;
    const base = checkout("main", { panes: ["root"] });
    const ref = (pane_id: string, depth: number) => ({ pane_id, occurrence: 0, depth });
    const tree = { rows: [ref("root", 0)], visible_rows: [ref("root", 0), ref("remote:win:pane:child", 1)], shown: [], more: 0, needs_you: false, turn_kind: null };
    const withTree = { ...base, agent_scope: { ...base.agent_scope, sidebar_tree: tree } } as unknown as Checkout;
    const workspace = { checkouts: [withTree] } as unknown as Workspace;

    const rows = checkoutAgentRows(workspace, [parent, child], "sidebar").get("main");

    expect(rows?.map((row) => [row.agent.pane_id, row.depth])).toEqual([["root", 0], ["remote:win:pane:child", 1]]);
    expect(() => checkoutAgentRows(workspace, [parent], "sidebar")).toThrow("Missing agent occurrence");
  });
});
