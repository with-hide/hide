// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "../ui/tooltip";
import { PreviewTileView, type TileFile } from "./preview-tile";

// An output tile under the pointer shows its full path in the app's hint, as
// the shelf's chip shows its place (docs/UI_BEHAVIOR.md, Desk's outputs), and
// no larger preview; the button keeps the tile's name as its own.
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function hover(file: TileFile): Promise<{ hint: string | null | undefined; name: string | null; title: string | null }> {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider delayDuration={0}><PreviewTileView file={file} preview={{ state: "none" }} onRetry={() => {}} onOpen={() => {}} /></TooltipProvider>));
  try {
    const button = container.querySelector<HTMLElement>("[data-desk-tile-open]")!;
    await act(async () => {
      button.dispatchEvent(new MouseEvent("pointermove", { bubbles: true }));
      await new Promise((done) => setTimeout(done, 0));
    });
    return { hint: document.querySelector("[role=tooltip]")?.textContent, name: button.getAttribute("aria-label"), title: button.getAttribute("title") };
  } finally {
    await act(async () => root.unmount());
  }
}

it("shows an HTML file's full path, and a page's address, in the app's hint", async () => {
  const html = { id: "v1", label: "index.html", path: "/repo/agents/runs/copy-pos/index.html", url: null, device: "local", root: "/repo" };
  expect(await hover(html)).toEqual({ hint: "/repo/agents/runs/copy-pos/index.html", name: "index.html", title: null });
  const page = { id: "v2", label: "PR 2529", path: null, url: "https://github.com/with-hide/hide/pull/2529/files", device: "local", root: "/repo" };
  expect((await hover(page)).hint).toBe("https://github.com/with-hide/hide/pull/2529/files");
});
