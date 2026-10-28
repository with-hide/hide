import { expect, it } from "vitest";
import { galleryTiles, pictureOf, rowDetail } from "./galleryTiles";
import type { RecordView, WorkSession } from "./snapshot";

const ROOT: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const OTHER: WorkSession = { device_id: "local", agent: "claude", session_id: "other" };
const view = (id: string, patch: Partial<RecordView> = {}): RecordView => ({ device_id: "local", checkout_path: "/c", display_id: id, kind: "file", label: id, path: `/c/${id}`, session: ROOT, ...patch });

it("gives a thumbnail only to an image and a page this window holds a capture of; everything else is a row (D-40)", () => {
  const still = (view: RecordView) => (view.display_id === "captured" ? "blob:still" : null);
  const picture = (one: RecordView) => pictureOf({ kind: "view", view: one }, still)?.kind ?? "row";
  expect(picture(view("a.png", { document_kind: "image" }))).toBe("image");
  // A file whose document is not held yet is still an image by its name.
  expect(picture(view("b.webp"))).toBe("image");
  expect(picture(view("captured", { kind: "browser", path: "", url: "https://example.invalid" }))).toBe("still");
  expect(picture(view("uncaptured", { kind: "browser", path: "", url: "https://example.invalid" }))).toBe("row");
  for (const other of [view("c.md", { document_kind: "markdown", lines: ["# c"] }), view("e.pdf", { document_kind: "pdf" }), view("g", { kind: "external", path: "" })]) expect(picture(other)).toBe("row");
  expect(pictureOf({ kind: "artifact", name: "Release notes" }, still)).toBeNull();
});

it("details a row by a document's first heading, a page's address or where a file sits (D-40)", () => {
  const detail = (one: RecordView) => rowDetail({ kind: "view", view: one });
  expect(detail(view("a.md", { lines: ["", "intro", "## Goal"] }))).toBe("Goal");
  expect(detail(view("b.md", { lines: ["", "plain first line"] }))).toBe("plain first line");
  expect(detail(view("p", { kind: "browser", path: "", url: "https://example.invalid/a" }))).toBe("example.invalid/a");
  expect(detail(view("x", { path: "/c/docs/menu/spec.pdf" }))).toBe("docs/menu");
  expect(detail(view("y", { path: "/c/top.pdf" }))).toBeNull();
  expect(rowDetail({ kind: "artifact", name: "Release notes" })).toBeNull();
});

it("keeps the shown record's subtree's Views in the core's order, then its named outputs (D-40)", () => {
  const views = [view("child", { session: CHILD }), view("root"), view("stranger", { session: OTHER })];
  const tiles = galleryTiles(views, [ROOT, CHILD], ["Release notes"]);
  expect(tiles.map((tile) => (tile.kind === "view" ? tile.view.display_id : tile.name))).toEqual(["child", "root", "Release notes"]);
  // A drilled child's record shows only its own.
  expect(galleryTiles(views, [CHILD], []).map((tile) => (tile.kind === "view" ? tile.view.display_id : tile.name))).toEqual(["child"]);
});
