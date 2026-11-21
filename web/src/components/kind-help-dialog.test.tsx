// @vitest-environment jsdom
// The `?` beside a work page's kind mark and the one window it opens: how a
// mission differs from a task, as two pictures that play when it opens.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "../i18n/instance";
import { KindHelpButton } from "./kind-help-dialog";
import { KindMark } from "./kind-mark";
import { TooltipProvider } from "./ui/tooltip";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

function reduced(on: boolean) {
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: on && query.includes("reduce"), addEventListener() {}, removeEventListener() {} }));
}

async function mount(from: "mission" | "task") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><KindMark kind={from} /><KindHelpButton from={from} /></TooltipProvider>));
  const button = () => container.querySelector<HTMLButtonElement>(`[data-kind-help=${from}]`)!;
  const dialog = () => document.querySelector<HTMLElement>("[data-kind-help-dialog]");
  const stage = (kind: string) => document.querySelector<HTMLElement>(`[data-kind-help-stage=${kind}]`)!;
  return { container, button, dialog, stage, unmount: async () => { await act(async () => root.unmount()); } };
}

const steps = () => [...document.querySelectorAll<HTMLElement>("[data-kind-help-step]")].map((row) => row.dataset.kindHelpStep);
const workers = () => [...document.querySelectorAll<HTMLElement>("[data-kind-help-worker]")].map((one) => one.dataset.kindHelpWorker);

it("wears the kind's word and mark, and opens the window with both kinds side by side, lighting the side it was pressed from", async () => {
  reduced(false);
  for (const from of ["mission", "task"] as const) {
    const view = await mount(from);
    try {
      expect(view.container.querySelector(`[data-kind-mark=${from}]`)?.textContent).toBe(from === "mission" ? "Mission" : "Task");
      expect(view.button().getAttribute("aria-label")).toBe("How a mission differs from a task");
      expect(view.dialog()).toBeNull();
      await act(async () => view.button().click());
      expect(view.dialog()?.dataset.kindHelpDialog).toBe(from);
      expect(view.stage("mission").querySelector("[role=img]")?.getAttribute("aria-label")).toBe("A Conductor sends work to three workers at the same time");
      expect(view.stage("task").querySelector("[role=img]")?.getAttribute("aria-label")).toBe("One agent finishes four steps in order");
      expect(view.stage("mission").textContent).toContain("Many at once.");
      expect(view.stage("task").textContent).toContain("One, in turn.");
      expect(view.stage(from).dataset.kindHelpLit).toBe("true");
      expect(view.stage(from === "mission" ? "task" : "mission").dataset.kindHelpLit).toBeUndefined();
    } finally { await view.unmount(); }
  }
});

it("plays the two pictures when it opens, all three workers at once and the steps one after another, and plays them again on 다시 보기", async () => {
  reduced(false);
  vi.useFakeTimers();
  const view = await mount("mission");
  try {
    await act(async () => view.button().click());
    // At the start nothing is lit and every step is still ahead.
    expect(workers()).toEqual(["idle", "idle", "idle"]);
    expect(steps()).toEqual(["planned", "planned", "planned", "planned"]);
    await act(async () => { vi.advanceTimersByTime(450); });
    expect(steps()).toEqual(["working", "planned", "planned", "planned"]);
    // The three workers light together.
    await act(async () => { vi.advanceTimersByTime(450); });
    expect(workers()).toEqual(["lit", "lit", "lit"]);
    expect(steps()).toEqual(["done", "planned", "planned", "planned"]);
    await act(async () => { vi.advanceTimersByTime(450 * 6); });
    expect(steps()).toEqual(["done", "done", "done", "done"]);
    // Again from the start.
    await act(async () => document.querySelector<HTMLButtonElement>("[data-kind-help-replay]")!.click());
    expect(workers()).toEqual(["idle", "idle", "idle"]);
    expect(steps()).toEqual(["planned", "planned", "planned", "planned"]);
  } finally { await view.unmount(); }
});

it("draws both pictures finished and offers no replay while the reduced-motion setting is on", async () => {
  reduced(true);
  vi.useFakeTimers();
  const view = await mount("task");
  try {
    await act(async () => view.button().click());
    expect(workers()).toEqual(["lit", "lit", "lit"]);
    expect(steps()).toEqual(["done", "done", "done", "done"]);
    expect(document.querySelector("[data-kind-help-replay]")).toBeNull();
    expect(document.querySelector("animateMotion")).toBeNull();
  } finally { await view.unmount(); }
});

it("closes on Escape and hands the keyboard back to the `?`", async () => {
  reduced(false);
  const view = await mount("task");
  try {
    view.button().focus();
    await act(async () => view.button().click());
    expect(view.dialog()).not.toBeNull();
    await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(view.dialog()).toBeNull();
  } finally { await view.unmount(); }
});
