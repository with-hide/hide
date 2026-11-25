import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { closeCovering, installKeyboard } from "./keyboard";
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
const actions = createActions(() => true);

function emit(type: string, key: string, modifiers: { metaKey?: boolean; altKey?: boolean } = {}) {
  const event = { key, code: key === "Alt" ? "AltLeft" : key, metaKey: modifiers.metaKey ?? false, altKey: modifiers.altKey ?? false, ctrlKey: false, shiftKey: false, isComposing: false, keyCode: 0, target: null, preventDefault() {}, stopPropagation() {} } as unknown as KeyboardEvent;
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
  // Escape reads whether a text field holding text has the keyboard.
  vi.stubGlobal("HTMLInputElement", class {});
  vi.stubGlobal("HTMLTextAreaElement", class {});
  vi.stubGlobal("document", { ...events, visibilityState: "visible" });
  useShellStore.setState({ rest: null });
  useUiStore.setState({ hint: null, cycle: null, overlay: "none", escapeLayers: [], workspaceDialog: null, pendingClose: null, pendingTrash: null, overviewOpen: false, screen: { kind: "desk" }, deskRoom: { session: "w1:p2", panel: null, closed: null, answer: 0 } });
  remove = installKeyboard(actions);
});

afterEach(() => {
  remove?.();
  remove = undefined;
  vi.unstubAllGlobals();
});

// PRD desk-2-ui B6, review S3: the session open beside Desk covers nothing,
// so it is no Escape layer: Escape closes it only once no layer, close
// confirmation or overlay is open, while ⌥'s keycaps, the Overview and ⌘W act
// as they do with nothing beside Desk.
describe("Desk with a session open beside it", () => {
  it("closes an open layer first and the session on the next Escape", () => {
    const closed: string[] = [];
    const pop = useUiStore.getState().pushEscape(() => {
      closed.push("panel");
      pop();
    });
    emit("keydown", "Escape");
    expect(closed).toEqual(["panel"]);
    expect(useUiStore.getState().deskRoom.session).toBe("w1:p2");
    emit("keydown", "Escape");
    expect(useUiStore.getState().deskRoom.session).toBeNull();
  });

  it("answers a close confirmation before it closes the session", () => {
    useUiStore.setState({ pendingClose: { kind: "pane" } as never });
    emit("keydown", "Escape");
    expect(useUiStore.getState().pendingClose).toBeNull();
    expect(useUiStore.getState().deskRoom.session).toBe("w1:p2");
  });

  it("reveals ⌥'s keycaps, opens the Overview and leaves ⌘W to the pane while it is open", () => {
    emit("keydown", "Alt", { altKey: true });
    wake(1200);
    expect(useUiStore.getState().hint).not.toBeNull();
    emit("keyup", "Alt");
    // ⌘W: nothing covers the session, so the chord reaches the pane in front.
    expect(closeCovering(actions)).toBe(false);
    actions.toggleOverview();
    expect(useUiStore.getState().overviewOpen || useUiStore.getState().screen?.kind === "main").toBe(true);
  });
});

// Operator decision 2026-11-25: in Desk ⌘E is the work tab's right-lane toggle, the session panel being Desk's right side.
describe("⌘E beside Desk", () => {
  const room = () => useUiStore.getState().deskRoom;

  it("closes the session panel, and the next ⌘E opens the same session again", () => {
    emit("keydown", "KeyE", { metaKey: true });
    expect(room().session).toBeNull();
    emit("keydown", "KeyE", { metaKey: true });
    expect(room().session).toBe("w1:p2");
  });

  it("opens the session a ✕ or Escape just closed, and the one pressed last", () => {
    actions.deskCloseSession();
    emit("keydown", "KeyE", { metaKey: true });
    expect(room().session).toBe("w1:p2");
    useUiStore.getState().setDeskRoom((now) => ({ ...now, session: "w1:p3" }));
    emit("keydown", "KeyE", { metaKey: true });
    emit("keydown", "KeyE", { metaKey: true });
    expect(room().session).toBe("w1:p3");
  });

  it("does nothing while no session was closed during this visit of the room", () => {
    useUiStore.setState({ deskRoom: { session: null, panel: null, closed: null, answer: 0 } });
    emit("keydown", "KeyE", { metaKey: true });
    expect(room().session).toBeNull();
    expect(room().closed).toBeNull();
  });
});
