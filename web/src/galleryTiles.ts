import type { RecordView, WorkSession } from "./snapshot";
import { workKey } from "./sessionWork";

/**
 * The session record's 결과물 (PRD session-docs-and-assistant D-40): the
 * Views the session and its descendants opened, then the outputs its turns
 * named. Only an item with a real picture is a thumbnail; every other one is
 * a compact row.
 */
export type GalleryTile = { kind: "view"; view: RecordView } | { kind: "artifact"; name: string };

/** A thumbnail's picture: an image itself, or a page's capture this window already holds. */
export type Picture = { kind: "image" } | { kind: "still"; src: string };

/** Thumbnails shown before `+N`: one row of the three columns. */
export const FIRST_PICTURES = 3;
/** Rows shown before `+N`. */
export const FIRST_ROWS = 4;

const IMAGE_FILE = /\.(png|jpe?g|gif|webp|bmp|avif|svg)$/i;

/** Whether a View is an image file, by the document its tab holds or, before one is, by its name. */
export function isImageView(view: RecordView): boolean {
  return view.kind === "file" && (view.document_kind === "image" || (view.document_kind === undefined && IMAGE_FILE.test(view.path)));
}

/**
 * The tiles of the record whose tree `sessions` holds, a drilled child's
 * record keeping only its own subtree's Views. The core sends the Views in
 * the order they were last looked at; the outputs follow, as the record
 * lists them.
 */
export function galleryTiles(views: readonly RecordView[], sessions: readonly WorkSession[], artifacts: readonly string[]): GalleryTile[] {
  const keys = new Set(sessions.map(workKey));
  return [
    ...views.filter((view) => keys.has(workKey(view.session))).map((view): GalleryTile => ({ kind: "view", view })),
    ...artifacts.map((name): GalleryTile => ({ kind: "artifact", name })),
  ];
}

/** A tile's picture, or null for a row; `stillOf` answers a page's capture this window already took. */
export function pictureOf(tile: GalleryTile, stillOf: (view: RecordView) => string | null): Picture | null {
  if (tile.kind !== "view") return null;
  if (isImageView(tile.view)) return { kind: "image" };
  const still = tile.view.kind === "browser" ? stillOf(tile.view) : null;
  return still ? { kind: "still", src: still } : null;
}

/**
 * A row's one short detail: an open document's first heading (else its first
 * line), a page's address, and for any other file where it sits in its
 * checkout; a named output has none.
 */
export function rowDetail(tile: GalleryTile): string | null {
  if (tile.kind !== "view") return null;
  const { view } = tile;
  if (view.kind === "browser") return view.url ? view.url.replace(/^https?:\/\//i, "") : null;
  const lines = view.lines ?? [];
  const heading = lines.find((line) => /^#{1,6}\s/.test(line)) ?? lines.find((line) => line.trim());
  if (heading) return heading.replace(/^#{1,6}\s+/, "").trim();
  if (!view.path) return null;
  const inside = view.path.startsWith(`${view.checkout_path}/`) ? view.path.slice(view.checkout_path.length + 1) : view.path;
  const folder = inside.split("/").slice(0, -1).join("/");
  return folder || null;
}
