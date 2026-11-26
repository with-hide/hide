// The preview a Desk tile shows for a file (feedback 2 frame M [1]): an
// image's picture, a PDF's first page and page count, a video's first frame
// and length, a Markdown or text file's title and first three lines, a page's
// still; and, when none can be made, why.
//
// Where and when: a preview is made in this window when a tile first shows
// its file, from bytes read over the file channel (`fileBytes.ts`), which a
// connected device's node answers by reading the file there; the node
// carries no PDF or video renderer, so the bytes come here and only the
// small result is kept. It is made once and kept for the window's life, as a
// small JPEG (`previewDraw.ts`), until the file changes on disk
// (`fileChanges`) or the operator asks again (`다시`).
//
// Bounds (engineering principle 15): at most `KEPT` previews are kept, the
// least recently shown let go first; at most `MAKING` are made at once and the
// rest wait their turn; a text file is read only to `TEXT_HEAD`; a file larger
// than its kind's cap is not read past its first byte and says so with its
// size; a video that does not give its first frame in time counts as
// unreadable, and a preview with no answer in `MAKE_WAIT_MS` frees its turn.
// A file on a device that is not connected is not asked for.

import { requestFileBytesWithTotal, type FileSource } from "./fileBytes";
import { IMAGE_BYTES } from "./imageFile";
import { drawPreview } from "./previewDraw";
import { useShellStore } from "./store";
import { isVideoPath } from "./viewers/video";

/** The kinds a tile draws a preview for; any other file is its kind icon and name. */
export type PreviewKind = "image" | "pdf" | "video" | "text";

/** What a tile shows: a picture (with a PDF's pages or a video's seconds), a text's head, or why there is none. */
export type Preview =
  | { state: "reading" }
  | { state: "picture"; picture: string; pages: number | null; seconds: number | null }
  | { state: "text"; title: string; lines: string[] }
  | { state: "failed" }
  | { state: "too_large"; bytes: number }
  | { state: "offline" }
  | { state: "none" };

/** Previews kept at once, the least recently shown dropped first. */
export const KEPT = 64;
/** Previews made at once; the rest wait in order. */
export const MAKING = 2;
/** How long one preview may take, its reads and its drawing, before it counts as unreadable. */
export const MAKE_WAIT_MS = 30_000;
/** How much of a text file is read for its title and lines. */
export const TEXT_HEAD = 4096;
/** The largest file read whole for each kind's preview; an image's is the picture cap every image tile keeps (`imageFile.ts`). */
export const CAP: Record<Exclude<PreviewKind, "text">, number> = {
  image: IMAGE_BYTES,
  pdf: 32 * 1024 * 1024,
  video: 64 * 1024 * 1024,
};

const IMAGE = /\.(png|jpe?g|gif|webp|bmp|avif|svg)$/i;
const PDF = /\.pdf$/i;
const TEXT = /\.(md|markdown|txt)$/i;

export function previewKind(path: string): PreviewKind | null {
  if (IMAGE.test(path)) return "image";
  if (PDF.test(path)) return "pdf";
  if (isVideoPath(path)) return "video";
  if (TEXT.test(path)) return "text";
  return null;
}

