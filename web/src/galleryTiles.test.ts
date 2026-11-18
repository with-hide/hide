import { expect, it } from "vitest";
import { galleryTiles, pictureOf, rowDetail, type GalleryTile } from "./galleryTiles";
import type { RecordView, WorkOutput, WorkSession } from "./snapshot";

const ROOT: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const OTHER: WorkSession = { device_id: "local", agent: "claude", session_id: "other" };
const view = (id: string, patch: Partial<RecordView> = {}): RecordView => ({ device_id: "local", checkout_path: "/c", display_id: id, kind: "file", label: id, path: `/c/${id}`, session: ROOT, ...patch });
const output = (kind: "pr" | "issue", number: number): WorkOutput => ({ kind, repository: "acme/app", number, title: `${kind} ${number}`, url: `https://example.invalid/${kind}/${number}`, state: "open", stale: false, last_read_at_unix_ms: null, created_at_unix_ms: number, source: ROOT, source_name: "root" });
const ISSUE = output("issue", 41);

it("gives a thumbnail only to an image and a page this window holds a capture of; everything else is a row (D-40)", () => {
  const still = (view: RecordView) => (view.display_id === "captured" ? "blob:still" : null);
  const picture = (one: RecordView) => pictureOf({ kind: "view", view: one }, still)?.kind ?? "row";
  expect(picture(view("a.png", { document_kind: "image" }))).toBe("image");
  // A file whose document is not held yet is still an image by its name.
  expect(picture(view("b.webp"))).toBe("image");
  expect(picture(view("captured", { kind: "browser", path: "", url: "https://example.invalid" }))).toBe("still");
  expect(picture(view("uncaptured", { kind: "browser", path: "", url: "https://example.invalid" }))).toBe("row");
  for (const other of [view("c.md", { document_kind: "markdown", lines: ["# c"] }), view("e.pdf", { document_kind: "pdf" }), view("g", { kind: "external", path: "" })]) expect(picture(other)).toBe("row");
  expect(pictureOf({ kind: "issue", issue: ISSUE }, still)).toBeNull();
});

it("details a row by a document's first heading, a page's address, where a file sits or an issue's number (D-40)", () => {
  const detail = (one: RecordView) => rowDetail({ kind: "view", view: one });
  expect(detail(view("a.md", { lines: ["", "intro", "## Goal"] }))).toBe("Goal");
  expect(detail(view("b.md", { lines: ["", "plain first line"] }))).toBe("plain first line");
  expect(detail(view("p", { kind: "browser", path: "", url: "https://example.invalid/a" }))).toBe("example.invalid/a");
  expect(detail(view("x", { path: "/c/docs/menu/spec.pdf" }))).toBe("docs/menu");
  expect(detail(view("y", { path: "/c/top.pdf" }))).toBeNull();
  expect(rowDetail({ kind: "issue", issue: ISSUE })).toBe("#41");
});

it("keeps the shown record's subtree's Views in the core's order, then the issues it made, never its PRs (D-40)", () => {
  const views = [view("child", { session: CHILD }), view("root"), view("stranger", { session: OTHER })];
  const name = (tile: GalleryTile) => (tile.kind === "view" ? tile.view.display_id : `#${tile.issue.number}`);
  expect(galleryTiles(views, [ROOT, CHILD], [ISSUE, output("pr", 31), output("issue", 40)]).map(name)).toEqual(["child", "root", "#41", "#40"]);
  // A drilled child's record shows only its own.
  expect(galleryTiles(views, [CHILD], []).map(name)).toEqual(["child"]);
});
