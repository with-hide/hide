// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { fileIcon } from "../../fileIcons";
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

it("marks a tile with no preview by its kind, never by a page's title: a web page's globe, a local page's file mark", async () => {
  // A page's label is its title once its View is known, which names no file type.
  const web = await face({ id: "v1", label: "Pull Request 2529 · with-hide/hide", path: null, url: "https://github.com/with-hide/hide/pull/2529/files", device: "mac", root: "/repo" });
  expect(web.querySelector("[data-desk-tile-mark='page']")).not.toBeNull();
  const local = await face({ id: "v2", label: "문구 위치 시안", path: null, url: "file:///repo/agents/runs/copy-pos/index.html", device: "mac", root: "/repo" });
  expect(local.querySelector("[data-desk-tile-mark='page']")).toBeNull();
  expect(local.querySelector<HTMLElement>("[data-file-mark]")!.style.getPropertyValue("--file-mark")).toBe(`url("${fileIcon("index.html").url}")`);
  expect(fileIcon("index.html").url).not.toBe(fileIcon("문구 위치 시안").url);
});
