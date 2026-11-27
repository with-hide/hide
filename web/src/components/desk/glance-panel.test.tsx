// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskJob } from "../../desk";
import type { DeskGlance, DeskGlanceFile, GlanceFolds } from "../../snapshot";
import { TooltipProvider } from "../ui/tooltip";
import { GLANCE_FILE_ROWS, GlanceCrumb, GlancePanel } from "./glance-panel";

// Expected answers are the approved 한눈에 picture (plan B of the Desk 다듬기 2
// mockup, section 4) and the operator's decisions of 2026-11-27: `세션` and
// `파일 n` tabs, a search that keeps the open tab's rows by name, three
// session groups whose heads fold them with 끝남 folded at first, a second
// line with the session's state and the time since it moved, files 볼 것
// first with their session named at the end, and `▯|` folding the panel.

const card = (pane: string, title: string, state: DeskJob["state"], extra: Partial<DeskJob> = {}): DeskJob => ({
  id: `pane:${pane}`, agent_id: null, pane_id: pane, title, agent_kind: "claude", project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "main" },
  state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: state === "asking" ? "question" : null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
  ...extra,
});
const file = (id: string, pane: string | null, seen: boolean): DeskGlanceFile => ({ id, kind: "doc", label: `${id}.md`, thumb: null, seen, pr_state: null, device_id: "mac", checkout: "/repo", path: `/repo/${id}.md`, url: null, pane_id: pane });
const noop = () => undefined;
const OPEN: GlanceFolds = { turn: false, working: false, ended: false };

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

async function render(node: React.ReactNode) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
}

const all = (selector: string) => [...document.body.querySelectorAll<HTMLElement>(selector)];

