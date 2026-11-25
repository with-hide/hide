// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskOutput } from "../../desk";
import { TooltipProvider } from "../ui/tooltip";
import { OutputShelf } from "./output-shelf";

const output = (id: string, kind: DeskOutput["kind"], seen: boolean): DeskOutput => ({ id, kind, label: id, thumb: null, seen, pr_state: kind === "pr" ? "pending" : null, device_id: "mac", checkout: "/repo", path: kind === "page" || kind === "pr" ? null : `/repo/${id}`, url: kind === "page" ? "http://127.0.0.1:4399" : null });

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(outputs: DeskOutput[], opened: string[]) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider><OutputShelf outputs={outputs} onOpen={(one) => opened.push(one.id)} /></TooltipProvider>));
  return host;
}

it("groups a session's outputs by kind, the groups with one not seen first and ending with the look eye (Doop B4, shelf 나)", async () => {
  const host = await render([output("store-shot-1.png", "image", false), output("store-shot-2.png", "image", true), output("page", "page", true), output("android-release.md", "doc", false), output("2537", "pr", true), output("2536", "pr", true)], []);
  const groups = Array.from(host.querySelectorAll<HTMLElement>("[data-desk-shelf-group]"));
  // A file or a View alone of its kind shows itself; a kind with more shows its count.
  expect(groups.map((group) => [group.dataset.deskShelfGroup, group.textContent])).toEqual([["image", "Images2"], ["doc", "android-release.md"], ["page", "page"], ["pr", "PRs2"]]);
  expect(groups.map((group) => group.querySelector('[data-mark="look"]') !== null)).toEqual([true, true, false, false]);
  // The image group shows two of its pictures.
  expect(groups[0]!.querySelector("[data-desk-shelf-thumbs]")?.getAttribute("data-desk-shelf-thumbs")).toBe("2");
});

it("opens a group of one in the tab panel and a larger group to its list, whose row opens there", async () => {
  const opened: string[] = [];
  const host = await render([output("a.png", "image", true), output("b.png", "image", true), output("notes.md", "doc", true)], opened);
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="doc"]')!.click());
  expect(opened).toEqual(["notes.md"]);
  expect(host.querySelector("[data-desk-shelf-list]")).toBeNull();
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="image"]')!.click());
  expect(host.querySelector("[data-desk-shelf-list]")?.getAttribute("data-desk-shelf-list")).toBe("image");
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-list] [data-desk-output="b.png"]')!.click());
  expect(opened).toEqual(["notes.md", "b.png"]);
});

it("draws nothing for a session with no output", async () => {
  const host = await render([], []);
  expect(host.querySelector("[data-desk-shelf]")).toBeNull();
});
