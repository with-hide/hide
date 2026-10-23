// What a practice chapter plays in: a world the real shell draws, and the core
// events it answers, applied here the way the core would apply them. Nothing
// leaves the page: no daemon, no Herdr, no agent.

import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, SnapshotRest } from "../snapshot";

export type CoreEvent = { kind: string; payload: Record<string, unknown> };

/**
 * What the practice window sends a world besides the core's events: text the
 * shell would have put on the clipboard (`{ text, what }`). The operator's
 * clipboard is never written from a practice screen.
 */
export const PRACTICE_COPY = "practice_copy";

export type WorldSnapshot = { rest: SnapshotRest; agents: AgentRow[]; focused: string | null; usage: NodeUsage | null };

export type PracticeWorld<M> = {
  initial: () => M;
  /** The model after `event`, or null for an event the chapter never needs. */
  apply: (model: M, event: CoreEvent) => M | null;
  snapshot: (model: M, nowMs: number) => WorldSnapshot;
  /** The panes whose terminals show a screen, and that screen for a terminal `cols` wide. */
  panes: (model: M) => string[];
  screen: (model: M, paneId: string, cols: number) => string;
  /** Whether the right lane is open on this world's screen. */
  lane: boolean;
};
