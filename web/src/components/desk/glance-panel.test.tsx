// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskJob } from "../../desk";
import type { DeskGlance, DeskGlanceFile } from "../../snapshot";
import { TooltipProvider } from "../ui/tooltip";
import { GLANCE_FILE_ROWS, GlanceCrumb, GlancePanel } from "./glance-panel";

// Expected answers are the approved 한눈에 picture (sections 1 and 2) and the
// operator's approved contract: sessions 내 차례 first, files 볼 것 first with their
// session named at the end, a capped list with `… n개 더`, `⟩` folds.

const card = (pane: string, title: string, state: DeskJob["state"]): DeskJob => ({
  id: `pane:${pane}`, agent_id: null, pane_id: pane, title, agent_kind: "claude", project: { workspace_id: "w", label: "w" }, location: { kind: "checkout", label: "main" },
  state, queue_position: null, now: null, plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: state === "asking" ? "question" : null, mark: null, mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
});
const file = (id: string, pane: string | null, seen: boolean): DeskGlanceFile => ({ id, kind: "doc", label: `${id}.md`, thumb: null, seen, pr_state: null, device_id: "mac", checkout: "/repo", path: `/repo/${id}.md`, url: null, pane_id: pane });

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

it("lists the sessions 내 차례 first and the files 볼 것 first, each file naming its session or Desk", async () => {
  const cards = [card("p1", "Play 스토어 등록", "asking"), card("p2", "하네스 강의", "working"), card("p3", "페이지 원인", "done")];
  const glance: DeskGlance = { sessions: ["p3", "p2", "p1"], files: [file("hero", "p1", false), file("plan", null, false), file("roadmap", "p2", true)], files_more: 0 };
  const opened: string[] = [];
  const sessions: string[] = [];
  const fold = vi.fn();
  await render(<GlancePanel glance={glance} cards={cards} onSession={(pane) => sessions.push(pane)} onFile={(one) => opened.push(one.id)} onFold={fold} />);

  expect(all("[data-desk-glance-session]").map((row) => row.dataset.deskGlanceGroup)).toEqual(["turn", "working", "idle"]);
  expect(document.querySelector("[data-desk-glance-sessions]")?.textContent).toContain("Needs You 1");
  const files = all("[data-desk-glance-file]");
  expect(files.map((row) => row.dataset.deskGlanceFile)).toEqual(["hero", "plan", "roadmap"]);
  expect(files.map((row) => row.querySelector("[data-desk-glance-by]")?.textContent)).toEqual(["Play 스토어 등록", "Desk", "하네스 강의"]);
  expect(files.map((row) => row.dataset.deskGlanceUnseen ?? "seen")).toEqual(["true", "true", "seen"]);
  expect(document.querySelector("[data-desk-glance-files]")?.textContent).toContain("To look at 2");

  await act(async () => files[0]!.click());
  await act(async () => all("[data-desk-glance-session]")[1]!.click());
  await act(async () => document.querySelector<HTMLElement>("[data-desk-glance-fold]")!.click());
  expect([opened, sessions, fold.mock.calls.length]).toEqual([["hero"], ["p2"], 1]);
});

it("shows a capped number of files with `… n개 더`, which shows the rest", async () => {
  const files = Array.from({ length: GLANCE_FILE_ROWS + 3 }, (_, at) => file(`f${at}`, "p1", true));
  await render(<GlancePanel glance={{ sessions: ["p1"], files, files_more: 4 }} cards={[card("p1", "one", "working")]} onSession={() => undefined} onFile={() => undefined} onFold={() => undefined} />);
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
  await render(<GlancePanel glance={{ sessions: ["p1"], files: [], files_more: 0 }} cards={[away]} onSession={() => undefined} onFile={() => undefined} onFold={() => undefined} />);
  const row = document.querySelector<HTMLElement>("[data-desk-glance-session]")!;
  expect(row.dataset.deskGlanceDisconnected).toBe("true");
  expect(row.textContent).toContain("Not connected");
});

it("says what gathers here while nothing has yet", async () => {
  await render(<GlancePanel glance={{ sessions: [], files: [], files_more: 0 }} cards={[]} onSession={() => undefined} onFile={() => undefined} onFold={() => undefined} />);
  expect(document.querySelector("[data-desk-glance-empty]")).not.toBeNull();
});

it("goes back from the session to 한눈에 with `‹ 한눈에`", async () => {
  const back = vi.fn();
  await render(<GlanceCrumb title="하네스 강의 챕터 설계" onBack={back} />);
  const crumb = document.querySelector("[data-desk-glance-crumb]")!;
  expect(crumb.textContent).toContain("하네스 강의 챕터 설계");
  await act(async () => crumb.querySelector<HTMLElement>("[data-desk-glance-back]")!.click());
  expect(back).toHaveBeenCalledTimes(1);
});
