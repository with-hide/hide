// What the operator was writing in a session's input beside Desk, kept per
// pane while they open another session or leave Desk, as Desk's own draft is
// (`deskDraft` in `ui.ts`). The window's memory alone, for the app's life.

/** The words written in each pane's input, the one written in last at the end. */
export type SessionDrafts = readonly { pane: string; text: string }[];

/** At most this many panes keep their words; a new one past it lets the one written in longest ago go. */
export const SESSION_DRAFTS_KEPT = 32;

export const NO_SESSION_DRAFTS: SessionDrafts = [];

/** The words `pane`'s input holds, none when it holds nothing. */
export function sessionDraft(drafts: SessionDrafts, pane: string): string {
  return drafts.find((draft) => draft.pane === pane)?.text ?? "";
}

/** `pane`'s input now holds `text`: kept as the newest, or forgotten when empty (a send that landed clears it). */
export function keepSessionDraft(drafts: SessionDrafts, pane: string, text: string): SessionDrafts {
  const others = drafts.filter((draft) => draft.pane !== pane);
  if (text === "") return others.length === drafts.length ? drafts : others;
  const next = [...others, { pane, text }];
  return next.length > SESSION_DRAFTS_KEPT ? next.slice(next.length - SESSION_DRAFTS_KEPT) : next;
}
