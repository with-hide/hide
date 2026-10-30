// A device's tasks as the Overview and the record read them (PRD
// overview-device-tasks B5, B8-B10, B25, B59): the backlog's project order,
// what a card's lines say, and which card an item or a pane belongs to. The
// expected answers are the PRD's Behaviors.

import { describe, expect, it } from "vitest";
import type { IncomingTaskScope, TaskCardScope, TaskState } from "./agentScope";
import { cardLine, cardOfItem, cardOfPane, cardPulls, cardQuestion, itemToken, overviewDeviceOf, projectOrder } from "./overviewTasks";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, SnapshotRest, Workspace } from "./snapshot";
import { sessionPull } from "../test/sessionPull";

type CheckoutSeed = { id: string; branch?: string; task_key?: string };

function card(root: string, state: TaskState, extra: Partial<TaskCardScope> = {}): TaskCardScope {
  return { root, state, members: [root], checkouts: [], recency: "", ...extra };
}

function project(id: string, cards: TaskCardScope[], extra: { needs_you?: number; recency?: string; incoming?: IncomingTaskScope[]; checkouts?: CheckoutSeed[]; tasks?: { key: string; id: string }[] } = {}): BoardProject {
  const workspace = {
    id,
    label: id,
    device_id: "local",
    checkouts: (extra.checkouts ?? []).map((checkout) => ({ label: checkout.id, branch: null, task_key: null, ...checkout })),
    tasks: { source: null, tasks: (extra.tasks ?? []).map((task) => ({ ...task, title: task.id, open: true })), overflow: false },
    agent_scope: { tasks: { cards, incoming: extra.incoming ?? [], needs_you: extra.needs_you ?? cards.filter((one) => one.state === "needs_you" || one.state === "blocked").length, recency: extra.recency ?? "" } },
  } as unknown as Workspace;
  return { workspace, agents: [], device: null };
}

function agent(pane: string, extra: Record<string, unknown> = {}): AgentRow {
  return { pane_id: pane, identity_label: pane, agent_kind: "claude", activity: "stopped", changed_at_unix_ms: null, state: {}, ...extra } as unknown as AgentRow;
}

const rowsOf = (...agents: AgentRow[]) => new Map(agents.map((one) => [one.pane_id, one]));

describe("the device the Overview shows (D-02)", () => {
  const rest = { navigator: { focused_device_id: "mini", devices: [{ id: "local" }, { id: "mini" }, { id: "laptop" }] } } as unknown as SnapshotRest;

  it("takes its own switch first, then the device a Home row named, then the rail's front device", () => {
    expect(overviewDeviceOf(rest, "laptop", "local")).toBe("laptop");
    expect(overviewDeviceOf(rest, null, "local")).toBe("local");
    expect(overviewDeviceOf(rest, null, null)).toBe("mini");
  });

  it("leaves a device removed since it was picked for the front one", () => {
    expect(overviewDeviceOf(rest, "gone", "also-gone")).toBe("mini");
  });
});

describe("an item's chip (B8)", () => {
  const workspace = project("p", [], { tasks: [{ key: "github:acme/app#12", id: "#12" }] }).workspace;

  it("uses the id the source shows, and reads `#N` or `L-N` from the key otherwise", () => {
    expect(itemToken("github:acme/app#12", workspace)).toBe("#12");
    expect(itemToken("github:acme/app#40", workspace)).toBe("#40");
    expect(itemToken("local:/work/app#7", workspace)).toBe("L-7");
  });

  it("has no chip for a key that names no number", () => {
    expect(itemToken("local:/work/app", workspace)).toBeNull();
    expect(itemToken("jira:APP-1#x", workspace)).toBeNull();
  });
});

describe("the backlog's project order (B25, B5)", () => {
  it("puts a project with a task waiting on the operator first, then the most recently active, then by name", () => {
    const calm = project("b-calm", [card("c1", "working")], { recency: "0000000000009" });
    const asking = project("z-asking", [card("a1", "needs_you")], { recency: "0000000000001" });
    const older = project("a-older", [card("o1", "working")], { recency: "0000000000005" });
    const tie = project("a-tie", [card("t1", "working")], { recency: "0000000000005" });
    expect(projectOrder([calm, older, asking, tie], null).map((one) => one.id)).toEqual(["z-asking", "b-calm", "a-older", "a-tie"]);
  });

  it("shows only the chosen project (B5)", () => {
    expect(projectOrder([project("a", [card("1", "working")]), project("b", [card("2", "working")])], "b").map((one) => one.id)).toEqual(["b"]);
  });
});

