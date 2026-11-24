// What Desk's composer does with the words the operator writes (PRD Desk 1차
// B7, B18, B48, B49): where an `@` word is, which sessions the `@` menu shows
// and how they fold, what picking writes, how a chip moves and goes, and what
// a send carries. The screen draws; these decide.
//
// The draft's text holds a picked mention as its own token,
// `@session(<pane> <name>)` or `@project(<path> <name>)`, which the box draws
// as one chip (D-85); the send carries the text as it is.

import { useEffect, useRef, useState } from "react";
import { MAX_FILE_BYTES, safeName, type AttachmentInput } from "./attachments";
import { markedMention, type DeskMention } from "./desk";
import { attachedOf } from "./mobile/sentReplies";
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

// Attachments (PRD desk-2-ui D-27, B41, B42): what a composer's tray holds,
// for Desk's composer and a session's alike. A dropped, pasted or picked file
// joins the tray at once; one past the per-file cap stays in it red and is
// never read, and the rest are read in with their progress, then sent with
// the words. The tray is the screen's own until a send carries it.

/** A file in a composer's tray. */
export type TrayFile = {
  id: string;
  name: string;
  size: number;
  /** An image's thumbnail (an object URL the tray owns), else null. */
  preview: string | null;
  /** `reading` while its bytes come in; `too_large` past the cap, never read; `unreadable` when the system would not give its bytes. Neither of the last two is sent. */
  state: "reading" | "ready" | "too_large" | "unreadable";
  /** Bytes read in so far. */
  read: number;
};

/** A file as the tray first takes it: its state from its size alone, before a byte is read (B42). */
export function trayEntry(file: { name: string; size: number }, id: string, preview: string | null): TrayFile {
  const tooLarge = file.size > MAX_FILE_BYTES;
  return { id, name: file.name, size: file.size, preview: tooLarge ? null : preview, state: tooLarge ? "too_large" : "reading", read: 0 };
}

/**
 * The files a send carries, in tray order, or null while the tray holds the
 * send: a file still being read, or one past the cap or unreadable, which is
 * never sent and holds the rest until the operator takes it out (B42).
 */
export function traySendable(files: readonly TrayFile[]): TrayFile[] | null {
  return files.every((file) => file.state === "ready") ? [...files] : null;
}

/** Whether a drag carries files from the system, not text or one of Hide's own rows. */
export function dragHasFiles(types: readonly string[]): boolean {
  return types.includes("Files");
}

export type ComposerTray = {
  files: readonly TrayFile[];
  add: (files: readonly File[]) => void;
  remove: (id: string) => void;
  /** The files' names and bytes, each picture also as an image the sent bubble can draw, or null while the tray holds the send (`traySendable`). */
  outgoing: () => (AttachmentInput & { type: string; image: Blob | null })[] | null;
  clear: () => void;
  /** Puts files a send carried back as they were read, ready to send again, when the message did not reach its agent. */
  restore: (files: readonly (AttachmentInput & { type: string })[]) => void;
};

/**
 * A composer's tray. Each image gets a thumbnail URL the tray owns and lets
 * go when the file leaves; each file under the cap is read in once, in
 * chunks, so its tile shows the progress; a file removed while it reads is
 * read no further.
 */
