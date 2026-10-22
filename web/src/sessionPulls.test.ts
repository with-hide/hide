import { describe, expect, it } from "vitest";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { ownPulls, sessionPulls, sessionStaleness } from "./prMark";
import type { AgentPullRequest, AgentRow, PrState, SnapshotRest, Workspace } from "./snapshot";

// The operator's A selection: the parent counts every distinct PR in its tree,
// even while descendants are hidden, and the menu identifies all owners.
function row(pane: string, numbers: number[] = [], children: string[] = [], states: PrState[] = numbers.map(() => "pending")): AgentRow {
  const agent = legacyAgentRow({ pane_id: pane, identity_label: `${pane} 세션`, lineage_child_pane_ids: children });
  const pulls: AgentPullRequest[] = numbers.map((number) => ({ number, title: `PR ${number}`, url: `https://github.com/acme/app/pull/${number}`, badge: "open", checks: "pending", head_branch: "feat/work", closing_issues: [], live: true, duty: false, created: true, settled_at_unix_ms: null }));
  return { ...agent, state: { ...agent.state, pr: numbers.length ? { count: numbers.length, worst: states[0]!, pulls: states.map((state, index) => ({ state, index })) } : null }, request: { verb: "working", verb_since_unix_ms: 0, request: null, later_by: null, pull_requests: pulls } };
}
const map = (rows: AgentRow[]) => new Map(rows.map((agent) => [agent.pane_id, agent]));

describe("a Sessions parent's PR list", () => {
  it("counts root, grandchildren and siblings past five, without borrowing their own marks", () => {
    const ids = ["c1", "c2", "c3", "c4", "c5", "c6"];
    const root = row("root", [41], ids);
    const children = ids.map((id, index) => row(id, [42 + index], id === "c1" ? ["grandchild"] : []));
    const grandchild = row("grandchild", [48], [], ["failed"]);
    const result = sessionPulls(root, map([root, ...children, grandchild]));
    expect(result.map(({ pull }) => pull.number)).toEqual([48, 41, 42, 43, 44, 45, 46, 47]);
    expect(result[0]!.owners.map((owner) => owner.identity_label)).toEqual(["grandchild 세션"]);
    expect(ownPulls(children[0]!).map(({ pull }) => pull.number)).toEqual([42]);
  });

  it("shares one URL entry between owners but keeps equal numbers in different repositories", () => {
    const root = row("root", [42], ["child", "foreign"], ["mergeable"]);
    const child = row("child", [42], [], ["failed"]);
    const foreign = row("foreign", [42]);
    foreign.request!.pull_requests[0]!.url = "https://github.com/acme/other/pull/42";
    const result = sessionPulls(root, map([root, child, foreign]));
    expect(result).toHaveLength(2);
    expect(result[0]!.state).toBe("failed");
    expect(result[0]!.owners.map((owner) => owner.pane_id)).toEqual(["root", "child"]);
    expect(result[1]!.pull.url).toBe("https://github.com/acme/other/pull/42");
  });

  it("uses the core's included PRs and terminates repeated or disappeared lineage", () => {
    const root = row("root", [41, 42], ["child", "child", "gone"]);
    root.state.pr = { count: 1, worst: "merged", pulls: [{ index: 1, state: "merged" }] };
    const child = row("child", [43], ["root"]);
    const result = sessionPulls(root, map([root, child]));
    expect(result.map(({ pull, state }) => [pull.number, state])).toEqual([[43, "pending"], [42, "merged"]]);
    expect(result[0]!.owners.map((owner) => owner.pane_id)).toEqual(["child"]);
    expect(sessionPulls(row("empty"), new Map())).toEqual([]);
  });

  it("keeps a stale child's last GitHub read visible even when the parent's read is fresh", () => {
    const root = row("root", [41], ["child", "other"]);
    const child = row("child", [42]);
    const other = row("other", [43]);
    const project = (agent: AgentRow, stale: boolean, lastRead: number | null) => ({ pull_requests: agent.request!.pull_requests, checkouts: [{ github: { stale, last_success_at_unix_ms: lastRead } }] }) as unknown as Workspace;
    const fresh = project(root, false, 300);
    const rest = { navigator: { workspaces: [fresh, project(child, true, 100)] }, status: { remote: [{ session: { workspaces: [project(other, true, 200)] } }] } } as unknown as SnapshotRest;
    const pulls = sessionPulls(root, map([root, child, other]));
    expect(sessionStaleness(rest, pulls)).toEqual({ stale: true, lastRead: 100 });
    expect(sessionStaleness({ navigator: { workspaces: [fresh, project(child, true, null)] } } as unknown as SnapshotRest, pulls)).toEqual({ stale: true, lastRead: null });
    expect(sessionStaleness(null, pulls)).toBeUndefined();
  });
});
