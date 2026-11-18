// @vitest-environment jsdom
// A notice in the window's corner (docs/UI_BEHAVIOR.md, Notices): an info one
// leaves after its time, held while the operator is inside it; an important one stays.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Toast, TOAST_INFO_MS, type ToastLevel } from "./toast";
import { TooltipProvider } from "./tooltip";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

/** Draws one notice that leaves the page when it is dismissed. */
function show(level: ToastLevel, timeout?: number) {
  function Shown() {
    return <Toast level={level} timeout={timeout} onDismiss={() => act(() => root.render(null))} dismissLabel="Dismiss">Saved</Toast>;
  }
  act(() => root.render(<TooltipProvider><Shown /></TooltipProvider>));
}

const shown = () => container.querySelector("[data-toast-level]");
const wait = (ms: number) => act(() => vi.advanceTimersByTime(ms));

it("leaves after its time when nothing holds it", () => {
  show("info", TOAST_INFO_MS);
  wait(TOAST_INFO_MS - 1);
  expect(shown()?.textContent).toBe("Saved");
  wait(1);
  expect(shown()).toBeNull();
});

it("holds its count while the keyboard is inside it, and goes on from where it stopped once it leaves", () => {
  show("info", TOAST_INFO_MS);
  wait(TOAST_INFO_MS - 1000);
  act(() => container.querySelector("button")?.focus());
  wait(TOAST_INFO_MS * 2);
  expect(shown()).not.toBeNull();
  act(() => container.querySelector("button")?.blur());
  wait(999);
  expect(shown()).not.toBeNull();
  wait(1);
  expect(shown()).toBeNull();
});

it("stays without a time until it is dismissed, drawn as important", () => {
  show("important");
  wait(TOAST_INFO_MS * 10);
  expect(shown()?.getAttribute("data-toast-level")).toBe("important");
  expect(shown()?.getAttribute("role")).toBe("alert");
  act(() => container.querySelector("button")?.click());
  expect(shown()).toBeNull();
});
