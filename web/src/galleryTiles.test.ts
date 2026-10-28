import { expect, it } from "vitest";
import { galleryTiles, viewThumb } from "./galleryTiles";
import type { RecordView, WorkSession } from "./snapshot";

const ROOT: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const OTHER: WorkSession = { device_id: "local", agent: "claude", session_id: "other" };
const view = (id: string, patch: Partial<RecordView> = {}): RecordView => ({ device_id: "local", checkout_path: "/c", display_id: id, kind: "file", label: id, path: `/c/${id}`, session: ROOT, ...patch });

it("pictures an image itself, an open document by its lines, a page as a page and anything else by its mark (D-40)", () => {
  expect(viewThumb(view("a.png", { document_kind: "image" }))).toBe("image");
  // A file whose document is not held yet is still an image by its name.
  expect(viewThumb(view("b.webp"))).toBe("image");
  expect(viewThumb(view("c.md", { document_kind: "markdown", lines: ["# c"] }))).toBe("lines");
  // An open document with no lines, a PDF, a diff and an outside file have nothing to show but their kind.
  expect(viewThumb(view("d.md", { document_kind: "markdown", lines: [] }))).toBe("mark");
  expect(viewThumb(view("e.pdf", { document_kind: "pdf" }))).toBe("mark");
  expect(viewThumb(view("f", { kind: "diff" }))).toBe("mark");
  expect(viewThumb(view("g", { kind: "external", path: "" }))).toBe("mark");
  expect(viewThumb(view("h", { kind: "browser", path: "", url: "https://example.invalid" }))).toBe("page");
});

it("keeps the shown record's subtree's Views in the core's order, then its named outputs (D-40)", () => {
  const views = [view("child", { session: CHILD }), view("root"), view("stranger", { session: OTHER })];
  const tiles = galleryTiles(views, [ROOT, CHILD], ["Release notes"]);
  expect(tiles.map((tile) => (tile.kind === "view" ? tile.view.display_id : tile.name))).toEqual(["child", "root", "Release notes"]);
  // A drilled child's record shows only its own.
  expect(galleryTiles(views, [CHILD], []).map((tile) => (tile.kind === "view" ? tile.view.display_id : tile.name))).toEqual(["child"]);
});
