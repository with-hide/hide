import type { RecordView, WorkSession } from "./snapshot";
import { workKey } from "./sessionWork";

/**
 * The session record's 결과물 gallery (PRD session-docs-and-assistant D-40):
 * the Views the session and its descendants opened, then the outputs its
 * turns named, each a tile whose picture is only what Hide already holds.
 */
export type GalleryTile =
  | { kind: "view"; view: RecordView; thumb: ViewThumb }
  | { kind: "artifact"; name: string };

/**
 * What a View's tile shows: an image itself, an open document's first
 * lines, a page's capture or its title and address, and anything else its
 * kind's mark and name.
 */
export type ViewThumb = "image" | "lines" | "page" | "mark";

/** Tiles shown before `+N`: three rows of the two columns. */
export const FIRST_TILES = 6;

const IMAGE_FILE = /\.(png|jpe?g|gif|webp|bmp|avif|svg)$/i;

export function viewThumb(view: RecordView): ViewThumb {
  if (view.kind === "browser") return "page";
  if (view.kind !== "file") return "mark";
  if (view.document_kind === "image" || (view.document_kind === undefined && IMAGE_FILE.test(view.path))) return "image";
  return view.lines?.length ? "lines" : "mark";
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
    ...views.filter((view) => keys.has(workKey(view.session))).map((view): GalleryTile => ({ kind: "view", view, thumb: viewThumb(view) })),
    ...artifacts.map((name): GalleryTile => ({ kind: "artifact", name })),
  ];
}
