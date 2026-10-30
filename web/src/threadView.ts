// The open session thread as one conversation (PRD session-docs-and-assistant
// B25-B29): the core keeps the pages before the live one in
// `session_thread_history` and the page the session is still writing in
// `session_thread`, each on its own revision; the screen draws them as one.

import type { SessionThreadHistorySection, SessionThreadSection, SnapshotRest, ThreadPart, ThreadSession, ThreadUpdate } from "./snapshot";

export type ThreadConversation = {
  /** Every update the thread holds, oldest first. */
  updates: ThreadUpdate[];
  /** The Hide part of each tool call that made one, by tool call id; a later page's reading of a call wins. */
  parts: Record<string, ThreadPart>;
  /** Where each session the thread names runs, by the name as written; a later page's placing wins. */
  sessions: Record<string, ThreadSession>;
};

const EMPTY: ThreadConversation = { updates: [], parts: {}, sessions: {} };

/**
 * The thread's conversation. Kept pages of another generation belong to a
 * thread that closed and are left out, so a screen never draws one thread's
 * pages under another's head.
 */
export function threadConversation(thread: SessionThreadSection | null, history: SessionThreadHistorySection | null): ThreadConversation {
  if (!thread || thread.state === "closed") return EMPTY;
  const pages = [...(history?.generation === thread.generation ? history.pages : []), ...(thread.tail ? [thread.tail] : [])];
  if (pages.length === 0) return EMPTY;
  return {
    updates: pages.flatMap((page) => page.updates),
    parts: Object.assign({}, ...pages.map((page) => page.parts)) as Record<string, ThreadPart>,
    sessions: Object.assign({}, ...pages.map((page) => page.sessions)) as Record<string, ThreadSession>,
  };
}

/**
 * Why the input box cannot send now, or null when it can: the pane no longer
 * runs the thread's session, or the last words are still on their way (one
 * input at a time).
 */
export function threadInputHeld(thread: SessionThreadSection | null): "pane_changed" | "pane_gone" | "in_flight" | "closed" | null {
  if (!thread || thread.state === "closed" || !thread.pane_id) return "closed";
  if (thread.pane === "changed") return "pane_changed";
  if (thread.pane === "gone") return "pane_gone";
  if (thread.input?.state === "sending") return "in_flight";
  return null;
}

/**
 * The model Hide launched the thread's session with, for its head's second
 * line: the launch choice the Overview map reads (`overview_graph.sessions`),
 * never a claim about a later switch inside the agent; null when the launch
 * chose none, so the line names no model the system cannot produce.
 */
export function threadLaunchModel(rest: SnapshotRest | null | undefined, thread: SessionThreadSection | null): string | null {
  if (!thread?.pane_id || !thread.machine) return null;
  const raw = thread.pane_id.startsWith(`remote:${thread.machine}:pane:`) ? thread.pane_id.slice(`remote:${thread.machine}:pane:`.length) : thread.pane_id;
  const launch = rest?.overview_graph?.sessions.find((one) => one.device_id === thread.machine && (one.pane_id === thread.pane_id || one.pane_id === raw));
  return launch?.launch_model ?? null;
}
