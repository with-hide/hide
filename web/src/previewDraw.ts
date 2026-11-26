// How a preview's picture is drawn from a file's bytes in this window: a
// PDF's first page through pdf.js, an image or a video's first frame through
// the browser's own decoders, each onto a canvas `TILE_PIXELS` wide and kept
// as a small JPEG. `filePreview.ts` decides what is read and when; this file
// only draws, so a test without a browser's decoders stands it in.

import type { Preview, PreviewKind } from "./filePreview";
import { loadPdfjs } from "./viewers/pdfjs";
import { videoMime } from "./viewers/video";

/** The tile is 150 CSS pixels wide; its picture is made at twice that. */
const TILE_PIXELS = 300;
/** A video that has not given its first frame by then counts as unreadable. */
const VIDEO_WAIT_MS = 10_000;

/** The picture of a file of `kind` whose whole bytes are `bytes`; `path` names its type. */
export async function drawPreview(kind: Exclude<PreviewKind, "text">, bytes: Uint8Array, path: string, signal: AbortSignal): Promise<Preview> {
  if (kind === "pdf") return await pdfPreview(bytes, signal);
  if (kind === "video") return await videoPreview(bytes, videoMime(path), signal);
  return await imagePreview(bytes, path.toLowerCase().endsWith(".svg") ? "image/svg+xml" : "image/*");
}

/** A canvas `TILE_PIXELS` wide holding `width` by `height`, kept to the tile's 3:2 at most in height. */
function tileCanvas(width: number, height: number): { canvas: HTMLCanvasElement; scale: number } {
  const scale = TILE_PIXELS / width;
  const canvas = document.createElement("canvas");
  canvas.width = TILE_PIXELS;
  canvas.height = Math.max(1, Math.round(Math.min(height * scale, TILE_PIXELS * 2)));
  return { canvas, scale };
}

function jpeg(canvas: HTMLCanvasElement): string {
  return canvas.toDataURL("image/jpeg", 0.82);
}

async function pdfPreview(bytes: Uint8Array, signal: AbortSignal): Promise<Preview> {
  const pdfjs = await loadPdfjs();
  if (signal.aborted) return { state: "failed" };
  // pdf.js detaches the buffer it is handed, so it gets a copy.
  const task = pdfjs.getDocument({ data: bytes.slice() });
  // A make stopped for its time ends its task now; what waits on it fails.
  const end = () => void task.destroy();
  signal.addEventListener("abort", end, { once: true });
  try {
    const pdf = await task.promise;
    const page = await pdf.getPage(1);
    const viewport = page.getViewport({ scale: 1 });
    const { canvas, scale } = tileCanvas(viewport.width, viewport.height);
    const context = canvas.getContext("2d");
    if (!context) return { state: "failed" };
    await page.render({ canvas, canvasContext: context, viewport: page.getViewport({ scale }) }).promise;
    return { state: "picture", picture: jpeg(canvas), pages: pdf.numPages, seconds: null };
  } finally {
    signal.removeEventListener("abort", end);
    void task.destroy();
  }
}

async function imagePreview(bytes: Uint8Array, type: string): Promise<Preview> {
  const url = URL.createObjectURL(new Blob([bytes.slice()], { type }));
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const { canvas } = tileCanvas(image.naturalWidth || TILE_PIXELS, image.naturalHeight || TILE_PIXELS);
    const context = canvas.getContext("2d");
    if (!context) return { state: "failed" };
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return { state: "picture", picture: jpeg(canvas), pages: null, seconds: null };
  } finally {
    URL.revokeObjectURL(url);
  }
}

async function videoPreview(bytes: Uint8Array, type: string, signal: AbortSignal): Promise<Preview> {
  const url = URL.createObjectURL(new Blob([bytes.slice()], { type }));
  const video = document.createElement("video");
  try {
    video.muted = true;
    video.preload = "auto";
    video.src = url;
    await new Promise<void>((resolve, reject) => {
      const timer = window.setTimeout(() => reject(new Error("video_timeout")), VIDEO_WAIT_MS);
      if (signal.aborted) reject(new Error("video_stopped"));
      signal.addEventListener("abort", () => {
        window.clearTimeout(timer);
        reject(new Error("video_stopped"));
      }, { once: true });
      video.onloadeddata = () => {
        // The very first frame is often black; a tenth of a second in is the picture.
        video.currentTime = Math.min(0.1, video.duration || 0);
      };
      video.onseeked = () => {
        window.clearTimeout(timer);
        resolve();
      };
      video.onerror = () => {
        window.clearTimeout(timer);
        reject(new Error("video_unreadable"));
      };
    });
    const { canvas } = tileCanvas(video.videoWidth || TILE_PIXELS, video.videoHeight || TILE_PIXELS);
    const context = canvas.getContext("2d");
    if (!context) return { state: "failed" };
    context.drawImage(video, 0, 0, canvas.width, canvas.height);
    return { state: "picture", picture: jpeg(canvas), pages: null, seconds: Number.isFinite(video.duration) ? Math.round(video.duration) : null };
  } finally {
    video.removeAttribute("src");
    video.load();
    URL.revokeObjectURL(url);
  }
}