export function useComposerTray(): ComposerTray {
  const [files, setFiles] = useState<TrayFile[]>([]);
  const bytes = useRef(new Map<string, { bytes: Uint8Array; type: string }>());
  const live = useRef(files);
  live.current = files;
  const release = (file: TrayFile) => {
    bytes.current.delete(file.id);
    if (file.preview) URL.revokeObjectURL(file.preview);
  };
  useEffect(() => () => live.current.forEach(release), []);
  const update = (id: string, change: Partial<TrayFile>) => setFiles((current) => current.map((file) => (file.id === id ? { ...file, ...change } : file)));
  const read = async (id: string, file: File) => {
    const parts: Uint8Array[] = [];
    let total = 0;
    try {
      const reader = file.stream().getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (!live.current.some((one) => one.id === id)) {
          await reader.cancel();
          return;
        }
        parts.push(value);
        total += value.byteLength;
        update(id, { read: total });
      }
    } catch {
      // It stays, red, holding the send until the operator takes it out, so no file goes missing from a message unseen.
      update(id, { state: "unreadable" });
      return;
    }
    if (!live.current.some((one) => one.id === id)) return;
    const whole = new Uint8Array(total);
    let at = 0;
    for (const part of parts) {
      whole.set(part, at);
      at += part.byteLength;
    }
    bytes.current.set(id, { bytes: whole, type: file.type });
    update(id, { state: "ready", read: total });
  };
  return {
    files,
    add: (picked) => {
      const entries = picked.map((file) => ({ file, entry: trayEntry(file, crypto.randomUUID(), file.type.startsWith("image/") && file.size <= MAX_FILE_BYTES ? URL.createObjectURL(file) : null) }));
      setFiles((current) => [...current, ...entries.map(({ entry }) => entry)]);
      for (const { file, entry } of entries) if (entry.state === "reading") void read(entry.id, file);
    },
    remove: (id) => setFiles((current) => current.filter((file) => file.id !== id || (release(file), false))),
    outgoing: () => {
      const ready = traySendable(live.current);
      if (!ready) return null;
      return ready.map((file) => {
        const read = bytes.current.get(file.id)!;
        return { name: safeName(file.name), bytes: read.bytes, type: read.type, image: file.preview ? new Blob([read.bytes as BlobPart], { type: read.type }) : null };
      });
    },
    clear: () => {
      live.current.forEach(release);
      setFiles([]);
    },
    restore: (sent) => {
      const entries = sent.map((file) => {
        const id = crypto.randomUUID();
        bytes.current.set(id, { bytes: file.bytes, type: file.type });
        const preview = file.type.startsWith("image/") ? URL.createObjectURL(new Blob([file.bytes as BlobPart], { type: file.type })) : null;
        return { id, name: file.name, size: file.bytes.byteLength, preview, state: "ready" as const, read: file.bytes.byteLength };
      });
      setFiles((current) => [...current, ...entries]);
    },
  };
}

/**
 * The files this window sent, by the stage each was sent under: the name the
 * operator's file had and, for a picture, a thumbnail, so a sent bubble draws
 * them while the page lives (B41). A file a device staged under a name Hide
 * made (`hide-<request>-<index>.<ext>`, `hide-host`'s staging) is found by the
 * same stage. A bubble whose stage is not here, after a reload or for a file
 * sent from another window, draws the name its path has and no picture. Each
 * URL is the registry's own, and the oldest go past `SENT_KEPT`.
 */
const SENT_KEPT = 32;
const sentFiles = new Map<string, { name: string; preview: string | null }>();

export function keepSent(stage: string, name: string, image: Blob | null): void {
  sentFiles.set(stage, { name, preview: image ? URL.createObjectURL(image) : null });
  while (sentFiles.size > SENT_KEPT) {
    const [oldest, kept] = sentFiles.entries().next().value!;
    if (kept.preview) URL.revokeObjectURL(kept.preview);
    sentFiles.delete(oldest);
  }
}

/** A sent file named by its staged path or its stage: the name the operator's file had, and this window's thumbnail of it when it kept one. */
export type SentAttachment = { name: string; size: number | null; preview: string | null };

const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
/** The staged file name hided makes: `<stage>-<name>`, the stage a UUID and, from a composer, the file's place in its send. */
const STAGED = new RegExp(`^(${UUID}(?:-\\d+)?)-(.+)$`);
/** The name a device's staging makes: `hide-<request>-<index>` and the original's extension. */
const DEVICE_STAGED = new RegExp(`^hide-(${UUID}-\\d+)(?:\\.[^.]*)?$`);

/** A sent file from the last part of its path, with the name and thumbnail this window kept for its stage. */
export function sentAttachment(path: string, size: number | null = null): SentAttachment {
  const base = path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
  const staged = STAGED.exec(base);
  const stage = staged?.[1] ?? DEVICE_STAGED.exec(base)?.[1];
  const kept = stage ? sentFiles.get(stage) : undefined;
  return { name: kept?.name ?? staged?.[2] ?? base, size, preview: kept?.preview ?? null };
}

/** An operator's words in a session's record and the files a composer put before them as quoted paths, read as the phone reads them (`attachedOf`). */
export function sentWords(text: string): { files: SentAttachment[]; words: string } {
  const { paths, words } = attachedOf(text);
  return { files: paths.map((path) => sentAttachment(path)), words };
}
