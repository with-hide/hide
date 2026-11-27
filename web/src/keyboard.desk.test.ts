import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { DESK_ROOM_CLOSED } from "./deskRoom";
import { closeCovering, installKeyboard, pressDeskRow } from "./keyboard";
import type { AgentRow, DeskSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { workspaceKey } from "./viewLayout";

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

function emit(type: string, key: string, modifiers: { metaKey?: boolean; altKey?: boolean; ctrlKey?: boolean } = {}) {
  const event = { key, code: key === "Alt" ? "AltLeft" : key, metaKey: modifiers.metaKey ?? false, altKey: modifiers.altKey ?? false, ctrlKey: modifiers.ctrlKey ?? false, shiftKey: false, isComposing: false, keyCode: 0, target: null, preventDefault() {}, stopPropagation() {} } as unknown as KeyboardEvent;
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
  useShellStore.setState({ rest: null, desk: { settings: { enabled: true } } as unknown as DeskSection, agents: ["w1:p2", "w1:p3"].map((pane_id) => ({ pane_id })) as unknown as AgentRow[] });
  useUiStore.setState({ hint: null, cycle: null, overlay: "none", escapeLayers: [], workspaceDialog: null, pendingClose: null, pendingTrash: null, overviewOpen: false, screen: { kind: "desk" }, deskRoom: { ...DESK_ROOM_CLOSED, session: "w1:p2" } });
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

// Operator decision 2026-11-27 (mission-109, replacing 2026-11-25's close and reopen of the session): ⌘E in Desk's room
// opens and folds Desk's desk, as it opens and folds a work tab's right lane; the session beside Desk stays, and
// Escape and `‹ 한눈에` close it.
describe("⌘E in Desk's room", () => {
  const room = () => useUiStore.getState().deskRoom;
  const layout = { root: { area: { id: "a2", active: "d1", displays: [{ id: "d1" }] } }, active_area: "a2", display_count: 1, limits: { areas: 6, depth: 3, displays: 32 } };
  const withTabs = (count: number) => useShellStore.setState({ rest: { desk_view: { device_id: "mac", path: "hide:desk", layout: { ...layout, display_count: count }, opened: 0 } } as unknown as SnapshotRest });

  it("opens Desk's desk with the keyboard in its area in front, folds it on the next ⌘E, and leaves the session beside Desk as it is", () => {
    withTabs(1);
    emit("keydown", "KeyE", { metaKey: true });
    expect([room().desk, room().session]).toEqual([true, "w1:p2"]);
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: workspaceKey({ device_id: "mac", path: "hide:desk" }), areaId: "a2" });
    emit("keydown", "KeyE", { metaKey: true });
    expect([room().desk, room().session]).toEqual([false, "w1:p2"]);
  });

  it("opens Desk's desk with no session beside Desk too, and nothing while the desk holds no tab", () => {
    useUiStore.setState({ deskRoom: DESK_ROOM_CLOSED, viewFocusRequest: null });
    withTabs(0);
    emit("keydown", "KeyE", { metaKey: true });
    expect([room().desk, useUiStore.getState().viewFocusRequest]).toEqual([false, null]);
    withTabs(2);
    emit("keydown", "KeyE", { metaKey: true });
    expect([room().desk, room().session]).toEqual([true, null]);
  });

  it("keeps the session beside Desk when the room is left and entered again, the desk folded", () => {
    withTabs(1);
    emit("keydown", "KeyE", { metaKey: true });
    useUiStore.setState({ screen: { kind: "main" } });
    useUiStore.setState({ screen: { kind: "desk" } });
    expect([room().session, room().desk]).toEqual(["w1:p2", false]);
  });
});

// Feedback 1 D2: a press on the sidebar's Desk row while Desk is in front
// leaves Desk as its head's `나가기 ✕` does, for the place used before it, or
// All projects when there is none. ⌃⇥ in Desk's room walks the recent places,
// Desk among them (issue 301), so its first chord lands on that same place.
describe("leaving Desk", () => {
  it("lands the recent-tab chord pressed in Desk's room on the place used before Desk, and does nothing with none", () => {
    emit("keydown", "Tab", { ctrlKey: true });
    emit("keyup", "Control");
    expect(useUiStore.getState().screen).toEqual({ kind: "desk" });

    const mission = { id: "m1", title: "payments refactor", device_id: "local", workspace_id: "w", project: "/w", checkout_path: "/w", status: "running", nodes: [] };
    useShellStore.setState({ rest: { navigator: { focused_device_id: "local", devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }], workspaces: [] }, missions: { items: [mission] }, ui_state: { recent_places: [{ kind: "desk" }, { kind: "mission", mission_id: "m1" }] } } as unknown as SnapshotRest });
    emit("keydown", "Tab", { ctrlKey: true });
    expect(useUiStore.getState().cycle?.items.map((item) => item.key)).toEqual(["desk", "mission\u0000m1"]);
    emit("keyup", "Control");
    expect(useUiStore.getState().screen).toEqual({ kind: "missions" });
    expect(useUiStore.getState().missions.selected).toBe("m1");
  });

  it("leaves on the Desk row's press while Desk is in front, and opens Desk on it otherwise", () => {
    pressDeskRow(actions);
    expect(useUiStore.getState().screen).toEqual({ kind: "main" });
    pressDeskRow(actions);
    expect(useUiStore.getState().screen?.kind).toBe("desk");
  });
});
