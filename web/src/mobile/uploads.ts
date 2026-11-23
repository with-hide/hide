// The files attached to the reply being written (Doop 08, 09): each one uploaded to the computer as soon as it is picked, one at a
// time, in `attach_chunk` frames of one phone message each (hided/src/mobile/uploads.rs owns the frames and their numbers), and sent
// with the reply's words as one `input` naming their stages. The computer answers every upload frame once, so the phone counts the
// frames of every kind it has out on the connection, whichever file they belong to, and keeps at most `WINDOW` unanswered: it never
// fills the eight frames hided takes from it at once, not even across a cancel and the next file. A lost connection takes the
// computer's copies with it (hided releases a connection's uploads no reply committed), so every file not yet sent is uploaded again
// under a new stage once the phone is back; the relay's daily limit ends the upload with its own line. A reply owns the files it
// named when it was sent: only those leave the bar once it lands, and a file picked while it was on its way stays; the reply is kept
// with its files' thumbnails for the conversation it went to (`sentReplies.ts`).

import { keyTag, type AgentKey, type PhoneMessage } from "./protocol";
import { withReply, type SentReply } from "./sentReplies";
import { patch, usePhone } from "./store";

/** The largest file one reply attaches, after a photo was made smaller (hided `uploads::FILE_BYTES`). */
export const FILE_BYTES = 10 * 1024 * 1024;
/** The bytes one `attach_chunk` carries (hided `uploads::CHUNK`). */
export const CHUNK = 12_000;
/** Upload frames out that the computer has not answered, of every kind and file (hided `uploads::WINDOW`). */
export const WINDOW = 4;
/** Files one reply attaches, and their bytes together: the attachment store's batch caps (hided `attachments::MAX_FILES`, `MAX_BATCH_BYTES`). */
export const MAX_FILES = 8;
export const BATCH_BYTES = 40 * 1024 * 1024;
/** The longest name the computer keeps for a file, in UTF-8 bytes (hided `attachments::safe_component`). */
const NAME_BYTES = 96;

/** A file on the reply bar: a photo shows its thumbnail, any other file its name and size. */
export type ComposerFile = {
  id: string;
  name: string;
  /** The bytes that leave the phone: a photo's after it was made smaller. */
  size: number;
  kind: "photo" | "file";
  /** A small JPEG of a photo, as a data URL; null for a file. */
  thumb: string | null;
  /** `preparing` while a photo is made smaller, `uploading` until the computer has every byte, `failed` with `reason`. */
  state: "preparing" | "uploading" | "ready" | "failed";
  /** The bytes the computer acknowledged. */
  sent: number;
  reason: string | null;
};

/** The one line under the files: what was left out, or why an upload stopped. */
export type FileNotice =
  | { kind: "too_large"; name: string; size: number }
  | { kind: "too_many" }
  | { kind: "too_much" }
  | { kind: "unreadable"; name: string }
  | { kind: "photo_failed"; name: string }
  | { kind: "daily_limit" }
  | { kind: "failed" };

/** A file as `addFile` takes it, once a photo was made smaller. */
export type PreparedFile = { name: string; bytes: Uint8Array; kind: "photo" | "file"; thumb: string | null };

/** A file of a sent reply, kept for its conversation on this phone. */
export type SentFile = { stage: string; name: string; size: number; kind: "photo" | "file"; thumb: string | null };

type Upload = { id: string; stage: string; bytes: Uint8Array; chunks: number; next: number; acked: number; ended: boolean };

let sender: (message: PhoneMessage) => boolean = () => false;
const held = new Map<string, Uint8Array>();
/** The stage each file was uploaded under on this connection. */
const stages = new Map<string, string>();
let active: Upload | null = null;
/** Upload frames sent on this connection that the computer has not answered yet. */
let unanswered = 0;
/** Stages to drop on the computer, sent ahead of any other upload frame once the window has room. */
const cancels: string[] = [];
let fileCounter = 0;

/** The connection's sender; set once by `connection.ts`. */
export function bindUploads(send: (message: PhoneMessage) => boolean): void {
  sender = send;
}

