// The answer to one start request (PRD home-device-rail D-17, D-26): the core
// has one task slot and one last-error slot for every surface, so a start
// surface reads only the receipt that carries the id it sent. Another
// surface's start, or an older one of its own, is never its answer.

import type { SnapshotRest } from "./snapshot";

/** The core's bound for a start, including the first catalog of its checkout. */
export const START_ANSWER_TIMEOUT_MS = 90_000;

export type StartAnswer =
  | { phase: "pending" }
  /** The core refused the request before any work: `kind` names why, worded by `startRefusal`. */
  | { phase: "refused"; kind: string }
  /** `message` is the core's reason, null when it sent none. */
  | { phase: "failed"; message: string | null }
  /** The tab exists; `agentPhase` says whether the agent behind it started (`starting`, `started`, `failed`). */
  | { phase: "ready"; taskId: number; paneId: string | null; agentPhase: string | null; agentReason: string | null; agentLine: string | null };

/** 1-64 characters of `[A-Za-z0-9_-]`, the core's request id format. */
export function startRequestId(): string {
  const random = globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;
  return `start-${random}`;
}

export function startAnswer(rest: SnapshotRest | null, requestId: string, kind: "agent_start" | "worktree_create" = "agent_start"): StartAnswer {
  const error = rest?.status?.last_error;
  if (error?.request_id === requestId) return { phase: "refused", kind: error.kind };
  const operation = rest?.task_operation;
  if (operation?.request_id !== requestId || operation.kind !== kind) return { phase: "pending" };
  if (operation.phase === "failed") return { phase: "failed", message: operation.failure };
  if (operation.phase !== "ready") return { phase: "pending" };
  return { phase: "ready", taskId: operation.id, paneId: operation.pane_id, agentPhase: operation.agent_phase, agentReason: operation.agent_reason, agentLine: operation.agent_line };
}
