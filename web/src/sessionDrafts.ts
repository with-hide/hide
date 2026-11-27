// What the operator was writing in a session's input beside Desk, kept per
// pane while they open another session or leave Desk, as Desk's own draft is
// (`deskDraft` in `ui.ts`). The window's memory alone, for the app's life.

import type { RemoteStatus } from "./snapshot";

/**
 * The words written in each pane's input, the one written in last at the end, with the device the pane is on
 * (null for this machine's), so a pane is let go only when its own machine says it is gone, and the words of a send
 * the pane has not answered yet (`sending`), kept here rather than in the panel so they outlive it.
 */
export type SessionDrafts = readonly SessionDraft[];
export type SessionDraft = { pane: string; device: string | null; text: string; sending?: { request: string; words: string } };

/** At most this many panes keep their words; a new one past it lets the one written in longest ago go. */
export const SESSION_DRAFTS_KEPT = 32;

/** A draft holds at most what a session takes in one send, in UTF-8 bytes (`operator_input::LIMIT_BYTES` in hide-core): more is cut. */
export const SESSION_DRAFT_BYTES = 16 * 1024;

export const NO_SESSION_DRAFTS: SessionDrafts = [];

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** `text` cut to [`SESSION_DRAFT_BYTES`] on a character's edge, and whether anything was cut. */
export function fitSessionDraft(text: string): { text: string; cut: boolean } {
  // Each UTF-16 unit is at most three UTF-8 bytes, so a short text needs no count.
  if (text.length * 3 <= SESSION_DRAFT_BYTES) return { text, cut: false };
  const bytes = encoder.encode(text);
  if (bytes.length <= SESSION_DRAFT_BYTES) return { text, cut: false };
  let end = SESSION_DRAFT_BYTES;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) end -= 1;
  return { text: decoder.decode(bytes.subarray(0, end)), cut: true };
}

/** The words `pane`'s input holds, none when it holds nothing. */
export function sessionDraft(drafts: SessionDrafts, pane: string): string {
  return drafts.find((draft) => draft.pane === pane)?.text ?? "";
}

/** `pane`'s input now holds `text` (already fitted): kept as the newest, or forgotten once empty with no send waiting. */
export function keepSessionDraft(drafts: SessionDrafts, pane: string, device: string | null, text: string): SessionDrafts {
  return put(drafts, { ...drafts.find((draft) => draft.pane === pane), pane, device, text });
}

/** `pane`'s input sent `words` as `request`: the box empties, and the words wait here for the pane's answer. */
export function sendSessionDraft(drafts: SessionDrafts, pane: string, device: string | null, request: string, words: string): SessionDrafts {
  return put(drafts, { pane, device, text: "", sending: { request, words } });
}

/**
 * The pane answered `request`: taken, its words go, so they never come back to go twice; refused, they come back to
 * the pane's box, before any words written since, so neither is lost.
 */
export function settleSessionSend(drafts: SessionDrafts, request: string, taken: boolean): SessionDrafts {
  const draft = drafts.find((one) => one.sending?.request === request);
  if (!draft?.sending) return drafts;
  const { words } = draft.sending;
  const text = taken ? draft.text : fitSessionDraft(draft.text === "" ? words : `${words}\n${draft.text}`).text;
  return drafts.map((one) => (one === draft ? { pane: one.pane, device: one.device, text } : one)).filter((one) => one.text !== "" || one.sending);
}

function put(drafts: SessionDrafts, draft: SessionDraft): SessionDrafts {
  const others = drafts.filter((one) => one.pane !== draft.pane);
  if (draft.text === "" && !draft.sending) return others.length === drafts.length ? drafts : others;
  const next = [...others, draft];
  return next.length > SESSION_DRAFTS_KEPT ? next.slice(next.length - SESSION_DRAFTS_KEPT) : next;
}

/**
 * The drafts whose pane is still there: one of this machine's goes once this machine's rows no longer list it, one of
 * a device's once that device, connected, no longer lists it. A device not connected, stale or no longer paired keeps
 * its panes' words, since it cannot say whether the pane is gone.
 */
export function sessionDraftsLeft(drafts: SessionDrafts, local: readonly { pane_id: string }[], remote: readonly RemoteStatus[] | undefined): SessionDrafts {
  const kept = drafts.filter((draft) => {
    if (draft.device === null) return local.some((row) => row.pane_id === draft.pane);
    const status = remote?.find((one) => one.target_id === draft.device);
    if (status?.state !== "connected" || !status.session) return true;
    return status.session.agents.some((row) => row.pane_id === draft.pane);
  });
  return kept.length === drafts.length ? drafts : kept;
}