/** A random version-4 UUID, the stage shape the computer takes. */
export function newStage(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** A name the computer keeps as one file name: no separator or control character, at most `NAME_BYTES`, its extension kept. */
export function fileName(name: string): string {
  // eslint-disable-next-line no-control-regex
  const flat = name.replace(/[/\\\u0000-\u001f\u007f]/g, "_").trim() || "file";
  const safe = flat === "." || flat === ".." ? "file" : flat;
  const encoder = new TextEncoder();
  if (encoder.encode(safe).length <= NAME_BYTES) return safe;
  const dot = safe.lastIndexOf(".");
  const extension = dot > 0 && safe.length - dot <= 16 ? safe.slice(dot) : "";
  let stem = [...safe.slice(0, safe.length - extension.length)];
  while (stem.length > 0 && encoder.encode(stem.join("") + extension).length > NAME_BYTES) stem = stem.slice(0, -1);
  return stem.join("") + extension;
}

function files(): ComposerFile[] {
  return usePhone.getState().files;
}

function update(id: string, change: Partial<ComposerFile>): void {
  patch({ files: files().map((file) => (file.id === id ? { ...file, ...change } : file)) });
}

/** A slot on the bar for a file being read or made smaller; null when the reply already attaches as many as it can. */
export function reserveFile(name: string, kind: ComposerFile["kind"]): string | null {
  if (files().length >= MAX_FILES) {
    patch({ fileNotice: { kind: "too_many" } });
    return null;
  }
  fileCounter += 1;
  const id = `file-${fileCounter}`;
  patch({ files: [...files(), { id, name: fileName(name), size: 0, kind, thumb: null, state: "preparing", sent: 0, reason: null }] });
  return id;
}

/** Puts a prepared file in its slot and starts its upload; a file past `FILE_BYTES`, or past the reply's `BATCH_BYTES`, leaves the bar with one line, and the others stay. */
export function addFile(id: string, file: PreparedFile): void {
  const slot = files().find((candidate) => candidate.id === id);
  if (!slot) return;
  const name = fileName(file.name);
  if (file.bytes.length > FILE_BYTES) {
    patch({ files: files().filter((candidate) => candidate.id !== id), fileNotice: { kind: "too_large", name, size: file.bytes.length } });
    return;
  }
  const others = files().reduce((total, candidate) => total + (candidate.id === id ? 0 : candidate.size), 0);
  if (others + file.bytes.length > BATCH_BYTES) {
    patch({ files: files().filter((candidate) => candidate.id !== id), fileNotice: { kind: "too_much" } });
    return;
  }
  held.set(id, file.bytes);
  update(id, { name, size: file.bytes.length, kind: file.kind, thumb: file.thumb, state: "uploading", sent: 0 });
  pump();
}

/** A picked file that is not attached leaves the bar with the line that says why: too large before it was read, or unreadable. */
export function leaveFile(id: string, notice: FileNotice): void {
  patch({ files: files().filter((candidate) => candidate.id !== id), fileNotice: notice });
}

/** Drops a stage on the computer: its frames already out stay counted until their answers come. */
function cancel(stage: string | undefined): void {
  if (stage) cancels.push(stage);
}

/** The `×` of one file: only that file leaves, and the computer drops what it took of it. */
export function removeFile(id: string): void {
  if (active?.id === id) active = null;
  cancel(stages.get(id));
  stages.delete(id);
  held.delete(id);
  patch({ files: files().filter((file) => file.id !== id) });
  pump();
}

/** A failed file pressed again: its upload starts over under a new stage. */
export function retryFile(id: string): void {
  const file = files().find((candidate) => candidate.id === id);
  if (!file || file.state !== "failed" || !held.has(id)) return;
  stages.delete(id);
  update(id, { state: "uploading", sent: 0, reason: null });
  if (usePhone.getState().fileNotice?.kind !== "too_large") patch({ fileNotice: null });
  pump();
}

/** Every file leaves the bar, and the computer drops what it took: the detail closed, or the computer no longer takes files. */
export function dropFiles(): void {
  for (const id of held.keys()) cancel(stages.get(id));
  active = null;
  held.clear();
  stages.clear();
  if (files().length > 0 || usePhone.getState().fileNotice) patch({ files: [], fileNotice: null });
  pump();
}

/** The files a reply names, in the bar's order, by what was picked and by the stage each is uploaded under, once the computer has every one; null while any still uploads or failed. */
export function replyFiles(): { ids: string[]; stages: string[] } | null {
  const list = files();
  if (list.some((file) => file.state !== "ready")) return null;
  const named = list.map((file) => stages.get(file.id));
  return named.every((stage): stage is string => typeof stage === "string") ? { ids: list.map((file) => file.id), stages: named } : null;
}

/** Sends one upload frame, counted until the computer answers it. */
function sendUpload(message: PhoneMessage): boolean {
  if (!sender(message)) return false;
  unanswered += 1;
  return true;
}

/** Sends what the window allows: the cancels first, then the upload in front, starting the next file when the last one is in. */
function pump(): void {
  if (!usePhone.getState().connected) return;
  while (cancels.length > 0 && unanswered < WINDOW) {
    if (!sendUpload({ type: "attach_cancel", stage: cancels[0]! })) return;
    cancels.shift();
  }
  if (cancels.length > 0 || unanswered >= WINDOW) return;
  if (!active) {
    const next = files().find((file) => file.state === "uploading");
    const bytes = next ? held.get(next.id) : undefined;
    if (!next || !bytes) return;
    const stage = newStage();
    stages.set(next.id, stage);
    active = { id: next.id, stage, bytes, chunks: Math.ceil(bytes.length / CHUNK), next: 0, acked: 0, ended: false };
    if (!sendUpload({ type: "attach_begin", stage, name: next.name, size: bytes.length })) {
      active = null;
      return;
    }
  }
  const upload = active;
  while (upload.next < upload.chunks && unanswered < WINDOW) {
    const start = upload.next * CHUNK;
    const data = base64(upload.bytes.subarray(start, start + CHUNK));
    if (!sendUpload({ type: "attach_chunk", stage: upload.stage, seq: upload.next, data })) return;
    upload.next += 1;
  }
  if (upload.next === upload.chunks && !upload.ended && unanswered < WINDOW) {
    if (!sendUpload({ type: "attach_end", stage: upload.stage })) return;
    upload.ended = true;
  }
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let at = 0; at < bytes.length; at += 4096) binary += String.fromCharCode(...bytes.subarray(at, at + 4096));
  return btoa(binary);
}

