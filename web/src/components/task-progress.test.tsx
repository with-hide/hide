// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { clientI18n } from "../i18n/translator";
import type { TaskState } from "../sessionWork";
import { shownTaskState, TaskMark, TaskWord } from "./task-progress";
import { TooltipProvider } from "./ui/tooltip";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// A checklist task's mark is a checkbox (PRD record-card-readability D-05,
// B7-B9): ☑ done, ☐ otherwise; a live session's question or block adds `!`,
// an ended one says it in words; no ring, dot, ?, or ⛔ on a task.
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("draws each task state as a checkbox, with `!` and words only where the box does not say enough", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  // A checklist task's states (sessionWork.taskState), not an agent's, spelled as one string so the
  // core's guard against a shell-side agent state vocabulary (axes_no_state_name_array_remains_in_the_shell) reads no array.
  const states = "done working question blocked unfinished review ci_failed".split(" ") as TaskState[];
  const asked = "question blocked".split(" ") as TaskState[];
  await act(async () => root.render(<TooltipProvider>
    {states.map((state) => <div key={state} data-case={state}><TaskMark state={state} live /><TaskWord state={state} live prShown /></div>)}
    {asked.map((state) => <div key={state} data-case={`ended-${state}`}><TaskMark state={state} live={false} /><TaskWord state={state} live={false} prShown /></div>)}
    <div data-case="grouped"><TaskWord state="ci_failed" live prShown={false} /></div>
  </TooltipProvider>));
  try {
    const box = (name: string) => container.querySelector(`[data-case="${name}"] [data-task-box]`)!.getAttribute("data-task-box");
    const asks = (name: string) => container.querySelector(`[data-case="${name}"] [data-task-asks]`) !== null;
    expect(states.map(box)).toEqual(["checked", "empty", "empty", "empty", "empty", "empty", "empty"]);
    expect(states.filter(asks)).toEqual(asked);
    expect([asks("ended-question"), asks("ended-blocked")]).toEqual([false, false]);
    const words = [...container.querySelectorAll("[data-task-word]")].map((word) => [word.closest("[data-case]")!.getAttribute("data-case"), word.textContent]);
    expect(words).toEqual([
      ["question", "내 차례 · 질문"], ["blocked", "내 차례 · 막힘"], ["unfinished", "일 남음"],
      ["ended-question", "일 남음 · 질문하고 끝남"], ["ended-blocked", "일 남음 · 막혀서 끝남"], ["grouped", "CI 실패"],
    ]);
    expect(container.querySelector("[data-mark]")).toBeNull();
    expect(container.textContent).not.toMatch(/[?⛔◐]/);
  } finally {
    await act(async () => root.unmount());
  }
});

it("draws a task left in progress as work left once its row stops", () => {
  expect(shownTaskState("working", "going")).toBe("working");
  expect(shownTaskState("working", "waiting")).toBe("working");
  for (const mark of ["unread", "quiet", "done", "turn", null] as const) expect(shownTaskState("working", mark)).toBe("unfinished");
  expect(shownTaskState("question", "quiet")).toBe("question");
});

// PRD task-continuation D-07, D-09: a done task whose turn ended on a
// question keeps the `!` in front of its ☑; a task whose newest turn went
// unjudged draws only its empty box, never `일 남음`.
it("keeps `!` before a done task that asks and draws an unjudged task as a bare box", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>
    <div data-case="done-asks"><TaskMark state="done" live question /><TaskWord state="done" live prShown /></div>
    <div data-case="done-ended"><TaskMark state="done" live={false} question /></div>
    <div data-case="unjudged"><TaskMark state="unjudged" live /><TaskWord state="unjudged" live prShown={false} /></div>
  </TooltipProvider>));
  try {
    const at = (name: string, selector: string) => container.querySelector(`[data-case="${name}"] ${selector}`);
    expect(at("done-asks", "[data-task-box]")?.getAttribute("data-task-box")).toBe("checked");
    expect(at("done-asks", "[data-task-asks]")).not.toBeNull();
    expect(at("done-asks", "[data-task-word]")).toBeNull();
    expect(at("done-ended", "[data-task-asks]")).toBeNull();
    expect(at("unjudged", "[data-task-box]")?.getAttribute("data-task-box")).toBe("empty");
    expect(at("unjudged", "[data-task-asks]")).toBeNull();
    expect(at("unjudged", "[data-task-word]")).toBeNull();
    expect(shownTaskState("unjudged", "quiet")).toBe("unjudged");
  } finally {
    await act(async () => root.unmount());
  }
});
