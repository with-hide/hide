// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PreviewTileView, type TileFile } from "./preview-tile";

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function face(file: TileFile): Promise<HTMLElement> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<PreviewTileView file={file} preview={{ state: "none" }} onRetry={() => undefined} />));
  return host.querySelector<HTMLElement>("[data-desk-tile-reason]")!;
}

it("marks a page tile with no preview as a page View is, never by its label, which is the page's title", async () => {
  const web = await face({ id: "v1", label: "Pull Request 2529 · with-hide/hide", path: null, url: "https://github.com/with-hide/hide/pull/2529/files", device: "mac", root: "/repo" });
  const local = await face({ id: "v2", label: "문구 위치 시안", path: null, url: "file:///repo/agents/runs/copy-pos/index.html", device: "mac", root: "/repo" });
  const doc = await face({ id: "v3", label: "notes.md", path: "/repo/notes.md", url: null, device: "mac", root: "/repo" });
  expect([web, local, doc].map((tile) => tile.querySelector<HTMLElement>("[data-view-mark]")?.dataset.viewMark)).toEqual(["browser", "browser", "file"]);
});
