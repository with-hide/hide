import { describe, expect, it } from "vitest";
import { deviceMissions, groupMissions, listedMissions, missionEntry, missionPillKey, missionProjects, rowLine, rowLineText, turnCount } from "./missionList";
import { workerEscalation } from "./missions";
import type { AgentRow, Mission, MissionNode } from "./snapshot";

// The mission screen's table (PRD mission-screen D-06, B1, B3-B5, B10, B11;
// PRD mission-tool B5, B7): which missions a device lists, its project chips,
// the group each stands in, the sidebar row's number and the list's order.

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: key, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

function mission(id: string, extra: Partial<Mission> = {}): Mission {
  return { id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const noRows = () => null;
const group = (m: Mission, rows: (node: MissionNode) => AgentRow | null = noRows) => missionEntry(m, (n) => workerEscalation(rows(n))).group;

describe("the missions a device lists (D-06, B3)", () => {
  it("takes every project's missions of the device and none of another device's", () => {
    const missions = [mission("a"), mission("b", { workspace_id: "w2", project: "/other" }), mission("c", { device_id: "mini" })];
    expect(deviceMissions(missions, "mac").map((m) => m.id)).toEqual(["a", "b"]);
    expect(deviceMissions(missions, "mini").map((m) => m.id)).toEqual(["c"]);
    expect(deviceMissions(missions, "gone")).toEqual([]);
  });

  it("offers each project once as a chip, by its catalog name or else its folder's", () => {
    const missions = [mission("a"), mission("b", { workspace_id: "w2", project: "/work/atorni-ax" }), mission("c")];
    expect(missionProjects(missions, [{ id: "w", label: "herdr-ide" }])).toEqual([{ id: "w2", label: "atorni-ax" }, { id: "w", label: "herdr-ide" }]);
    expect(missionProjects([mission("d", { workspace_id: "w3", project: "C:\\work\\billing" })], [])).toEqual([{ id: "w3", label: "billing" }]);
  });
});

describe("the group a mission stands in (B5) and the rail's number (D-08)", () => {
  it("puts an open mission with anything in its 내 차례 there, whatever its state", () => {
    expect(group(mission("m", { decision: "Which order?" }))).toBe("turn");
    expect(group(mission("m", { conductor: { pane_id: "p1", name: "c", agent_kind: "claude", session: null, present: false, takes_answers: true } }))).toBe("turn");
    expect(group(mission("m", { nodes: [node("x", { kind: "external", status: "working", attention: { reason: "pr_closed", pr: 3, to: "operator", since_unix_ms: 1, on_row: false, item: true, choices: ["leave", "drop"] } })] }))).toBe("turn");
    // What waits on the Conductor is not the operator's yet.
    expect(group(mission("m", { nodes: [node("x", { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } })] }))).not.toBe("turn");
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
    const entries = [mission("a", { decision: "?" }), mission("b"), mission("c", { status: "done", decision: "?" }), mission("d", { status: "merge_wait", nodes: [node("x", { status: "done", pr: 1, pr_state: "pending", landing: { state: "left", action: "merge" } })], landing_order: ["x"] })].map((m) => missionEntry(m, () => null));
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
    ].map((m) => missionEntry(m, () => null));
    expect(groupMissions(entries).map(({ group: name, entries: rows }) => [name, rows.map((row) => row.mission.id)])).toEqual([
      ["turn", ["ask"]],
      ["running", ["new-run", "old-run"]],
      ["closed", ["closed-last", "closed-first"]],
    ]);
  });
});

describe("the list beside the detail (B10, B11)", () => {
  it("walks the groups top down, 끝남 cut to its newest three until 모두 ›", () => {
    const closed = (id: string, at: number) => mission(id, { status: "done", closed_at_unix_ms: at });
    const entries = [mission("run"), mission("ask", { decision: "Which?" }), closed("c1", 10), closed("c2", 40), closed("c3", 30), closed("c4", 20)].map((m) => missionEntry(m, noRows));
    const groups = groupMissions(entries);
    expect(listedMissions(groups, false).map((entry) => entry.mission.id)).toEqual(["ask", "run", "c2", "c3", "c4"]);
    expect(listedMissions(groups, true).map((entry) => entry.mission.id)).toEqual(["ask", "run", "c2", "c3", "c4", "c1"]);
  });
});

describe("a mission whose Conductor has not got its letters (hide-private issue 678)", () => {
  it("stands in 내 차례 with the count and what to do, and leaves it once they are delivered", async () => {
    const { initializeInterfaceI18n } = await import("./i18n/instance");
    const t = initializeInterfaceI18n("en").getFixedT(null, "translation");
    const held = mission("a", { conductor_letters: { count: 4, reason: "unanswered", since_unix_ms: 1 } });
    const entry = missionEntry(held, () => null);
    expect(entry.group).toBe("turn");
    expect(turnCount([entry])).toBe(1);
    expect(missionPillKey(held, entry.turn[0] ?? null)).toBe("missionScreen.pill.letters");
    expect(rowLineText(rowLine(held, entry.turn[0] ?? null)!, t)).toBe("Letters waiting for the Conductor: 4 · three bells went unanswered: tell the Conductor to read its letters");
    expect(group(mission("a", { conductor_letters: null }))).toBe("running");
  });
});