/**
 * The computer's answer to an upload frame, one for each: it frees a place in the window whichever file it names; for the upload in
 * front a chunk on disk moves the bar's band, the end makes the file ready, a refusal fails it.
 */
export function onAttachAck(frame: { stage: string; seq?: number; begun?: boolean; done?: boolean; cancelled?: boolean; failed?: string }): void {
  unanswered = Math.max(0, unanswered - 1);
  const upload = active;
  if (!upload || frame.stage !== upload.stage) {
    pump();
    return;
  }
  if (frame.failed) {
    active = null;
    update(upload.id, { state: "failed", reason: frame.failed });
    patch({ fileNotice: { kind: "failed" } });
    pump();
    return;
  }
  if (frame.done) {
    active = null;
    update(upload.id, { state: "ready", sent: upload.bytes.length });
    pump();
    return;
  }
  if (typeof frame.seq === "number" && frame.seq + 1 > upload.acked) {
    upload.acked = frame.seq + 1;
    update(upload.id, { sent: Math.min(upload.acked * CHUNK, upload.bytes.length) });
  }
  pump();
}

/** The computer said hello: the uploads of the files not yet sent start, or start again. */
export function uploadsConnected(): void {
  pump();
}

/**
 * The connection is gone, and with it what the computer held of this phone's uploads: every file not yet sent is uploaded again
 * once the phone is back. The relay's daily limit (`daily_limit`) ends the uploads instead, with its line: no retry gets past it
 * until the relay's day turns.
 */
export function uploadsLost(reason: string | null): void {
  active = null;
  unanswered = 0;
  cancels.length = 0;
  stages.clear();
  if (files().length === 0) return;
  if (reason === "daily_limit") {
    patch({ files: files().map((file) => (file.state === "preparing" ? file : { ...file, state: "failed", sent: 0, reason })), fileNotice: { kind: "daily_limit" } });
    return;
  }
  patch({ files: files().map((file) => (file.state === "ready" || file.state === "uploading" ? { ...file, state: "uploading", sent: 0 } : file)) });
}

/** The reply naming these files was refused because the computer no longer holds one of them: every file is uploaded again. */
export function reuploadFiles(): void {
  active = null;
  for (const stage of stages.values()) cancel(stage);
  stages.clear();
  patch({ files: files().map((file) => (file.state === "preparing" || !held.has(file.id) ? file : { ...file, state: "uploading", sent: 0, reason: null })) });
  pump();
}

/**
 * The reply that named the files `ids` landed in `key`'s conversation: those files leave the bar, and the reply is kept with their
 * thumbnails under the stages it named (`sentReplies.ts`). A file picked while the reply was on its way is not the reply's, and stays.
 */
export function filesSent(key: AgentKey, ids: readonly string[]): void {
  const mine = files().filter((file) => ids.includes(file.id));
  if (mine.length === 0) return;
  const reply: SentReply = {
    tag: keyTag(key),
    files: mine.map((file) => ({ stage: stages.get(file.id) ?? "", name: file.name, size: file.size, kind: file.kind, thumb: file.thumb })),
  };
  patch({ sent: withReply(usePhone.getState().sent, reply) });
  for (const id of ids) {
    held.delete(id);
    stages.delete(id);
  }
  const left = files().filter((file) => !ids.includes(file.id));
  patch(left.length > 0 ? { files: left } : { files: [], fileNotice: null });
}

/** Forgets what this module holds; for tests. */
export function resetUploads(): void {
  active = null;
  unanswered = 0;
  cancels.length = 0;
  held.clear();
  stages.clear();
  patch({ sent: [] });
  sender = () => false;
}
