// @vitest-environment jsdom
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { useShellStore } from "./store";
import type { SnapshotRest } from "./snapshot";
import { attachTerminal, pasteText, typeText } from "./terminals";

// jsdom has no canvas: answer xterm's capability probe at the browser
// boundary and keep real terminals.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

let detach = () => {};
afterEach(() => {
  detach();
  useShellStore.setState({ rest: null });
  vi.unstubAllGlobals();
});

/** A real terminal for `pane`, and the `key` events it sends, their bytes read back as text. */
function terminal(pane: string) {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} }));
  document.documentElement.style.setProperty("--text-terminal-base", "14px");
  document.body.innerHTML = `<div data-pane-view="${pane}"></div>`;
  useShellStore.setState({ rest: { terminal: { panes: [{ pane_id: pane, transport_state: "attached" }] } } as unknown as SnapshotRest });
  const events: { kind: string; payload: Record<string, unknown> }[] = [];
  detach = attachTerminal(pane, document.querySelector<HTMLElement>(`[data-pane-view="${pane}"]`)!, (event) => {
    events.push(event as (typeof events)[number]);
    return true;
  }, createActions(() => true), 1);
  return () => events
    .filter((event) => event.kind === "key")
    .map(({ payload }) => ({ ...payload, bytes_base64: new TextDecoder().decode(Uint8Array.from(atob(String(payload.bytes_base64)), (char) => char.charCodeAt(0))) }));
}

// with-hide/hide issue 2952: the terminal never sees the program turn bracketed
// paste on, so a paste goes marked as one for the pane's holder to wrap, its
// line breaks carriage returns as a terminal sends them, never as keys.
it("a paste goes to the pane marked as one, its line breaks carriage returns", () => {
  const sent = terminal("w1:p1");
  pasteText("w1:p1", "one\ntwo\r\nthree");
  expect(sent()).toEqual([{ pane_id: "w1:p1", bytes_base64: "one\rtwo\rthree", paste: true }]);
});

it("typed text goes as unmarked keys, each carriage return an Enter", () => {
  const sent = terminal("w1:p1");
  typeText("w1:p1", "\x15hi\r");
  expect(sent()).toEqual([{ pane_id: "w1:p1", bytes_base64: "\x15hi\r" }]);
});
