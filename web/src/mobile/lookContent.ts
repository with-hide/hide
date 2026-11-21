// What the phone makes of one View's content (PRD mobile-companion-ui B13-B19, B23, D-10): the chunks hided
// sends under a flow-controlled window put back into text or an image, and the states that are not content.
// Pure, so every rule is tested without a socket (hided/src/mobile/looks.rs is the sending side).

import type { LookFile, LookHead, LookResultState } from "./protocol";

/** The caps hided enforces (D-10); a head that claims more is refused, so the phone never holds more than a cap. */
export const LOOK_TEXT_CAP = 1024 * 1024;
export const LOOK_IMAGE_CAP = 10 * 1024 * 1024;
/** More chunks than the largest cap can make, at the smallest chunk hided may use. */
const MAX_CHUNKS = 1024;

/** What the viewer draws for one View: the transfer's progress, its content, or the reason there is none. */
export type LookContent =
  | { state: "loading" }
  | { state: "text"; markdown: boolean; text: string; truncated: boolean }
  | { state: "image"; url: string; size: number }
  | { state: "changes"; files: LookFile[]; count: number; added: number; removed: number }
  /** A page the desktop app showed, as it looked when its device took it (B20). */
  | { state: "shot"; url: string; size: number; address: string; takenAt: number; takenBy: string }
  /** An HTML file of a checkout as one document, the files it names folded in, drawn in the sandboxed frame. */
  | { state: "html"; document: string; size: number }
  /** A page whose screen cannot be taken now: its address alone, and refresh still works (B21). */
  | { state: "page"; url: string }
  /** Nothing the phone can show: a PDF, a video, another binary (B19). */
  | { state: "other" }
  | { state: "too_large" }
  /** The View is not open any more (B27). */
  | { state: "closed" }
  | { state: "device_unreachable" }
  | { state: "failed" };

/** The state a `look_result` stands for; a refusal or an agent that left is a failed read to the reader of the screen. */
export function resultContent(state: LookResultState, url: string | undefined): LookContent {
  switch (state) {
    case "page":
      return { state: "page", url: url ?? "" };
    case "other":
    case "too_large":
    case "closed":
    case "device_unreachable":
      return { state };
    default:
      return { state: "failed" };
  }
}

/** The chunks of one read as they arrive, in order. */
export type Assembly = {
  requestId: string;
  viewId: string;
  mode: LookHead["mode"];
  /** A page's address, when it was taken and by which device; null for any other read. */
  page: { address: string; takenAt: number; takenBy: string } | null;
  size: number;
  truncated: boolean;
  chunks: number;
  parts: string[];
  /** Bytes received so far, counted from the base64 text. */
  bytes: number;
};

/** The bytes a base64 text carries. */
function decodedLength(data: string): number {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.floor((data.length * 3) / 4) - padding;
}

/** Starts a read from its head; null when the head claims more than its cap or contradicts itself. */
export function beginAssembly(head: LookHead): Assembly | null {
  const cap = head.mode === "text" || head.mode === "markdown" ? LOOK_TEXT_CAP : LOOK_IMAGE_CAP;
  const sane = Number.isSafeInteger(head.size) && head.size >= 0 && head.size <= cap && Number.isSafeInteger(head.chunks) && head.chunks >= 0 && head.chunks <= MAX_CHUNKS;
  if (!sane || (head.size === 0) !== (head.chunks === 0)) return null;
  // A screenshot says when and by whom it was taken, or it is not one.
  const page = head.mode === "page" ? pageFacts(head) : null;
  if (head.mode === "page" && !page) return null;
  return { requestId: head.request_id, viewId: head.view_id, mode: head.mode, page, size: head.size, truncated: head.truncated, chunks: head.chunks, parts: [], bytes: 0 };
}

function pageFacts(head: LookHead): Assembly["page"] {
  const { url, taken_at: takenAt, taken_by: takenBy } = head;
  return typeof url === "string" && Number.isSafeInteger(takenAt) && typeof takenBy === "string" ? { address: url, takenAt: takenAt as number, takenBy } : null;
}

/** Takes the next chunk; null when it is out of order or the read already holds more than its head said. */
export function addChunk(assembly: Assembly, seq: number, data: string): Assembly | null {
  if (seq !== assembly.parts.length || seq >= assembly.chunks) return null;
  const bytes = assembly.bytes + decodedLength(data);
  if (bytes > assembly.size) return null;
  return { ...assembly, parts: [...assembly.parts, data], bytes };
}