/** Types `text` into `input` as React reads a change. */
function type(input: HTMLInputElement, text: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Presses a tab as a pointer does: Radix picks a tab on the press, not the click. */
function press(element: Element) {
  element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
}

it("groups the sessions under 내 차례, 일하는 중 and 끝남 with their counts, 끝남 folded at first, and a head folds or unfolds its group", async () => {
  const cards = [card("p1", "Play 스토어 등록", "asking"), card("p2", "하네스 강의", "working"), card("p3", "페이지 원인", "done"), card("p4", "릴리스 노트", "sleeping")];
  const glance: DeskGlance = { sessions: ["p4", "p3", "p2", "p1"], files: [], files_more: 0 };
  const groups: [string, boolean][] = [];
  await render(<GlancePanel glance={glance} cards={cards} onTab={noop} onGroup={(group, folded) => groups.push([group, folded])} onSession={noop} onFile={noop} onFold={noop} />);

  expect(all("[data-desk-glance-group-head]").map((head) => [head.dataset.deskGlanceGroupHead, head.textContent, head.getAttribute("aria-expanded")])).toEqual([
    ["turn", "Needs You · 1", "true"],
    ["working", "Working · 1", "true"],
    ["ended", "Ended · 2", "false"],
  ]);
  expect(all("[data-desk-glance-session]").map((row) => row.dataset.deskGlanceSession)).toEqual(["p1", "p2"]);
  await act(async () => document.querySelector<HTMLElement>('[data-desk-glance-group-head="ended"]')!.click());
  await act(async () => document.querySelector<HTMLElement>('[data-desk-glance-group-head="turn"]')!.click());
  expect(groups).toEqual([["ended", false], ["turn", true]]);
});

it("draws the folds the core keeps, and each row's state in one line with the time since it moved", async () => {
  const at = Date.now() - 3 * 60_000;
  const cards = [
    card("p1", "Play 스토어 등록", "asking", { question: "스크린샷을 다시 올릴까요?", changed_at_unix_ms: at }),
    card("p2", "하네스 강의", "working", { now: "챕터 2 고치는 중", changed_at_unix_ms: at }),
    card("p3", "페이지 원인", "done", { summary: "원인을 찾았어요" }),
  ];
  await render(<GlancePanel glance={{ sessions: ["p3", "p2", "p1"], files: [], files_more: 0 }} cards={cards} folds={{ turn: true, working: false, ended: false }} onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={noop} />);

  const rows = all("[data-desk-glance-session]");
  expect(rows.map((row) => row.dataset.deskGlanceSession)).toEqual(["p2", "p3"]);
  expect(rows.map((row) => row.querySelector("[data-desk-job-line]")?.textContent)).toEqual(["챕터 2 고치는 중", "Done · 원인을 찾았어요"]);
  expect(rows[0]!.querySelector("[data-desk-glance-since]")?.textContent).toBe("3m");
  // A card whose time Hide never measured shows none.
  expect(rows[1]!.querySelector("[data-desk-glance-since]")).toBeNull();
});

it("keeps only the open tab's rows whose name holds the search, and says so when none does", async () => {
  const cards = [card("p1", "Play 스토어 등록", "asking"), card("p2", "하네스 강의", "working"), card("p3", "하네스 리뷰", "done")];
  await render(<GlancePanel glance={{ sessions: ["p3", "p2", "p1"], files: [], files_more: 0 }} cards={cards} folds={OPEN} onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={noop} />);
  const search = document.querySelector<HTMLInputElement>('[data-desk-glance-search="sessions"]')!;
  expect(search.placeholder).toBe("Find a session");

  await act(async () => type(search, " 하네스 "));
  expect(all("[data-desk-glance-session]").map((row) => row.dataset.deskGlanceSession)).toEqual(["p2", "p3"]);
  expect(all("[data-desk-glance-group-head]").map((head) => head.textContent)).toEqual(["Working · 1", "Ended · 1"]);
  await act(async () => type(search, "없는 이름"));
  expect(all("[data-desk-glance-session]")).toHaveLength(0);
  expect(document.querySelector('[data-desk-glance-none="sessions"]')).not.toBeNull();
});

it("lists the files 볼 것 first on the 파일 tab, each naming its session or Desk, and a press on a tab asks for it", async () => {
  const cards = [card("p1", "Play 스토어 등록", "asking"), card("p2", "하네스 강의", "working")];
  const glance: DeskGlance = { sessions: ["p2", "p1"], files: [file("hero", "p1", false), file("plan", null, false), file("roadmap", "p2", true)], files_more: 0 };
  const opened: string[] = [];
  const tabs: string[] = [];
  await render(<GlancePanel glance={glance} cards={cards} tab="files" onTab={(tab) => tabs.push(tab)} onGroup={noop} onSession={noop} onFile={(one) => opened.push(one.id)} onFold={noop} />);

  expect(document.querySelector('[data-desk-glance-tab-trigger="files"]')?.textContent).toBe("Files 3");
  expect(all("[data-desk-glance-session]")).toHaveLength(0);
  const files = all("[data-desk-glance-file]");
  expect(files.map((row) => row.dataset.deskGlanceFile)).toEqual(["hero", "plan", "roadmap"]);
  expect(files.map((row) => row.querySelector("[data-desk-glance-by]")?.textContent)).toEqual(["Play 스토어 등록", "Desk", "하네스 강의"]);
  expect(files.map((row) => row.dataset.deskGlanceUnseen ?? "seen")).toEqual(["true", "true", "seen"]);
  expect(document.querySelector("[data-desk-glance-files]")?.textContent).toContain("To look at 2");

  const search = document.querySelector<HTMLInputElement>('[data-desk-glance-search="files"]')!;
  await act(async () => type(search, "ROAD"));
  expect(all("[data-desk-glance-file]").map((row) => row.dataset.deskGlanceFile)).toEqual(["roadmap"]);

  await act(async () => all("[data-desk-glance-file]")[0]!.click());
  await act(async () => press(document.querySelector('[data-desk-glance-tab-trigger="sessions"]')!));
  expect([opened, tabs]).toEqual([["roadmap"], ["sessions"]]);
});

it("folds the whole panel with ▯|", async () => {
  const fold = vi.fn();
  await render(<GlancePanel glance={{ sessions: [], files: [], files_more: 0 }} cards={[]} onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={fold} />);
  await act(async () => document.querySelector<HTMLElement>("[data-desk-glance-fold]")!.click());
  expect(fold).toHaveBeenCalledTimes(1);
});

it("shows a capped number of files with `… n개 더`, which shows the rest", async () => {
  const files = Array.from({ length: GLANCE_FILE_ROWS + 3 }, (_, at) => file(`f${at}`, "p1", true));
  await render(<GlancePanel glance={{ sessions: ["p1"], files, files_more: 4 }} cards={[card("p1", "one", "working")]} tab="files" onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={noop} />);
  expect(all("[data-desk-glance-file]")).toHaveLength(GLANCE_FILE_ROWS);
  const more = document.querySelector<HTMLElement>("[data-desk-glance-more]")!;
  expect(more.dataset.deskGlanceMore).toBe("7");
  expect(document.querySelector("[data-desk-glance-files]")?.getAttribute("data-desk-glance-files")).toBe(String(GLANCE_FILE_ROWS + 7));
  await act(async () => more.click());
  expect(all("[data-desk-glance-file]")).toHaveLength(GLANCE_FILE_ROWS + 3);
  expect(document.querySelector("[data-desk-glance-more]")).toBeNull();
  expect(document.querySelector("[data-desk-glance-older]")).not.toBeNull();
});

it("says a session on a device that is not connected is so, dimmed, as its card does", async () => {
  const away = { ...card("p1", "기기 세션", "working"), disconnected: true };
  await render(<GlancePanel glance={{ sessions: ["p1"], files: [], files_more: 0 }} cards={[away]} onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={noop} />);
  const row = document.querySelector<HTMLElement>("[data-desk-glance-session]")!;
  expect(row.dataset.deskGlanceDisconnected).toBe("true");
  expect(row.textContent).toContain("Not connected");
});

it("says on each tab what gathers there while nothing has yet, with no search box", async () => {
  await render(<GlancePanel glance={{ sessions: [], files: [], files_more: 0 }} cards={[]} onTab={noop} onGroup={noop} onSession={noop} onFile={noop} onFold={noop} />);
  expect(document.querySelector("[data-desk-glance-empty]")?.textContent).toContain("gather here");
  expect(document.querySelector("[data-desk-glance-search]")).toBeNull();
});

it("goes back from the session to 한눈에 with `‹ 한눈에`", async () => {
  const back = vi.fn();
  await render(<GlanceCrumb title="하네스 강의 챕터 설계" onBack={back} />);
  const crumb = document.querySelector("[data-desk-glance-crumb]")!;
  expect(crumb.textContent).toContain("하네스 강의 챕터 설계");
  await act(async () => crumb.querySelector<HTMLElement>("[data-desk-glance-back]")!.click());
  expect(back).toHaveBeenCalledTimes(1);
});