describe("a card's pull requests (B10)", () => {
  it("reads its root's list, where a member that ended keeps the pull request it made", () => {
    const made = sessionPull(5, "failed", { own: false, holders: ["child"] });
    const rows = rowsOf(agent("r", { pulls: { pulls: [made, sessionPull(6, "merged")] } }));
    expect(cardPulls(card("r", "working"), rows)).toEqual([made, sessionPull(6, "merged")]);
    expect(cardLine(card("r", "working", { stage: 2 }), rows)).toEqual({ kind: "pr", number: 5, state: "failed" });
  });
});

describe("a card's status line (B9)", () => {
  it("says why a task waits on the operator, from its root's question", () => {
    const rows = rowsOf(agent("r", { agent_kind: "codex", state: { ask: { verb: "answer", what: "Which branch?" } } }));
    expect(cardLine(card("r", "needs_you"), rows)).toEqual({ kind: "needs_you", verb: "answer", what: "Which branch?", agent: "codex" });
  });

  it("says why a task is blocked", () => {
    expect(cardLine(card("r", "blocked"), rowsOf(agent("r", { request: { line: "CI cannot reach the registry" } })))).toEqual({ kind: "blocked", what: "CI cannot reach the registry" });
  });

  it("says merged, with the cleanup left when the worktree stays", () => {
    expect(cardLine(card("r", "cleanup", { stage: 4 }), new Map())).toEqual({ kind: "merged", cleanup: true });
    expect(cardLine(card("r", "resting", { stage: 4 }), new Map())).toEqual({ kind: "merged", cleanup: false });
  });

  it("names the open pull request rather than a merged one", () => {
    const rows = rowsOf(agent("r", { pulls: { pulls: [sessionPull(1, "merged"), sessionPull(2, "draft")] } }));
    expect(cardLine(card("r", "working", { stage: 2 }), rows)).toEqual({ kind: "pr", number: 2, state: "draft" });
  });

  it("counts the members working, says a finished task is unread, and calls the rest resting", () => {
    const rows = rowsOf(agent("r", { activity: "working" }), agent("c1", { activity: "working" }), agent("c2"));
    expect(cardLine(card("r", "working", { members: ["r", "c1", "c2"] }), rows)).toEqual({ kind: "working", agents: 2 });
    expect(cardLine(card("r", "done"), new Map())).toEqual({ kind: "done", status: null });
    // The card says how the root's turn ended, as its row does (status-marks B6, B20).
    expect(cardLine(card("r", "done"), rowsOf(agent("r", { status_code: "unfinished" })))).toEqual({ kind: "done", status: "unfinished" });
    expect(cardLine(card("r", "resting"), new Map())).toEqual({ kind: "resting" });
  });

  it("puts the root's question on one line, its first non-empty one, only while it asks (B8)", () => {
    const rows = rowsOf(agent("r", { user_turn: { content: { text: "\n  Should I drop the cache?  \nIt is 2 GB." } } }));
    expect(cardQuestion(card("r", "needs_you"), rows)).toBe("Should I drop the cache?");
    expect(cardQuestion(card("r", "working"), rows)).toBeNull();
  });
});

describe("the card an item or a pane belongs to (D-31, B59)", () => {
  const started = project("p", [card("root", "working", { members: ["root", "child"], checkouts: ["c1"] })], { checkouts: [{ id: "c1", task_key: "local:/work/p#3" }] });
  const other = project("q", [card("solo", "resting")]);

  it("finds the task a started item is on, and none for an item no task works on", () => {
    expect(cardOfItem([other, started], "local:/work/p#3")).toMatchObject({ project: { id: "p" }, card: { root: "root" } });
    expect(cardOfItem([other, started], "local:/work/p#4")).toBeNull();
  });

  it("finds a member's task as well as a root's, and none for a pane in no task", () => {
    expect(cardOfPane([other, started], "child")?.card.root).toBe("root");
    expect(cardOfPane([other, started], "solo")?.project.id).toBe("q");
    expect(cardOfPane([other, started], "stranger")).toBeNull();
  });
});