function bytesOf(parts: readonly string[]): Uint8Array {
  const pieces = parts.map((part) => Uint8Array.from(atob(part), (char) => char.charCodeAt(0)));
  const whole = new Uint8Array(pieces.reduce((sum, piece) => sum + piece.length, 0));
  let at = 0;
  for (const piece of pieces) {
    whole.set(piece, at);
    at += piece.length;
  }
  return whole;
}

/** A vector image is text: `<svg` near its start, after an optional BOM, XML declaration or comment. */
function isSvg(bytes: Uint8Array): boolean {
  const head = String.fromCharCode(...bytes.slice(0, 512)).replace(/^\uFEFF|^\xEF\xBB\xBF/, "").trimStart();
  return (head.startsWith("<svg") || head.startsWith("<?xml") || head.startsWith("<!--")) && head.includes("<svg");
}

const IMAGE_SIGNATURES: readonly { mime: string; matches: (bytes: Uint8Array) => boolean }[] = [
  { mime: "image/png", matches: (b) => b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47 },
  { mime: "image/jpeg", matches: (b) => b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff },
  { mime: "image/gif", matches: (b) => b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x38 },
  { mime: "image/webp", matches: (b) => b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50 },
  { mime: "image/bmp", matches: (b) => b[0] === 0x42 && b[1] === 0x4d },
  { mime: "image/svg+xml", matches: isSvg },
  { mime: "image/avif", matches: (b) => b[4] === 0x66 && b[5] === 0x74 && b[6] === 0x79 && b[7] === 0x70 && b[8] === 0x61 && b[9] === 0x76 && b[10] === 0x69 && b[11] === 0x66 },
];

/**
 * The finished read as content, or null when the bytes are not what the head said: a short read, or an image whose first bytes
 * name no format the phone draws. The image is a `data:` address, which the phone's content policy already allows.
 */
export function finishAssembly(assembly: Assembly): LookContent | null {
  if (assembly.parts.length !== assembly.chunks || assembly.bytes !== assembly.size) return null;
  if (assembly.mode === "image" || assembly.mode === "page") {
    const mime = IMAGE_SIGNATURES.find((candidate) => candidate.matches(bytesOf(assembly.parts.slice(0, 1))))?.mime;
    if (!mime) return null;
    // 96 KiB chunks hold whole base64 groups, so the parts join into one valid text.
    const url = `data:${mime};base64,${assembly.parts.join("")}`;
    return assembly.page ? { state: "shot", url, size: assembly.size, ...assembly.page } : { state: "image", url, size: assembly.size };
  }
  const text = new TextDecoder("utf-8").decode(bytesOf(assembly.parts));
  if (assembly.mode === "html") return { state: "html", document: text, size: assembly.size };
  // The cap may cut a character in two; the half is not shown.
  return { state: "text", markdown: assembly.mode === "markdown", text: assembly.truncated ? text.replace(/�+$/, "") : text, truncated: assembly.truncated };
}

/** The folder and the file name of a changed file's path: `hide-platform/src/path.rs` is `path.rs` in `hide-platform/src`. */
export function splitPath(path: string): { name: string; folder: string } {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? { name: path, folder: "" } : { name: path.slice(slash + 1), folder: path.slice(0, slash) };
}

/** A changed file's extension in capitals, for its tile (`RS`, `TSX`); empty when it has none. */
export function fileTag(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1, dot + 5).toUpperCase() : "";
}

/** How a changed file's counts read (B14): counted, a binary hided found git cannot count, or a new file left uncounted (only its 새 파일 chip shows). */
export function countState(file: LookFile): "counted" | "binary" | "uncounted" {
  if (file.binary) return "binary";
  return file.added === null && file.removed === null ? "uncounted" : "counted";
}

/** The most an image zooms to. */
export const MAX_ZOOM = 8;

/** The scale after two fingers moved apart or together by `ratio`: never smaller than the fitted image. */
export function zoomBy(scale: number, ratio: number): number {
  return Math.min(MAX_ZOOM, Math.max(1, scale * ratio));
}

/** A zoomed image moves only as far as its edge reaches the frame's edge, along an axis `extent` long. */
export function clampOffset(offset: number, scale: number, extent: number): number {
  const room = (extent * (scale - 1)) / 2;
  return Math.max(-room, Math.min(room, offset));
}
