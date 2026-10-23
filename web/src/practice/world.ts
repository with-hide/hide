// What a practice chapter plays in: a world the real shell draws, and the core
// events it answers, applied here the way the core would apply them. Nothing
// leaves the page: no daemon, no Herdr, no agent.

import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, EditorSnapshot, LinkPanel, SessionRecordSection, SessionWorkSearch, SnapshotRest } from "../snapshot";
import type { Documents, FileIndexResult } from "../store";

export type CoreEvent = { kind: string; payload: Record<string, unknown> };

/**
 * What the practice window sends a world besides the core's events: text the
 * shell would have put on the clipboard (`{ text, what }`). A paste on a
 * practice screen takes the last of them back; the operator's clipboard is
 * never read or written from a practice screen.
 */
export const PRACTICE_COPY = "practice_copy";

/**
 * What the core sends beside the snapshot: the ⌘P index's answer, the open
 * editor tabs and their documents, ⌘K's session search, and the record and
 * link record the lane's 보기 slot shows.
 */
export type WorldFrames = {
  fileIndex?: FileIndexResult | null; editor?: EditorSnapshot | null; documents?: Documents;
  sessionWorkSearch?: SessionWorkSearch | null; sessionRecord?: SessionRecordSection | null; linkPanel?: LinkPanel | null;
};

export type WorldSnapshot = { rest: SnapshotRest; agents: AgentRow[]; focused: string | null; usage: NodeUsage | null; frames?: WorldFrames };

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
  /**
   * What on the screen is an invented record that could pass for a real one
   * (a pull request's title): the coach marks each match `예시`, so the
   * shell's own components stay as they are.
   */
  examples?: string[];
};