/** A Markdown or text file's head as a tile shows it: its heading or first line, and the three lines after it, marks left out. */
export function textHead(text: string): { title: string; lines: string[] } {
  const body = text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "");
  const lines = body.split(/\r?\n/).map((line) => line.replace(/^\s*(#{1,6}\s+|[-*+]\s+(\[[ xX]\]\s+)?|>\s?|\d+[.)]\s+)/, "").trim()).filter((line) => line !== "" && !/^(```|~~~|---+|\*\*\*+)$/.test(line));
  return { title: lines[0] ?? "", lines: lines.slice(1, 4) };
}

type Entry = { preview: Preview; listeners: Set<(preview: Preview) => void> };
type Make = { key: string; path: string; source: FileSource };
const kept = new Map<string, Entry>();
const waiting: Make[] = [];
let running = 0;
/** Bumped by `clearPreviews`, so a make from before it neither frees a slot nor settles. */
let generation = 0;

function pump(): void {
  while (running < MAKING && waiting.length > 0) {
    const next = waiting.shift()!;
    // A preview let go before its turn came is not made.
    if (!kept.has(next.key)) continue;
    running += 1;
    const started = generation;
    const stop = new AbortController();
    void within(make(next.path, next.source, stop.signal), MAKE_WAIT_MS, stop).then((preview) => {
      if (started !== generation) return;
      running -= 1;
      settle(next.key, preview);
      pump();
    });
  }
}

/**
 * `work`'s answer, or `failed` once `ms` pass without one: a read nothing
 * answers (a device gone quiet) never holds a slot for good. A make past its
 * time is stopped: a PDF's pdf.js task is destroyed and a video's load given
 * up at once, and a read it still waits on is followed by nothing more.
 */
function within(work: Promise<Preview>, ms: number, stop: AbortController): Promise<Preview> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      stop.abort();
      useShellStore.getState().noteDiagnostic(`desk preview: no answer within ${ms} ms`);
      resolve({ state: "failed" });
    }, ms);
    void work.then((preview) => {
      clearTimeout(timer);
      resolve(preview);
    });
  });
}

function settle(key: string, preview: Preview): void {
  const entry = kept.get(key);
  if (!entry) return;
  entry.preview = preview;
  for (const listener of entry.listeners) listener(preview);
}

/** Lets the least recently shown previews go past `KEPT`; one a tile shows now stays. */
function trim(): void {
  for (const [key, entry] of kept) {
    if (kept.size <= KEPT) return;
    if (entry.listeners.size === 0) kept.delete(key);
  }
}

/**
 * The preview of `path` on the machine `source` names, made once per `key`
 * (which names the file and its version) and told to `listener` as it
 * changes; returns what is known now and the unsubscribe.
 */
export function watchPreview(key: string, path: string, source: FileSource, listener: (preview: Preview) => void): { now: Preview; stop: () => void } {
  let entry = kept.get(key);
  if (entry) {
    // Shown again: it moves to the newest end.
    kept.delete(key);
    kept.set(key, entry);
  } else {
    entry = { preview: { state: "reading" }, listeners: new Set() };
    kept.set(key, entry);
    waiting.push({ key, path, source });
    pump();
  }
  entry.listeners.add(listener);
  trim();
  const held = entry;
  return {
    now: held.preview,
    stop: () => {
      held.listeners.delete(listener);
      trim();
    },
  };
}

/** Forgets one preview, so the next tile to show it makes it again (`다시`). */
export function forgetPreview(key: string): void {
  kept.delete(key);
}

/** Forgets every preview and waiting make (tests). */
export function clearPreviews(): void {
  kept.clear();
  waiting.length = 0;
  running = 0;
  generation += 1;
}

async function make(path: string, source: FileSource, signal: AbortSignal): Promise<Preview> {
  const kind = previewKind(path);
  if (kind === null) return { state: "none" };
  try {
    if (kind === "text") {
      const { bytes } = await requestFileBytesWithTotal(path, { length: TEXT_HEAD }, source);
      if (bytes.includes(0)) return { state: "failed" };
      return { state: "text", ...textHead(new TextDecoder().decode(bytes)) };
    }
    const head = await requestFileBytesWithTotal(path, { length: 1 }, source);
    if (signal.aborted) return { state: "failed" };
    if (head.total > CAP[kind]) return { state: "too_large", bytes: head.total };
    const { bytes } = await requestFileBytesWithTotal(path, undefined, source);
    if (signal.aborted) return { state: "failed" };
    return await drawPreview(kind, bytes, path, signal);
  } catch {
    return { state: "failed" };
  }
}
