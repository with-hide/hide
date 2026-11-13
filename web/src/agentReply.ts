import { useEffect } from "react";
import { create } from "zustand";
import { allAgents } from "./paneAgents";
import type { PaneRepliesSection, PaneReplyReceipt } from "./snapshot";
import { useShellStore } from "./store";

/**
 * The Overview's reply to an agent without opening its session (the core's
 * `pane_reply`). One field per pane, shared by the 나를 기다리는 것 item and
 * the session detail, so the words follow the agent between them: the words
 * not yet taken, the session they were written for, the request out, and why
 * the last one was refused. The words stay until the core's receipt for the
 * request says the pane took them.
 */
export type ReplyField = {
  text: string;
  /** The digest of the session the words were written for; a field of another session in the same pane is not shown. */
  session: string | null;
  sent: string | null;
  refused: string | null;
  /** Order of the last change, for the cap. */
  at: number;
};

/** Fields kept at most: past it the least recently changed one that is not sending goes. */
export const REPLY_FIELDS_MAX = 24;
/** A reply the core holds no receipt for after this long reads as unknown: the event was lost, or the daemon restarted. */
export const REPLY_RECEIPT_WAIT_MS = 15_000;

const EMPTY: Omit<ReplyField, "session" | "at"> = { text: "", sent: null, refused: null };
let clock = 0;

type ReplyFields = {
  fields: Record<string, ReplyField>;
  change: (paneId: string, session: string | null, patch: Partial<Omit<ReplyField, "session" | "at">>) => void;
  /** Drops the fields of panes no agent runs in any more. */
  retire: (livePanes: ReadonlySet<string>) => void;
};

export const useReplyFields = create<ReplyFields>((set) => ({
  fields: {},
  change: (paneId, session, patch) =>
    set((state) => {
      const held = state.fields[paneId];
      // Words written for another session in this pane are not carried over.
      const base = held && held.session === session ? held : { ...EMPTY, session };
      const next: ReplyField = { ...base, ...patch, session, at: ++clock };
      const fields = { ...state.fields };
      // A field with nothing in it holds nothing.
      if (!next.text && !next.sent && !next.refused) delete fields[paneId];
      else fields[paneId] = next;
      return { fields: capped(fields) };
    }),
  retire: (livePanes) =>
    set((state) => {
      const gone = Object.keys(state.fields).filter((pane) => !livePanes.has(pane));
      if (gone.length === 0) return state;
      const fields = { ...state.fields };
      for (const pane of gone) delete fields[pane];
      return { fields };
    }),
}));

function capped(fields: Record<string, ReplyField>): Record<string, ReplyField> {
  const panes = Object.keys(fields);
  if (panes.length <= REPLY_FIELDS_MAX) return fields;
  const idle = panes.filter((pane) => !fields[pane]!.sent).sort((a, b) => fields[a]!.at - fields[b]!.at);
  for (const pane of idle.slice(0, panes.length - REPLY_FIELDS_MAX)) delete fields[pane];
  return fields;
}

// A pane that closed takes its field with it. Read only while the page is
// live with agents listed, so a reconnect's empty first frame keeps the drafts.
useShellStore.subscribe((state, before) => {
  if (state.agents === before.agents && state.rest?.status?.remote === before.rest?.status?.remote) return;
  if (state.connection !== "live" || state.agents.length === 0) return;
  if (Object.keys(useReplyFields.getState().fields).length === 0) return;
  useReplyFields.getState().retire(new Set(allAgents(state.agents, state.rest?.status?.remote).map((agent) => agent.pane_id)));
});

/** The core's receipt for `request`, if it holds one. */
export function replyReceipt(section: PaneRepliesSection | null, request: string | null): PaneReplyReceipt | null {
  if (!section || !request) return null;
  return section.receipts.find((receipt) => receipt.request_id === request) ?? null;
}

/**
 * What the core's receipt does to a field: nothing until it names the
 * field's own request and has settled; then `sent` empties the field and
 * `refused` keeps the words with the reason. `lost` (no receipt after the
 * wait) keeps the words as unknown, never as sending for ever.
 */
export function settleReply(field: Pick<ReplyField, "text" | "sent">, receipt: PaneReplyReceipt | null, lost = false): Partial<ReplyField> | null {
  if (!field.sent) return null;
  if (!receipt) return lost ? { sent: null, refused: "unknown" } : null;
  if (receipt.request_id !== field.sent || receipt.state === "sending") return null;
  return receipt.state === "sent" ? { text: "", sent: null, refused: null } : { sent: null, refused: receipt.reason ?? "failed" };
}

/**
 * The reply field of `paneId` for the session `session` drew there: its
 * words, whether a reply is out, the refusal to show, and `submit`, which
 * sends the words through `send` (one `pane_reply`). A send that never left
 * the page is refused as `offline`; editing the words clears a refusal.
 */
export function useAgentReply(paneId: string, session: string | null, send: (paneId: string, text: string, session: string | null) => string | null) {
  const held = useReplyFields((state) => state.fields[paneId]);
  const field = held && held.session === session ? held : null;
  const change = useReplyFields((state) => state.change);
  const sent = field?.sent ?? null;
  const receipt = useShellStore((state) => replyReceipt(state.paneReplies, sent));
  useEffect(() => {
    if (!field) return;
    const settled = settleReply(field, receipt);
    if (settled) change(paneId, session, settled);
  }, [receipt, field, paneId, session, change]);
  // No receipt for a reply out: after the wait the result is unknown.
  useEffect(() => {
    if (!sent || receipt) return;
    const timer = setTimeout(() => {
      const now = useReplyFields.getState().fields[paneId];
      if (now?.sent !== sent || replyReceipt(useShellStore.getState().paneReplies, sent)) return;
      change(paneId, session, settleReply(now, null, true)!);
    }, REPLY_RECEIPT_WAIT_MS);
    return () => clearTimeout(timer);
  }, [sent, receipt, paneId, session, change]);
  const text = field?.text ?? "";
  return {
    text,
    sending: sent !== null,
    refused: field?.refused ?? null,
    setText: (next: string) => change(paneId, session, { text: next, refused: null }),
    submit: () => {
      if (sent || !text.trim()) return;
      const request = send(paneId, text, session);
      change(paneId, session, request ? { sent: request, refused: null } : { refused: "offline" });
    },
  };
}
