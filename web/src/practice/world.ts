// What a practice chapter plays in: a world the real shell draws, and the core
// events it answers, applied here the way the core would apply them. Nothing
// leaves the page: no daemon, no engine, no agent.

import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, SnapshotRest } from "../snapshot";
import type { useShellStore } from "../store";
import type { PracticeScene } from "./chapter";

export type CoreEvent = { kind: string; payload: Record<string, unknown> };

/**
 * What the practice window sends a world besides the core's events: text the
 * shell would have put on the clipboard (`{ text, what }`). A paste on a
 * practice screen takes the last of them back; the operator's clipboard is
 * never read or written from a practice screen.
 */
export const PRACTICE_COPY = "practice_copy";

/**
 * What the practice window sends a world when the shell would have left the
 * page (`{ what, detail }`: a window it would open, a link, a navigation, or a
 * call on the desktop host), held there instead (`outside.ts`). A world that
 * has no answer for it keeps it as told.
 */
export const PRACTICE_OUTSIDE = "practice_outside";

/** Shell state a world holds beside the core's snapshot: the documents a lane shows. */
export type WorldExtra = Partial<Pick<ReturnType<typeof useShellStore.getState>, "editor" | "documents" | "listings">>;

export type WorldSnapshot = { rest: SnapshotRest; agents: AgentRow[]; focused: string | null; usage: NodeUsage | null; extra?: WorldExtra };

export type PracticeWorld<M> = {
  initial: () => M;
  /** The model with a card's scene set on it (`PracticeScene`), for a world that has one. */
  scene?: (model: M, scene: PracticeScene) => M;
  /** The model after `event`, or null for an event the chapter never needs. */
  apply: (model: M, event: CoreEvent) => M | null;
  snapshot: (model: M, nowMs: number) => WorldSnapshot;
  /** The panes whose terminals show a screen, and that screen for a terminal `cols` wide. */
  panes: (model: M) => string[];
  screen: (model: M, paneId: string, cols: number) => string;
};
