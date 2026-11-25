// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskOutput } from "../../desk";
import type { SessionPull } from "../../snapshot";
import { TooltipProvider } from "../ui/tooltip";
import { OutputShelf } from "./output-shelf";

const output = (id: string, kind: Exclude<DeskOutput["kind"], "pr">, seen: boolean): DeskOutput => ({ id, kind, label: id, thumb: null, seen, pr_state: null, device_id: "mac", checkout: "/repo", path: kind === "page" ? null : `/repo/${id}`, url: kind === "page" ? "http://127.0.0.1:4399" : null });
const pull = (number: number): SessionPull => ({ url: `https://github.com/o/r/pull/${number}`, number, title: `change ${number}`, state: "mergeable", own: true, holders: [], stale: false, last_read_at_unix_ms: null });

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(outputs: DeskOutput[], pulls: SessionPull[], opened: string[]) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider><OutputShelf outputs={outputs} pulls={pulls} onOpen={(one) => opened.push(one.id)} onPull={(one, external) => opened.push(`${external ? "github " : ""}#${one.number}`)} /></TooltipProvider>));
  return host;
}

it("groups what a session made by kind, the groups with one not seen first and ending with the look eye (Doop B4, shelf 나)", async () => {
  const host = await render([output("store-shot-1.png", "image", false), output("store-shot-2.png", "image", true), output("page", "page", true), output("a.md", "doc", true), output("b.md", "doc", false)], [pull(2537), pull(2536)], []);
  const groups = Array.from(host.querySelectorAll<HTMLElement>("[data-desk-shelf-group]"));
  expect(groups.map((group) => [group.dataset.deskShelfGroup, group.textContent])).toEqual([["image", "Images2"], ["doc", "Files2"], ["page", "page"], ["pr", "PRs2"]]);
  // A pull request is not a View, so its group never carries the eye.
  expect(groups.map((group) => group.querySelector('[data-mark="look"]') !== null)).toEqual([true, true, false, false]);
  expect(groups[0]!.querySelector("[data-desk-shelf-thumbs]")?.getAttribute("data-desk-shelf-thumbs")).toBe("2");
});

it("shows a thing alone of its kind as itself and opens it with one press; a larger group opens its list, whose row opens (Doop B5)", async () => {
  const opened: string[] = [];
  const host = await render([output("a.png", "image", true), output("b.png", "image", true), output("notes.md", "doc", true)], [pull(2532)], opened);
  expect(host.querySelector('[data-desk-shelf-group="doc"]')?.textContent).toBe("notes.md");
  expect(host.querySelector('[data-desk-shelf-group="pr"]')?.textContent).toContain(`#${2532}`);
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="doc"]')!.click());
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="pr"]')!.click());
  // ⌘-press opens a pull request on its host, as every PR mark does.
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="pr"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true })));
  expect(opened).toEqual(["notes.md", `#${2532}`, `github #${2532}`]);
  expect(host.querySelector("[data-desk-shelf-list]")).toBeNull();
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="image"]')!.click());
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-list="image"] [data-desk-shelf-item="b.png"]')!.click());
  expect(opened).toEqual(["notes.md", `#${2532}`, `github #${2532}`, "b.png"]);
});

it("lists six pull requests and the rest behind + n more", async () => {
  const host = await render([], [2537, 2536, 2535, 2532, 2526, 2521, 2520, 917, 914].map(pull), []);
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="pr"]')!.click());
  expect(host.querySelectorAll("[data-desk-shelf-list] [data-desk-shelf-item]")).toHaveLength(6);
  expect(host.querySelector("[data-desk-shelf-more]")?.textContent).toBe(`+ 3 more · ${[2520, 917, 914].map((number) => `#${number}`).join(" ")}`);
  await act(async () => host.querySelector<HTMLElement>("[data-desk-shelf-more]")!.click());
  expect(host.querySelectorAll("[data-desk-shelf-list] [data-desk-shelf-item]")).toHaveLength(9);
});

it("draws a listed pull request as every PR list does: stale, closed and who holds it", async () => {
  const pulls: SessionPull[] = [{ ...pull(2537), stale: true }, { ...pull(2536), state: "closed" }, { ...pull(2535), own: false, holders: ["fixer"] }];
  const host = await render([], pulls, []);
  await act(async () => host.querySelector<HTMLElement>('[data-desk-shelf-group="pr"]')!.click());
  const rows = Array.from(host.querySelectorAll("[data-desk-shelf-list] [data-desk-shelf-item]"));
  expect(rows[0]?.querySelector("[data-stale=\"true\"]")).not.toBeNull();
  expect(rows[1]?.querySelector("[data-pr-closed]")).not.toBeNull();
  expect(rows[2]?.textContent).toContain("fixer");
});

it("draws nothing for a session that made nothing", async () => {
  const host = await render([], [], []);
  expect(host.querySelector("[data-desk-shelf]")).toBeNull();
});
