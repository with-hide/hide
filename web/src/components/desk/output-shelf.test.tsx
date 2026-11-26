// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskOutput } from "../../desk";
import type { SessionPull } from "../../snapshot";
import { TooltipProvider } from "../ui/tooltip";
import { OutputShelf, viewWhere } from "./output-shelf";

// Expected answers are PRD desk-own-desk D-08, D-17, D-18, B21-B27 (shelf 안 1).

const output = (id: string, kind: Exclude<DeskOutput["kind"], "pr">, seen: boolean, path = `/repo/docs/${id}`): DeskOutput => ({ id, kind, label: id, thumb: null, seen, pr_state: null, device_id: "mac", checkout: "/repo", path: kind === "page" ? null : path, url: kind === "page" ? "http://127.0.0.1:4399/report" : null });
const pull = (number: number): SessionPull => ({ url: `https://github.com/o/r/pull/${number}`, number, title: `change ${number}`, state: "mergeable", own: true, holders: [], stale: false, last_read_at_unix_ms: null });

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(outputs: DeskOutput[], pulls: SessionPull[], opened: string[], more = false) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider><OutputShelf outputs={outputs} more={more} pulls={pulls} onOpen={(one) => opened.push(one.id)} onPull={(one, external) => opened.push(`${external ? "github " : ""}#${one.number}`)} /></TooltipProvider>));
  return host;
}

/** Opens a chip's floating list as a key press does (Radix opens a menu on pointer down or Enter). */
async function openList(selector: string) {
  await act(async () => document.querySelector<HTMLElement>(selector)!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
}

const rows = () => Array.from(document.querySelectorAll<HTMLElement>("[data-desk-view-row]")).map((row) => row.dataset.deskViewRow);

it("is one 뷰 N chip with the 볼 것 eye while any View is not looked at, and the PR mark beside it (B21)", async () => {
  const host = await render([output("a.png", "image", false), output("b.md", "doc", true), output("page", "page", true)], [pull(41)], []);
  const chip = host.querySelector<HTMLElement>("[data-desk-views]")!;
  expect([chip.dataset.deskViews, chip.textContent, chip.dataset.deskViewsUnseen]).toEqual(["3", "Views 3", "true"]);
  expect(chip.querySelector('[data-mark="look"]')).not.toBeNull();
  expect(host.querySelectorAll("[data-desk-shelf-group]")).toHaveLength(0);
  expect(host.querySelector('[data-desk-pulls="1"]')?.textContent).toContain("#41");
});

it("floats its list over the conversation, those to look at first and those seen after, each in the order the core lists them (B22, B23)", async () => {
  const host = await render([output("new.md", "doc", false), output("shot.png", "image", true), output("page", "page", false), output("old.md", "doc", true)], [], []);
  // The shelf is one line; the list is drawn outside it, in a layer.
  expect(host.querySelector("[data-desk-views-list]")).toBeNull();
  await openList("[data-desk-views]");
  const list = document.querySelector("[data-desk-views-list]")!;
  expect(host.contains(list)).toBe(false);
  expect(Array.from(list.querySelectorAll("[data-desk-views-group]")).map((group) => group.textContent)).toEqual(["To look at 2", "Seen 2"]);
  expect(rows()).toEqual(["new.md", "page", "shot.png", "old.md"]);
  expect(Array.from(document.querySelectorAll<HTMLElement>("[data-desk-view-unseen]")).map((row) => row.dataset.deskViewRow)).toEqual(["new.md", "page"]);
});

it("names where each View is: a file's folder under its checkout, a page's address (B23)", () => {
  expect(viewWhere(output("a.md", "doc", true, "/repo/docs/lessons/a.md"))).toBe("docs/lessons");
  expect(viewWhere(output("a.md", "doc", true, "/repo/a.md"))).toBeNull();
  expect(viewWhere(output("page", "page", true))).toBe("127.0.0.1:4399/report");
});

it("opens a row's View and closes the list (B25)", async () => {
  const opened: string[] = [];
  await render([output("a.md", "doc", false), output("b.md", "doc", true)], [], opened);
  await openList("[data-desk-views]");
  await act(async () => document.querySelector<HTMLElement>('[data-desk-view-row="b.md"]')!.click());
  expect(opened).toEqual(["b.md"]);
  expect(document.querySelector("[data-desk-views-list]")).toBeNull();
});

it("ends the list with a dim line when older Views were left out (B27)", async () => {
  await render([output("a.md", "doc", true)], [], [], true);
  await openList("[data-desk-views]");
  expect(document.querySelector("[data-desk-views-older]")?.textContent).toBe("Older views are not listed");
});

it("opens one pull request with a press, on its host with ⌘, and several from a PR n list (B26)", async () => {
  const opened: string[] = [];
  const one = await render([], [pull(2532)], opened);
  await act(async () => one.querySelector<HTMLElement>('[data-desk-pulls="1"]')!.click());
  await act(async () => one.querySelector<HTMLElement>('[data-desk-pulls="1"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true, metaKey: true })));
  expect(opened).toEqual(["#2532", "github #2532"]);
  const many = await render([], [pull(2537), pull(2536)], opened);
  expect(many.querySelector('[data-desk-pulls="2"]')?.textContent).toBe("PR 2");
  await openList('[data-desk-pulls="2"]');
  await act(async () => document.querySelector<HTMLElement>('[data-desk-pull-row="pr:https://github.com/o/r/pull/2536"]')!.click());
  expect(opened.at(-1)).toBe("#2536");
});

it("has no 뷰 chip for a session that opened no View, and draws nothing for one that made nothing (B21)", async () => {
  const pulls = await render([], [pull(1)], []);
  expect(pulls.querySelector("[data-desk-views]")).toBeNull();
  const none = await render([], [], []);
  expect(none.querySelector("[data-desk-shelf]")).toBeNull();
});
