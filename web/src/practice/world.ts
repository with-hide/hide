// What a practice chapter plays in: a world the real shell draws, and the core
// events it answers, applied here the way the core would apply them. Nothing
// leaves the page: no daemon, no engine, no agent.

import type { NodeUsage } from "../generated/hided-ws";
import type { AgentRow, ChangesSnapshot, EditorSnapshot, LinkPanel, MemorySection, SessionRecordSection, SessionWorkSearch, SnapshotRest, SuggestionsSection } from "../snapshot";
import type { DirectoryList, Documents, FileIndexResult } from "../store";

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

/**
 * What the core sends beside the snapshot: the ⌘P index's answer, the open
 * editor tabs and their documents, ⌘K's session search, the record and link
 * record the lane's 보기 slot shows, the Explorer's folder listings (keyed by
 * folder path) and the checkout's Git changes, and the Memory tool's Project
 * with the Hide AI tool's 제안 section it is drawn under.
 */
export type WorldFrames = {
  fileIndex?: FileIndexResult | null; editor?: EditorSnapshot | null; documents?: Documents;
  sessionWorkSearch?: SessionWorkSearch | null; sessionRecord?: SessionRecordSection | null; linkPanel?: LinkPanel | null;
  listings?: Record<string, DirectoryList>; changes?: ChangesSnapshot | null; memory?: MemorySection | null; suggestions?: SuggestionsSection | null;
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
  /**
   * The words typed into `paneId`'s prompt and not sent yet. A world whose
   * chapter types to an agent keeps it, so ⏎ sends what the learner wrote
   * there and types the step's example only into an empty prompt.
   */
  prompt?: (model: M, paneId: string) => string;
  /**
   * What on the screen is an invented record that could pass for a real one
   * (a pull request's title): the coach marks each match `예시`, so the
   * shell's own components stay as they are.
   */
  examples?: string[];
  /**
   * What the world does next on its own, as an agent at work would: the event
   * applied at `at` (epoch ms) unless another event changes the model first,
   * which asks again. A world that only answers the learner has none.
   */
  later?: (model: M) => { at: number; event: CoreEvent } | null;
};
