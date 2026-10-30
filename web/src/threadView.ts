// The open session thread as one conversation (PRD session-docs-and-assistant
// B25-B29): the core keeps the pages before the live one in
// `session_thread_history` and the page the session is still writing in
// `session_thread`, each on its own revision; the screen draws them as one.

import type { SessionThreadHistorySection, SessionThreadSection, ThreadPart, ThreadUpdate } from "./snapshot";

export type ThreadConversation = {
  /** Every update the thread holds, oldest first. */
  updates: ThreadUpdate[];
  /** The Hide part of each tool call that made one, by tool call id; a later page's reading of a call wins. */
  parts: Record<string, ThreadPart>;
};

const EMPTY: ThreadConversation = { updates: [], parts: {} };

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
