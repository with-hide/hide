import { create } from "zustand";
import { allAgents } from "./paneAgents";
import type { PaneRepliesSection, PaneReplyReceipt } from "./snapshot";
import { useShellStore } from "./store";

/**
 * The Overview's reply to an agent without opening its session (the core's
 * `pane_reply`). One field per pane, shared by the 나를 기다리는 것 item and
 * the session detail, so the words follow the agent between them: the words
 * not yet taken, the session they were written for, the request out and when
 * it went, and why the last one was refused. The words stay until the core's
 * receipt for the request says the pane took them.
 *
 * The store owns every field, mounted or not: it settles a field when the
 * snapshot carries its receipt, and one timer reads a reply the core holds no
 * receipt for after [`REPLY_RECEIPT_WAIT_MS`] as unknown. Fields are bounded
 * by their owner, one per live pane: a pane that closes takes its field, and
 * words written for another session in the same pane are replaced. Each
 * field's words are bounded by [`REPLY_MAX_LENGTH`].
 */
export type ReplyField = {
  text: string;
  /** The digest of the session the words were written for; a field of another session in the same pane is not shown. */
  session: string | null;
  sent: string | null;
  /** When `sent` went, in the page's clock. */
  sentAt: number | null;
  refused: string | null;
};

/** A reply the core holds no receipt for after this long reads as unknown: the event was lost, or the daemon restarted. */
export const REPLY_RECEIPT_WAIT_MS = 15_000;
/** UTF-16 units a field holds: three UTF-8 bytes at most each, so the words stay under the core's 16 KiB refusal. */
export const REPLY_MAX_LENGTH = Math.floor((16 * 1024) / 3);

type Patch = Partial<Omit<ReplyField, "session">>;

type ReplyFields = {
  fields: Record<string, ReplyField>;
  change: (paneId: string, session: string | null, patch: Patch) => void;
  /** Sends the field's words through `send` (one `pane_reply`); a send that never left the page is refused as `offline`. */
  submit: (paneId: string, session: string | null, send: (paneId: string, text: string, session: string | null) => string | null) => void;
  /** Settles the fields the core's receipts answer, and those it never answered past the wait. */
  settle: (section: PaneRepliesSection | null, now: number) => void;
  /** Drops the fields of panes no agent runs in any more. */
  retire: (livePanes: ReadonlySet<string>) => void;
};

function write(fields: Record<string, ReplyField>, paneId: string, session: string | null, patch: Patch): Record<string, ReplyField> {
  const held = fields[paneId];
  // Words written for another session in this pane are not carried over.
  const base: ReplyField = held && held.session === session ? held : { text: "", session, sent: null, sentAt: null, refused: null };
  const next: ReplyField = { ...base, ...patch, session };
  const out = { ...fields };
  // A field with nothing in it holds nothing.
  if (!next.text && !next.sent && !next.refused) delete out[paneId];
  else out[paneId] = next;
  return out;
}

export const useReplyFields = create<ReplyFields>((set, get) => ({
  fields: {},
  change: (paneId, session, patch) => set((state) => ({ fields: write(state.fields, paneId, session, patch) })),
  submit: (paneId, session, send) => {
    const field = get().fields[paneId];
    if (!field || field.session !== session || field.sent || !field.text.trim()) return;
    const request = send(paneId, field.text, session);
    get().change(paneId, session, request ? { sent: request, sentAt: Date.now(), refused: null } : { refused: "offline" });
  },
  settle: (section, now) =>
    set((state) => {
      let fields = state.fields;
      for (const [paneId, field] of Object.entries(state.fields)) {
        const settled = settleReply(field, replyReceipt(section, field.sent), field.sentAt !== null && now - field.sentAt >= REPLY_RECEIPT_WAIT_MS);
        if (settled) fields = write(fields, paneId, field.session, settled);
      }
      return fields === state.fields ? state : { fields };
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

// The one timer: armed for the earliest reply still waiting on a receipt.
let expiry: ReturnType<typeof setTimeout> | null = null;
function armExpiry(): void {
  if (expiry) clearTimeout(expiry);
  expiry = null;
  const waits = Object.values(useReplyFields.getState().fields).flatMap((field) => (field.sent && field.sentAt !== null ? [field.sentAt + REPLY_RECEIPT_WAIT_MS] : []));
  if (waits.length === 0) return;
  expiry = setTimeout(() => {
    expiry = null;
    useReplyFields.getState().settle(useShellStore.getState().paneReplies, Date.now());
  }, Math.max(0, Math.min(...waits) - Date.now()));
}
useReplyFields.subscribe((state, before) => {
  if (state.fields !== before.fields) armExpiry();
});

useShellStore.subscribe((state, before) => {
  // A receipt settles its field whether or not the field is on screen.
  if (state.paneReplies !== before.paneReplies) useReplyFields.getState().settle(state.paneReplies, Date.now());
  // A pane that closed takes its field with it. Read only while the page is
  // live with agents listed, so a reconnect's empty first frame keeps the drafts.
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
export function settleReply(field: Pick<ReplyField, "text" | "sent">, receipt: PaneReplyReceipt | null, lost = false): Patch | null {
  if (!field.sent) return null;
  if (!receipt) return lost ? { sent: null, sentAt: null, refused: "unknown" } : null;
  if (receipt.request_id !== field.sent || receipt.state === "sending") return null;
  return receipt.state === "sent" ? { text: "", sent: null, sentAt: null, refused: null } : { sent: null, sentAt: null, refused: receipt.reason ?? "failed" };
}

/**
 * The reply field of `paneId` for the session `session` drew there: its
 * words, whether a reply is out, the refusal to show, `setText` (editing the
 * words clears a refusal) and `submit`.
 */
export function useAgentReply(paneId: string, session: string | null, send: (paneId: string, text: string, session: string | null) => string | null) {
  const held = useReplyFields((state) => state.fields[paneId]);
  const field = held && held.session === session ? held : null;
  return {
    text: field?.text ?? "",
    sending: (field?.sent ?? null) !== null,
    refused: field?.refused ?? null,
    setText: (text: string) => useReplyFields.getState().change(paneId, session, { text: text.slice(0, REPLY_MAX_LENGTH), refused: null }),
    submit: () => useReplyFields.getState().submit(paneId, session, send),
  };
}
