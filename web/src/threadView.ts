// The open session thread as one conversation (PRD session-docs-and-assistant
// B25-B29): the core keeps the pages before the live one in
// `session_thread_history` and the page the session is still writing in
// `session_thread`, each on its own revision; the screen draws them as one.

import { threadLines, type AcpSessionUpdate, type DeskOutput, type DeskSessionRef, type ThreadHidePart, type ThreadLine, type ThreadReadFailure } from "./desk";
import { missionCounts, missionTurn } from "./missions";
import { heldBy } from "./components/mission-hold";
import type { AgentRow, MissionsSnapshot, PlansSnapshot, PlanView, SessionThreadHistorySection, SessionThreadSection, ThreadPage, ThreadPart, ThreadSession, ThreadUpdate } from "./snapshot";

export type ThreadConversation = {
  /** Every update the thread holds, oldest first. */
  updates: ThreadUpdate[];
  /** The Hide part of each tool call that made one, by tool call id; a later page's reading of a call wins. */
  parts: Record<string, ThreadPart>;
  /** Where each session the thread names runs, by the name as written; a later page's placing wins. */
  sessions: Record<string, ThreadSession>;
  /**
   * Each update's name, which neither a page read in above, the live page filling and joining the kept ones, nor the
   * oldest kept page being let go moves: its page's own first update and its place in that page. A screen keeps a
   * line's fold, its measured height and the place a reader stands at by it.
   */
  ids: string[];
};

const EMPTY: ThreadConversation = { updates: [], parts: {}, sessions: {}, ids: [] };

