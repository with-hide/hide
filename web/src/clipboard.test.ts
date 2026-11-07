// @vitest-environment jsdom
import { afterAll, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { takeProgramCopy } from "./clipboard";
import { useShellStore } from "./store";
import type { SnapshotRest } from "./snapshot";
import { attachTerminal, liveTerminalIds, terminalShown } from "./terminals";

// jsdom has no canvas: answer xterm's capability probe at the browser
// boundary and keep real terminals.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

function recorder() {
  const written: [string, string][] = [];
  const logged: string[] = [];
  const write = (text: string, what: string) => {
    written.push([text, what]);
    return Promise.resolve(true);
  };
  return { written, logged, write, diagnostic: (message: string) => logged.push(message) };
}

// with-hide/hide-private#659: the window in front writes a program's copy,
// its text read as UTF-8, as a terminal writes it for the client in front.
it("the window in front writes a program's copy as its text", async () => {
  const seen = recorder();
  const text = "복사 – copied\n";
  const encoded = btoa(String.fromCharCode(...new TextEncoder().encode(text)));
  expect(await takeProgramCopy({ pane_id: "w1:p1", text_base64: encoded }, { inFront: true, shows: () => true }, seen.write, seen.diagnostic)).toBe(true);
  expect(seen.written).toEqual([[text, "pane w1:p1 copy"]]);
  expect(seen.logged).toEqual([]);
});

// Every window is handed the copy; one behind leaves the clipboard alone and
// says so in the log only.
it("a window behind leaves the clipboard alone", async () => {
  const seen = recorder();
  expect(await takeProgramCopy({ pane_id: "w1:p1", text_base64: "eA==" }, { inFront: false, shows: () => true }, seen.write, seen.diagnostic)).toBe(false);
  expect(seen.written).toEqual([]);
  expect(seen.logged).toHaveLength(1);
});

// with-hide/hide-private#659 review R1: every pane's copy reaches the window,
// and a pane kept attached behind another tab, screen or zoom is parked with
// no host; its copy must not replace the one the operator made on screen.
it("only a pane on this window's screen writes its copy", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }));
  document.documentElement.style.setProperty("--text-terminal-base", "14px");
  document.body.innerHTML = '<div data-pane-view="shown"></div><div data-pane-view="hidden"></div>';
  // The core streams both: the hidden pane stays attached while parked.
  useShellStore.setState({ rest: { terminal: { panes: [{ pane_id: "shown", transport_state: "attached" }, { pane_id: "hidden", transport_state: "attached" }] } } as unknown as SnapshotRest });
  const actions = createActions(() => true);
  const host = (pane: string) => document.querySelector<HTMLElement>(`[data-pane-view="${pane}"]`)!;
  const detachShown = attachTerminal("shown", host("shown"), () => true, actions, 1);
  // Shown once, then its tab was left: parked, still attached for output.
  attachTerminal("hidden", host("hidden"), () => true, actions, 1)();
  try {
    expect(liveTerminalIds()).toContain("hidden");
    expect([terminalShown("shown"), terminalShown("hidden"), terminalShown("never")]).toEqual([true, false, false]);
    const seen = recorder();
    const window = { inFront: true, shows: terminalShown };
    expect(await takeProgramCopy({ pane_id: "hidden", text_base64: btoa("hidden") }, window, seen.write, seen.diagnostic)).toBe(false);
    expect(await takeProgramCopy({ pane_id: "never", text_base64: btoa("never") }, window, seen.write, seen.diagnostic)).toBe(false);
    expect(await takeProgramCopy({ pane_id: "shown", text_base64: btoa("shown") }, window, seen.write, seen.diagnostic)).toBe(true);
    expect(seen.written).toEqual([["shown", "pane shown copy"]]);
    expect(seen.logged).toHaveLength(2);
  } finally {
    detachShown();
    useShellStore.setState({ rest: null });
    vi.unstubAllGlobals();
  }
});
