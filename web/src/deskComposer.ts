// What Desk's composer does with the words the operator writes (PRD Desk 1차
// B7, B18): where an `@` word is, what picking from the `@` menu writes, and
// what a send carries. The screen draws; these decide.

import { markedMention, type DeskMention } from "./desk";
import type { DeskMentions } from "./snapshot";
import type { DeskDraft } from "./ui";

/** The `@` word the caret ends: where its `@` is and what follows it; none unless the caret is right after one that starts a word. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  if (!match) return null;
  const query = match[2]!;
  return { start: caret - query.length - 1, query };
}

/** The `@` menu's rows for what follows `@`: sessions and projects whose names hold it, case aside. */
export function mentionRows(section: DeskMentions, query: string): DeskMentions {
  const needle = query.toLocaleLowerCase();
  const hit = (name: string) => name.toLocaleLowerCase().includes(needle);
  return { sessions: section.sessions.filter((session) => hit(session.name)), projects: section.projects.filter((project) => hit(project.name)) };
}

/** The draft once a mention is picked for the `@` word: the word becomes `@name` and one space, the caret goes after the space, and the mention is kept once. */
export function pickMention(draft: DeskDraft, at: { start: number; query: string }, mention: DeskMention): { draft: DeskDraft; caret: number } {
  const after = draft.text.slice(at.start + 1 + at.query.length);
  const word = `@${mention.name}${/^\s/.test(after) ? "" : " "}`;
  const text = draft.text.slice(0, at.start) + word + after;
  const known = draft.mentions.some((one) => one.kind === mention.kind && one.id === mention.id);
  return { draft: { text, mentions: known ? draft.mentions : [...draft.mentions, mention] }, caret: at.start + mention.name.length + 2 };
}

const AFTER = /[\s,.!?;:)]/;

/** Where `@name` stands as a word of its own: at the start or after a space, and followed by the end, a space or punctuation. */
function wordAt(text: string, word: string): number {
  for (let at = text.indexOf(word); at >= 0; at = text.indexOf(word, at + 1)) {
    const before = at === 0 || /\s/.test(text[at - 1]!);
    const end = at + word.length;
    if (before && (end === text.length || AFTER.test(text[end]!))) return at;
  }
  return -1;
}

/**
 * What a send carries (B18): each picked mention still written as `@name`
 * becomes its token where it is first written, longest name first so a name
 * inside a longer one is not taken for it; one the operator deleted is not
 * sent.
 */
export function outgoing(draft: DeskDraft): { text: string; mentions: DeskMention[] } {
  let text = draft.text.trim();
  const mentions: DeskMention[] = [];
  for (const mention of [...draft.mentions].sort((a, b) => b.name.length - a.name.length)) {
    const word = `@${mention.name}`;
    const at = wordAt(text, word);
    if (at < 0) continue;
    text = text.slice(0, at) + markedMention(mention) + text.slice(at + word.length);
    mentions.push(mention);
  }
  return { text, mentions };
}
