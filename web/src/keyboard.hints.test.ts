import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { installKeyboard } from "./keyboard";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

vi.mock("./host", () => ({ hostKind: () => "electron", keySystem: () => "mac", hostBridge: () => null, browserBridge: () => null }));
vi.mock("./viewFocus", () => ({
  installKeyboardOwner: () => () => {}, subscribeKeyboardOwner: () => () => {},
  keyboardOwner: () => ({ kind: "none" }), keyboardCommandOwner: () => ({ kind: "none" }),
  drawnViews: () => null, noteCommandDelivered: () => {},
}));

const listeners = new Map<string, Set<(event: KeyboardEvent) => void>>();
const pending = new Map<number, { callback: () => void; delay: number }>();
let now: number;
let sequence: number;
let remove: (() => void) | undefined;

function emit(type: string, key = "Meta", metaKey = true) {
  const event = { key, code: key === "Meta" ? "MetaLeft" : "KeyA", metaKey, altKey: false, ctrlKey: false, shiftKey: false, isComposing: false, keyCode: 0, preventDefault() {}, stopPropagation() {} } as KeyboardEvent;
  for (const listener of listeners.get(type) ?? []) listener(event);
}

function wake(at: number) {
  expect(pending.size).toBe(1);
  const [id, timer] = [...pending][0]!;
  pending.delete(id);
  now = at;
  timer.callback();
}

beforeEach(() => {
  now = 1000.3;
  sequence = 0;
  listeners.clear();
  pending.clear();
  vi.stubGlobal("performance", { now: () => now });
  vi.stubGlobal("setTimeout", (callback: () => void, delay: number) => {
    const id = ++sequence;
    pending.set(id, { callback, delay });
    return id;
  });
  vi.stubGlobal("clearTimeout", (id: number) => pending.delete(id));
  const events = {
    addEventListener: (type: string, listener: (event: KeyboardEvent) => void) => {
      const set = listeners.get(type) ?? new Set();
      set.add(listener);
      listeners.set(type, set);
    },
    removeEventListener: (type: string, listener: (event: KeyboardEvent) => void) => listeners.get(type)?.delete(listener),
  };
  vi.stubGlobal("window", events);
  vi.stubGlobal("document", { ...events, visibilityState: "visible" });
  useShellStore.setState({ rest: null });
  useUiStore.setState({ hint: null, cycle: null, overlay: "none", escapeLayers: [], workspaceDialog: null, pendingClose: null, pendingTrash: null });
  remove = installKeyboard(createActions(() => {}));
});

afterEach(() => {
  remove?.();
  remove = undefined;
  vi.unstubAllGlobals();
});

describe("modifier hint deadline scheduling", () => {
  it("rearms an early wake without revealing, then reveals once at maturity", () => {
    const reveals: unknown[] = [];
    const unsubscribe = useUiStore.subscribe((state, previous) => { if (state.hint !== previous.hint) reveals.push(state.hint); });
    try {
      emit("keydown");
      wake(1150.1);
      expect(useUiStore.getState().hint).toBeNull();
      expect(pending.size).toBe(1);
      expect([...pending.values()][0]!.delay).toBeGreaterThan(0);
      wake(1150.3);
      expect(useUiStore.getState().hint).toBe("tabs");
      expect(reveals).toEqual(["tabs"]);
      expect(pending.size).toBe(0);
    } finally { unsubscribe(); }
  });

  it.each(["release", "key", "blur", "layer", "unmount"])("cancels a rearmed hint on %s without a late reveal", (reason) => {
    emit("keydown");
    wake(1150.1);
    expect(pending.size).toBe(1);
    if (reason === "release") emit("keyup", "Meta", false);
    else if (reason === "key") emit("keydown", "a");
    else if (reason === "blur") emit("blur");
    else if (reason === "layer") useUiStore.getState().openOverlay("shortcuts");
    else { remove?.(); remove = undefined; }
    expect(pending.size).toBe(0);
    expect(useUiStore.getState().hint).toBeNull();
  });
});
