// @vitest-environment jsdom
import { afterAll, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { applyTerminalFontSize, attachTerminal } from "./terminals";

// docs/UI_BEHAVIOR.md, The pane floor: a window's measure names the Terminal
// text size it was taken at, and the core judges no measure of another size.

// jsdom has no canvas: answer xterm's capability probe at the browser
// boundary, and measure a cell as 0.6 of its font size.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
vi.mock("./terminalFonts", async (importOriginal) => ({ ...(await importOriginal<typeof import("./terminalFonts")>()), cellWidth: (size: number) => size * 0.6 }));

it("sends a pane's measure at a new Terminal text size its own size rounds to the same", () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }));
  document.documentElement.style.setProperty("--text-terminal-base", "14px");
  document.documentElement.style.setProperty("--spacing-sm", "8px");
  document.body.innerHTML = '<div data-canvas><div data-pane-view="p1"></div></div>';
  Object.defineProperty(document.querySelector("[data-canvas]")!, "clientWidth", { value: 1000 });
  const sent: { kind: string; payload: { floor?: { font_size: number | null; cell_px: number } } }[] = [];
  applyTerminalFontSize(15);
  // A pane at 0.7: 15 and 16 both draw it at 11 px.
  attachTerminal("p1", document.querySelector<HTMLElement>('[data-pane-view="p1"]')!, (event) => { sent.push(event as (typeof sent)[number]); return true; }, createActions(() => true), 0.7);
  sent.length = 0;
  applyTerminalFontSize(16);
  const resizes = sent.filter((event) => event.kind === "terminal_resize");
  expect(resizes.map((event) => event.payload.floor?.font_size)).toEqual([16]);
  expect(resizes[0]?.payload.floor?.cell_px).toBeCloseTo(11 * 0.6);
});
