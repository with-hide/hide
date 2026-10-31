// What Desk's composer does with the words the operator writes (PRD Desk 1차
// B7, B18, B48, B49): where an `@` word is, which sessions the `@` menu shows
// and how they fold, what picking writes, how a chip moves and goes, and what
// a send carries. The screen draws; these decide.
//
// The draft's text holds a picked mention as its own token,
// `@session(<pane> <name>)` or `@project(<path> <name>)`, which the box draws
// as one chip (D-85); the send carries the text as it is.

import { markedMention, type DeskMention } from "./desk";
import type { DeskMentions } from "./snapshot";
import type { DeskDraft } from "./ui";

export type MentionSession = DeskMentions["sessions"][number];
export type MentionProject = DeskMentions["projects"][number];

/** The `@` word the caret ends: where its `@` is and what follows it; none unless the caret is right after one that starts a word. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  if (!match) return null;
  const query = match[2]!;
  return { start: caret - query.length - 1, query };
}

/**
 * One session row of the `@` menu (D-84): how deep it is drawn, how many
 * children fold under it and whether they show, and, in the `하위 세션`
 * group a search makes, the parent it belongs to.
 */
export type MentionRow = { session: MentionSession; depth: number; children: number; open: boolean; crumb: string | null };

/** The `@` menu for what follows `@`: the session rows, the `하위 세션` group a search makes, and the projects. */
export type MentionTree = { rows: MentionRow[]; found: MentionRow[]; projects: MentionProject[] };

/**
 * The `@` menu's sessions as a tree (D-84, B48): parents only, each child
 * folded under its parent until opened, a parent opened from the start while
 * a session under it asks the operator (`folds` holds the operator's own
 * openings and closings, by pane). What follows `@` filters by name, case
 * aside: the parents that hold it first, then a `하위 세션` group of the
 * children that hold it, each with its parent's name, leaving out one an
 * opened parent already shows.
 */
export function mentionTree(section: DeskMentions, query: string, folds: Readonly<Record<string, boolean>>): MentionTree {
  const byId = new Map(section.sessions.map((session) => [session.id, session]));
  const kids = new Map<string, MentionSession[]>();
  for (const session of section.sessions) {
    if (session.parent === null) continue;
    const list = kids.get(session.parent) ?? [];
    list.push(session);
    kids.set(session.parent, list);
  }
  const asking = new Map<string, boolean>();
  const asks = (id: string): boolean => {
    const known = asking.get(id);
    if (known !== undefined) return known;
    asking.set(id, false);
    const answer = (kids.get(id) ?? []).some((child) => child.mark === "turn" || asks(child.id));
    asking.set(id, answer);
    return answer;
  };
  const rows: MentionRow[] = [];
  const walk = (session: MentionSession, depth: number) => {
    const children = kids.get(session.id) ?? [];
    const open = children.length > 0 && (folds[session.id] ?? asks(session.id));
    rows.push({ session, depth, children: children.length, open, crumb: null });
    if (open) for (const child of children) walk(child, depth + 1);
  };
  const needle = query.toLocaleLowerCase();
  const hit = (name: string) => name.toLocaleLowerCase().includes(needle);
  const roots = section.sessions.filter((session) => session.parent === null);
  for (const root of needle ? roots.filter((one) => hit(one.name)) : roots) walk(root, 0);
  const shown = new Set(rows.map((row) => row.session.id));
  const found = needle
    ? section.sessions
      .filter((session) => session.parent !== null && !shown.has(session.id) && hit(session.name))
      .map((session) => ({ session, depth: 0, children: 0, open: false, crumb: byId.get(session.parent!)?.name ?? null }))
    : [];
  return { rows, found, projects: section.projects.filter((project) => hit(project.name)) };
}

/** A mention picked into the box, where its token stands in the text. */
export type DraftChip = { start: number; end: number; mention: DeskMention };

/** Where each picked mention stands in the draft as its token, the longest token first so a name inside a longer one is not taken for it. */
export function draftChips(draft: DeskDraft): DraftChip[] {
  const tokens = draft.mentions.map((mention) => ({ mention, token: markedMention(mention) })).sort((a, b) => b.token.length - a.token.length);
  const chips: DraftChip[] = [];
  for (let at = draft.text.indexOf("@"); at >= 0; at = draft.text.indexOf("@", at + 1)) {
    const hit = tokens.find((one) => draft.text.startsWith(one.token, at));
    if (!hit) continue;
    chips.push({ start: at, end: at + hit.token.length, mention: hit.mention });
    at += hit.token.length - 1;
  }
  return chips;
}

/** The draft once a mention is picked for the `@` word: the word becomes the mention's token and one space, the caret goes after the space, and the mention is kept once. */
export function pickMention(draft: DeskDraft, at: { start: number; query: string }, mention: DeskMention): { draft: DeskDraft; caret: number } {
  const after = draft.text.slice(at.start + 1 + at.query.length);
  const token = markedMention(mention);
  const word = `${token}${/^\s/.test(after) ? "" : " "}`;
  const text = draft.text.slice(0, at.start) + word + after;
  const known = draft.mentions.some((one) => one.kind === mention.kind && one.id === mention.id);
  return { draft: { text, mentions: known ? draft.mentions : [...draft.mentions, mention] }, caret: at.start + token.length + 1 };
}

/** The draft with `start`..`end` replaced by `words`, and the caret after them. */
export function replaceRange(draft: DeskDraft, start: number, end: number, words: string): { draft: DeskDraft; caret: number } {
  return { draft: { ...draft, text: draft.text.slice(0, start) + words + draft.text.slice(end) }, caret: start + words.length };
}

/** ⌫ right after a chip, or ⌦ right before one, takes the whole chip out (B49); none when the caret is beside no chip on that side. */
export function chipDelete(draft: DeskDraft, caret: number, forward: boolean): { draft: DeskDraft; caret: number } | null {
  const chip = draftChips(draft).find((one) => (forward ? one.start === caret : one.end === caret));
  return chip ? replaceRange(draft, chip.start, chip.end, "") : null;
}

/** ← right after a chip, or → right before one, steps over it whole (D-85); none when the caret is beside no chip on that side. */
export function chipStep(draft: DeskDraft, caret: number, forward: boolean): number | null {
  const chip = draftChips(draft).find((one) => (forward ? one.start === caret : one.end === caret));
  return chip ? (forward ? chip.end : chip.start) : null;
}

/** What a send carries (B49): the words as written, each chip as its token, and the mentions whose chips are still there, in the order they stand; one the operator deleted is not sent. */
export function outgoing(draft: DeskDraft): { text: string; mentions: DeskMention[] } {
  const text = draft.text.trim();
  const mentions: DeskMention[] = [];
  for (const chip of draftChips({ ...draft, text })) {
    if (!mentions.some((one) => one.kind === chip.mention.kind && one.id === chip.mention.id)) mentions.push(chip.mention);
  }
  return { text, mentions };
}
