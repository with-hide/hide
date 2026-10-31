import { describe, expect, it } from "vitest";
import { groupMissions, missionEntry, projectMissions, shownMission, turnCount } from "./missionList";
import type { AgentRow, Mission, MissionNode } from "./snapshot";

// The Mission tool's list (PRD mission-tool D-08, D-09, B2, B5, B7, B23): which
// missions it lists, the group each stands in, and the rail's number.

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: key, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, worktree: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

function mission(id: string, extra: Partial<Mission> = {}): Mission {
  return { id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, other_vendor: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const noRows = () => null;
const group = (m: Mission, rows: (node: MissionNode) => AgentRow | null = noRows) => missionEntry(m, rows).group;

describe("the missions it lists (D-09)", () => {
  it("takes the front project's missions on its device, in any of its checkouts, and no other project's", () => {
    const project = { id: "w", path: "/repo", device_id: "mac" };
    const mine = mission("a", { checkout_path: "/repo-wt/feature" });
    const registeredLater = mission("b", { workspace_id: "", project: "/repo" });
    const other = mission("c", { workspace_id: "w2", project: "/other" });
    const otherDevice = mission("d", { workspace_id: "w9", device_id: "mini", project: "/repo" });
    expect(projectMissions([mine, registeredLater, other, otherDevice], project).map((m) => m.id)).toEqual(["a", "b"]);
  });
});

describe("the group a mission stands in (B5) and the rail's number (D-08)", () => {
  it("puts an open mission with anything in its 내 차례 there, whatever its state", () => {
    expect(group(mission("m", { decision: "Which order?" }))).toBe("turn");
    expect(group(mission("m", { conductor: { pane_id: "p1", name: "c", agent_kind: "claude", session: null, present: false, takes_answers: true } }))).toBe("turn");
    expect(group(mission("m", { nodes: [node("x", { kind: "external", status: "working", attention: { reason: "pr_closed", pr: 3, to: "operator", since_unix_ms: 1 } })] }))).toBe("turn");
    // What waits on the Conductor is not the operator's yet.
    expect(group(mission("m", { nodes: [node("x", { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1 } })] }))).not.toBe("turn");
    expect(group(mission("m", { status: "merge_wait", nodes: [node("x", { status: "done", pr: 3, pr_state: "mergeable", landing: { state: "left", action: "merge" } })], landing_order: ["x"] }))).toBe("turn");
    const raised = { escalation: { cause: "bell_exhausted" } } as unknown as AgentRow;
    expect(group(mission("m", { nodes: [node("x", { status: "working", pane_id: "p9" })] }), (n) => (n.pane_id === "p9" ? raised : null))).toBe("turn");
  });

  it("puts a plan or a running mission with nothing for the operator in 진행 중, a quiet merge wait in 머지 기다림, and a closed one in 끝남", () => {
    expect(group(mission("m", { status: "planning" }))).toBe("running");
    expect(group(mission("m", { nodes: [node("x", { status: "working" })] }))).toBe("running");
    expect(group(mission("m", { status: "merge_wait", nodes: [node("x", { status: "done", pr: 3, pr_state: "merged", landing: { state: "landed", proof: { by: "merged" } } })] }))).toBe("merge_wait");
    expect(group(mission("m", { status: "done", decision: "left over" }))).toBe("closed");
    expect(group(mission("m", { status: "cancelled" }))).toBe("closed");
  });

  it("counts the missions in 내 차례, so the rail and the group never disagree", () => {
    const entries = [mission("a", { decision: "?" }), mission("b"), mission("c", { status: "done", decision: "?" }), mission("d", { status: "merge_wait", nodes: [node("x", { status: "done", pr: 1, pr_state: "pending", landing: { state: "left", action: "merge" } })], landing_order: ["x"] })].map((m) => missionEntry(m, noRows));
    expect(turnCount(entries)).toBe(2);
    expect(turnCount([])).toBe(0);
  });
});

describe("the list's order (B5, B7)", () => {
  it("draws only groups that hold a mission, top down, open ones newest first and closed ones by when they closed", () => {
    const entries = [
      mission("old-run", { created_at_unix_ms: 1 }),
      mission("new-run", { created_at_unix_ms: 5 }),
      mission("closed-first", { status: "done", closed_at_unix_ms: 10 }),
      mission("closed-last", { status: "cancelled", closed_at_unix_ms: 30 }),
      mission("ask", { decision: "?" }),
    ].map((m) => missionEntry(m, noRows));
    expect(groupMissions(entries).map(({ group: name, entries: rows }) => [name, rows.map((row) => row.mission.id)])).toEqual([
      ["turn", ["ask"]],
      ["running", ["new-run", "old-run"]],
      ["closed", ["closed-last", "closed-first"]],
    ]);
  });
});

describe("the selected row (B6, B23)", () => {
  it("is the mission the 보기 slot shows, whichever place opened it, and none while the slot shows something else or is folded", () => {
    expect(shownMission({ items: [{ kind: "mission", mission: "m1" }], shown: true })).toBe("m1");
    expect(shownMission({ items: [{ kind: "mission", mission: "m1" }, { kind: "pull_request", workspace_id: "w", number: 2 }], shown: true })).toBeNull();
    expect(shownMission({ items: [{ kind: "mission", mission: "m1" }], shown: false })).toBeNull();
    expect(shownMission(null)).toBeNull();
  });
});
