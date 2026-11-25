// The replies this phone sent and the operator's message the agent recorded for each (Doop 09): a reply's files stand over that message
// with this phone's thumbnails, on this phone only. A reply is found by an upload id the message names, and by nothing else: a path the
// agent kept in the words, or a file the agent recorded beside them by its path (`files`, as Codex does around an image). An upload id
// names one upload, so one reply; a message with the same words or other photos names none of its ids and never takes its files. A
// message whose images the agent kept without any path names no upload: on every phone each image is a photo with no name and no
// thumbnail, never tied to an upload by its order or its count. Values only: the store holds the replies and `uploads.ts` adds one as it
// lands.

import { keyTag, type AgentKey, type ConversationMessage } from "./protocol";
import type { SentFile } from "./uploads";

/** How many sent replies this phone keeps; an older one's files are named from its paths alone. */
export const SENT_REPLIES = 32;

/** A sent reply: the agent's conversation it went to (`keyTag`) and its files. */
export type SentReply = { tag: string; files: SentFile[] };

/**
 * A file an operator's message names: the name the operator picked and the stage it was uploaded under, when it was; `name` null is an
 * image the agent recorded with no path, so with no name.
 */
export type NamedFile = { name: string | null; stage: string | null };

const STAGE_NAME = /^([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})-(.+)$/;

/** A file by the last part of its path: a staged file by the name the operator picked and its stage. */
function namedFile(base: string): NamedFile {
  const staged = STAGE_NAME.exec(base);
  return staged ? { name: staged[2]!, stage: staged[1]! } : { name: base, stage: null };
}

/**
 * The files an operator's message attached, read from the front of its words: each a path in double quotes, as the computer types an
 * attached file, then the words, with each file's path as written. A message that does not start with one is all words.
 */
export function attachedOf(text: string): { files: NamedFile[]; paths: string[]; words: string } {
  const named: NamedFile[] = [];
  const paths: string[] = [];
  let rest = text;
  for (;;) {
    const match = /^"((?:[^"\\]|\\.)*)"(?: |$)/.exec(rest);
    if (!match) break;
    const path = match[1]!.replace(/\\(.)/g, "$1");
    if (!/^(?:\/|[A-Za-z]:[\\/])/.test(path)) break;
    paths.push(path);
    named.push(namedFile(path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1)));
    rest = rest.slice(match[0].length);
  }
  return { files: named, paths, words: named.length > 0 ? rest : text };
}

/** The replies with `reply` added as the newest, the oldest past `SENT_REPLIES` gone. */
export function withReply(sent: readonly SentReply[], reply: SentReply): SentReply[] {
  return [...sent, reply].slice(-SENT_REPLIES);
}

/**
 * The files an operator's message of `key`'s conversation attached, as this phone draws them: a reply this phone sent, found by an
 * upload id the message names, with all its files and their thumbnails; any other message's files by their names alone; and where it
 * names no file, each image the agent recorded as a photo with no name.
 */
export function shownFiles(sent: readonly SentReply[], key: AgentKey, message: ConversationMessage): { files: (SentFile | NamedFile)[]; words: string } {
  const { files: inWords, words } = attachedOf(message.text);
  const named = [...inWords, ...(message.files ?? []).map(namedFile)];
  const stages = named.flatMap((file) => (file.stage ? [file.stage] : []));
  const tag = keyTag(key);
  const reply = stages.length > 0 ? sent.find((reply) => reply.tag === tag && reply.files.some((file) => stages.includes(file.stage))) : undefined;
  if (reply) return { files: reply.files, words };
  // The agent's own limit on a message's images bounds them (Claude takes 100 at most).
  if (named.length === 0 && message.images) return { files: Array.from({ length: message.images }, () => ({ name: null, stage: null })), words };
  return { files: named, words };
}
