import { useEffect } from "react";
import { create } from "zustand";
import type { SessionThreadInput } from "./snapshot";
import { useShellStore } from "./store";

/**
 * The Overview's reply to an agent without opening its session (the core's
 * `pane_reply`). One field per pane, shared by the 나를 기다리는 것 item and
 * the session detail, so the words follow the agent between them: the words
 * not yet taken, the request out, and why the last one was refused. The
 * words stay until the core says the pane took them.
 */
export type ReplyField = { text: string; sent: string | null; refused: string | null };

const EMPTY: ReplyField = { text: "", sent: null, refused: null };

type ReplyFields = {
  fields: Record<string, ReplyField>;
  change: (paneId: string, patch: Partial<ReplyField>) => void;
};

export const useReplyFields = create<ReplyFields>((set) => ({
  fields: {},
  change: (paneId, patch) =>
    set((state) => {
      const next = { ...(state.fields[paneId] ?? EMPTY), ...patch };
      const fields = { ...state.fields };
      // A field with nothing in it holds nothing.
      if (!next.text && !next.sent && !next.refused) delete fields[paneId];
      else fields[paneId] = next;
      return { fields };
    }),
}));

/**
 * What the core's answer does to a field: nothing until it names the
 * field's own request and has settled; then `sent` empties the field and
 * `refused` keeps the words with the reason.
 */
export function settleReply(field: ReplyField, answer: SessionThreadInput | null): ReplyField | null {
  if (!field.sent || answer?.request_id !== field.sent || answer.state === "sending") return null;
  return answer.state === "sent" ? EMPTY : { text: field.text, sent: null, refused: answer.reason ?? "failed" };
}

/**
 * The reply field of `paneId`: its words, whether a reply is out, the
 * refusal to show, and `submit`, which sends the words through `send` (one
 * `pane_reply`). A send that never left the page is refused as `offline`.
 */
export function useAgentReply(paneId: string, send: (paneId: string, text: string) => string | null) {
  const field = useReplyFields((state) => state.fields[paneId]) ?? EMPTY;
  const change = useReplyFields((state) => state.change);
  const answer = useShellStore((state) => state.paneReplies?.replies[paneId] ?? null);
  useEffect(() => {
    const settled = settleReply(field, answer);
    if (settled) change(paneId, settled);
  }, [answer, field, paneId, change]);
  const sending = field.sent !== null;
  return {
    text: field.text,
    sending,
    refused: field.refused,
    setText: (text: string) => change(paneId, { text }),
    submit: () => {
      if (sending || !field.text.trim()) return;
      const request = send(paneId, field.text);
      change(paneId, request ? { sent: request, refused: null } : { refused: "offline" });
    },
  };
}