/** A small hash of a string, enough to tell pages apart by what they begin with. */
function hashOf(text: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

/** What a page begins with, without reading an image's bytes: a page is never read again from another start. */
function pageName(page: ThreadPage): string {
  const first = page.updates[0];
  if (!first) return "empty";
  const { content, ...rest } = first as { content?: { type?: string; text?: string; data?: string; mimeType?: string } } & Record<string, unknown>;
  const said = content ? { type: content.type, text: content.text?.slice(0, 256), mime: content.mimeType, bytes: content.data?.length } : null;
  return hashOf(JSON.stringify([rest.sessionUpdate, rest.toolCallId ?? null, rest.messageId ?? null, rest._meta ?? null, rest.title ?? null, said]));
}

/** Each update's lasting name (`ThreadConversation.ids`); two pages that begin alike are told apart by their order. */
function updateIds(pages: readonly ThreadPage[]): string[] {
  const seen = new Map<string, number>();
  return pages.flatMap((page) => {
    const name = pageName(page);
    const count = (seen.get(name) ?? 0) + 1;
    seen.set(name, count);
    const prefix = count === 1 ? name : `${name}.${count}`;
    return page.updates.map((_, index) => `${prefix}:${index}`);
  });
}

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
    ids: updateIds(pages),
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

/** The sections a thread's parts are found in, apart from the rows: a screen redraws the thread when only these move. */
export type ThreadHeld = { plans?: PlansSnapshot; missions?: MissionsSnapshot };

/** What a thread's screen draws from the core's sections: its lines, and whether its record is still being read or could not be read. */
export type ThreadScreen = { lines: ThreadLine[]; reading: boolean; failure: ThreadReadFailure | null };

/**
 * The thread as its screen draws it (B25-B27): the conversation's lines with
 * each `hide` call drawn as the Hide part it made, found in Hide's own state
 * by the names the record wrote (B26). A part Hide cannot draw truly (a
 * mission it does not hold, a diff View, a call with no target) stays the
 * command it was. A plan is drawn once, at its last change, as it is now;
 * the plan calls before it stay commands.
 */
export function threadScreen(thread: SessionThreadSection | null, history: SessionThreadHistorySection | null, held: ThreadHeld, rows: readonly AgentRow[]): ThreadScreen {
  const conversation = threadConversation(thread, history);
  const byPane = new Map(rows.map((row) => [row.pane_id, row]));
  const session = (name: string): DeskSessionRef => {
    const place = conversation.sessions[name];
    const row = place ? byPane.get(place.pane_id) : undefined;
    if (row) return { name: row.identity_label, mark: row.mark, agent_kind: row.agent_kind, pane_id: row.pane_id };
    return { name, mark: null, agent_kind: null, pane_id: place?.pane_id ?? null };
  };
  const plan = thread?.pane_id ? (held.plans?.items.find((one) => one.pane_id === thread.pane_id) ?? null) : null;
  const lastPlan = [...conversation.updates].reverse().find((update) => update.sessionUpdate === "tool_call" && conversation.parts[update.toolCallId as string]?.kind === "plan")?.toolCallId as string | undefined;
  const parts: Record<string, ThreadHidePart> = {};
  for (const [id, part] of Object.entries(conversation.parts)) {
    const drawn = hidePart(part, { session, plan: id === lastPlan ? plan : null, missions: held.missions });
    if (drawn) parts[id] = drawn;
  }
  return {
    lines: threadLines(conversation.updates as unknown as AcpSessionUpdate[], parts, session, conversation.ids),
    reading: thread?.state === "loading",
    failure: thread?.state === "failed" ? thread.failure : null,
  };
}

function hidePart(part: ThreadPart, found: { session: (name: string) => DeskSessionRef; plan: PlanView | null; missions: MissionsSnapshot | undefined }): ThreadHidePart | null {
  switch (part.kind) {
    case "plan":
      return found.plan ? { kind: "plan", plan: found.plan, target: "plan" } : null;
    case "spawn": {
      const named = part.agent_id ?? part.name;
      if (!named) return null;
      const placed = found.session(named);
      // A new session Hide cannot place is called by the name it was given, not its id, and told apart by its id.
      return { kind: "spawn", session: placed.mark === null && part.name ? { ...placed, name: part.name } : placed, target: `spawn:${placed.pane_id ?? named}` };
    }
    case "letter": {
      const peer = found.session(part.to);
      return { kind: "letter", direction: "sent", peer, body: (part.body ?? "").trim().split("\n", 1)[0] ?? "", target: `letter:${peer.pane_id ?? part.to}` };
    }
    case "view": {
      const output = viewOutput(part.view, part.target);
      // The View its answer named is the one thing opened, however its path was typed.
      return output ? { kind: "view", output, view_id: part.view_id ?? null, target: `view:${part.view_id ?? part.target}` } : null;
    }
    case "mission": {
      const mission = part.mission ? (found.missions?.items.find((one) => one.id === part.mission) ?? null) : null;
      if (!mission) return null;
      const { done, total } = missionCounts(mission);
      const delay = heldBy(mission);
      return { kind: "mission", target: `mission:${mission.id}`, mission: { id: mission.id, title: mission.title, status: mission.status, done, total, turn: missionTurn(mission), delay: delay?.line ?? null, delay_until_unix_ms: delay?.until ?? null } };
    }
  }
}

const IMAGE = /\.(png|jpe?g|gif|webp|svg|avif)$/i;

/**
 * An opened View as 결과물 draws it (D-65): a page or a file by its target; a
 * diff is not an output. Whether the operator saw it is not in the record, so
 * it carries no unseen dot.
 */
function viewOutput(view: "browser" | "file" | "diff", target: string): DeskOutput | null {
  if (view === "diff") return null;
  const kind = view === "browser" ? "page" : IMAGE.test(target) ? "image" : "doc";
  const label = view === "browser" ? target.replace(/^[a-z]+:\/\//i, "") : (target.split(/[\\/]/).filter(Boolean).at(-1) ?? target);
  // Where the session's call named it: a page's address, or a file as typed, read on this machine (D-65, B31).
  return { id: target, kind, label, thumb: null, seen: true, pr_state: null, device_id: null, checkout: null, path: view === "browser" ? null : target, url: view === "browser" ? target : null };
}
