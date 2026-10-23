import { describe, expect, it } from "vitest";
import type { SnapshotRest } from "./snapshot";
import { startAnswer, startRequestId } from "./startAnswer";

const operation = (over: Record<string, unknown>) => ({ id: 7, kind: "agent_start", phase: "working", pane_id: null, agent_phase: null, agent_message: null, message: null, request_id: "mine", ...over });
const rest = (task: Record<string, unknown> | null, error: Record<string, unknown> | null = null) =>
  ({ task_operation: task, status: { last_error: error } }) as unknown as SnapshotRest;

describe("the answer to one start request", () => {
  it("waits while the task is working or nothing has answered", () => {
    expect(startAnswer(rest(operation({})), "mine")).toEqual({ phase: "pending" });
    expect(startAnswer(rest(null), "mine")).toEqual({ phase: "pending" });
    expect(startAnswer(null, "mine")).toEqual({ phase: "pending" });
  });

  it("reads only the receipt that carries its own id", () => {
    expect(startAnswer(rest(operation({ phase: "ready", request_id: "theirs", pane_id: "p9" })), "mine")).toEqual({ phase: "pending" });
    expect(startAnswer(rest(null, { kind: "task_operation.busy", message: "busy", request_id: "theirs" }), "mine")).toEqual({ phase: "pending" });
    expect(startAnswer(rest(operation({ phase: "ready", request_id: null, pane_id: "p9" })), "mine")).toEqual({ phase: "pending" });
  });

  it("ignores another kind of task that happens to carry the id", () => {
    expect(startAnswer(rest(operation({ kind: "worktree_create", phase: "ready" })), "mine")).toEqual({ phase: "pending" });
  });

  it("accepts a correlated worktree start only when that was the requested kind", () => {
    const answer = rest(operation({ kind: "worktree_create", phase: "ready", pane_id: "tree-pane" }));
    expect(startAnswer(answer, "mine", "worktree_create")).toMatchObject({ phase: "ready", paneId: "tree-pane" });
    expect(startAnswer(answer, "another", "worktree_create")).toEqual({ phase: "pending" });
  });

  it("carries a refusal's reason", () => {
    expect(startAnswer(rest(null, { kind: "home.conflict", message: "~/hide is not Hide's", request_id: "mine" }), "mine")).toEqual({ phase: "refused", kind: "home.conflict" });
  });

  it("reports a ready task with the pane and how the agent stands", () => {
    expect(startAnswer(rest(operation({ phase: "ready", pane_id: "p1", agent_phase: "starting" })), "mine")).toEqual({
      phase: "ready",
      taskId: 7,
      paneId: "p1",
      agentPhase: "starting",
      agentMessage: null,
    });
    expect(startAnswer(rest(operation({ phase: "ready", pane_id: "p1", agent_phase: "failed", agent_message: "model refused" })), "mine")).toMatchObject({ agentPhase: "failed", agentMessage: "model refused" });
  });

  it("reports a failed task with its message", () => {
    expect(startAnswer(rest(operation({ phase: "failed", message: "no tab" })), "mine")).toEqual({ phase: "failed", message: "no tab" });
  });

  it("carries no message when the failed task has none", () => {
    expect(startAnswer(rest(operation({ phase: "failed" })), "mine")).toEqual({ phase: "failed", message: null });
  });

  it("makes ids in the core's format", () => {
    const id = startRequestId();
    expect(id).toMatch(/^[A-Za-z0-9_-]{1,64}$/);
    expect(startRequestId()).not.toBe(id);
  });
});
