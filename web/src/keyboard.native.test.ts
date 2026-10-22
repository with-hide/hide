import { emptyScope } from "../test/legacyAgentScope";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { noteAreaFrame } from "./areaFrames";
import { areaGeometry } from "./areaLayout";
import { registerBrowserSlot, syncBrowserFront } from "./browserViews";
import type { BrowserHostEvent, HostBridge } from "./host";
import { installKeyboard, reconcileHeldCycle } from "./keyboard";
import { resetRecent } from "./recent";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { noteKeyboardOwner } from "./viewFocus";

// Fake only the platform bridge/DOM/animation clock. Scope, recent order,
// keyboard controller, actions and visible-slot synchronization stay real.
const browserListeners = new Set<(event: BrowserHostEvent) => void>();
const ended: number[] = [];
const pageCommands: [string, string, string][] = [];
const frames = new Map<number, FrameRequestCallback>();
const events: unknown[] = [];
let frameSequence = 0;
let remove: (() => void) | undefined;
let unregister: (() => void) | undefined;

class PlatformKeyboardEvent extends Event {
  readonly key: string;
  readonly code: string;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
  readonly isComposing = false;
  readonly keyCode = 0;
  constructor(type: string, init: KeyboardEventInit = {}) {
    super(type, { cancelable: true });
    this.key = init.key ?? "";
    this.code = init.code ?? "";
    this.ctrlKey = !!init.ctrlKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
    this.shiftKey = !!init.shiftKey;
  }
}

const bridge: HostBridge = {
  kind: "electron", platform: "darwin", onCommand: () => () => {}, reportBindings() {}, reportLanguage() {},
  revealPath() {}, pickFolder: async () => null, probePaths: async () => [], openPath() {}, notify() {}, onNotificationOpen: () => () => {}, withdrawNotifications() {}, notificationPermission: async () => "unsupported", onWindowChrome: () => () => undefined, editorAvailable: async () => false, openInEditor: () => undefined, openNotificationSettings() {}, reportBadge() {},
  updates: { onStatus: () => () => {}, check() {}, restart() {}, setPrerelease() {} },
  browser: {
    sync() {}, capture: async () => null,
    endCycle: (id) => { ended.push(id); },
    command: (workspace, id, command) => { pageCommands.push([workspace, id, command]); },
    onEvent: (listener) => { browserListeners.add(listener); return () => { browserListeners.delete(listener); }; },
  },
};

function draw(count = 2) {
  const displays = Array.from({ length: count }, (_, index) => ({ id: `d${index + 1}`, kind: "browser", label: `Page ${index + 1}`, url: `https://page${index + 1}.test/`, state: "open", tab_id: null }));
  const layout = { root: { area: { id: "a1", active: "d1", displays } }, active_area: "a1", display_count: count, limits: { areas: 6, depth: 5, displays: 64 } };
  const rest = {
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }], workspaces: [{ agent_scope: emptyScope(), id: "w", label: "fixture", device_id: "local", checkouts: [{ agent_scope: emptyScope(), id: "c", workspace_id: "w", path: "/fixture", label: "fixture", tabs: [], active_tab_id: null, strip: [] }] }] },
    workspace_view: { device_id: "local", path: "/fixture", open: true, documents: true, tool: null, layout },
  } as unknown as SnapshotRest;
  const sizes = { areaMinWidth: 100, areaMinHeight: 100, divider: 4, tabStrip: 30 };
  const workspace = { device_id: "local", path: "/fixture" };
  noteAreaFrame("view", { workspace, layout, geometry: areaGeometry(layout.root, { x: 0, y: 0, width: 1000, height: 800 }, sizes), sizes } as never);
  useShellStore.setState({ rest });
  noteKeyboardOwner({ kind: "view", workspace: "c", areaId: "a1" });
  syncBrowserFront(workspace, rest.workspace_view!.layout, displays.map((display) => ({ ...workspace, view_id: display.id, area_id: "a1" })));
  unregister = registerBrowserSlot("d1", { isConnected: true, closest: () => null, getBoundingClientRect: () => ({ left: 0, top: 30, width: 1000, height: 770 }) } as unknown as HTMLElement);
}

function flushFrames() {
  const queued = [...frames.values()];
  frames.clear();
  for (const callback of queued) callback(0);
}

function input(type: "keyDown" | "keyUp" = "keyDown", key = "Tab", cycleId = 1) {
  const event: BrowserHostEvent = { kind: "cycle-input", workspace: "local\u0000/fixture", id: "d1", cycleId, type, key, code: key === "Control" ? "ControlLeft" : key, control: key !== "Control", alt: false, meta: false, shift: false };
  for (const listener of browserListeners) listener(event);
}

beforeEach(() => {
  ended.length = 0;
  pageCommands.length = 0;
  events.length = 0;
  frames.clear();
  const body = {};
  vi.stubGlobal("window", Object.assign(new EventTarget(), { hideHost: bridge }));
  vi.stubGlobal("document", Object.assign(new EventTarget(), { body, activeElement: body, visibilityState: "visible", documentElement: { hasAttribute: () => false }, querySelectorAll: () => [] }));
  vi.stubGlobal("KeyboardEvent", PlatformKeyboardEvent);
  vi.stubGlobal("Element", class {});
  vi.stubGlobal("HTMLInputElement", class {});
  vi.stubGlobal("HTMLTextAreaElement", class {});
  vi.stubGlobal("MutationObserver", class { observe() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { const id = ++frameSequence; frames.set(id, callback); return id; });
  resetRecent();
  useUiStore.setState({ screen: { kind: "workspace" }, cycle: null, hint: null, overlay: "none", escapeLayers: [], workspaceDialog: null, pendingClose: null, pendingTrash: null, recordingShortcut: false, viewFocusRequest: null });
  remove = installKeyboard(createActions((event) => { events.push(event); }));
  draw();
});

afterEach(() => {
  remove?.(); remove = undefined;
  unregister?.(); unregister = undefined;
  flushFrames();
  noteAreaFrame("view", null);
  noteAreaFrame("agent", null);
  vi.unstubAllGlobals();
});

describe("native cycle responder return", () => {
  it.each(["one item", "no owner", "shortcut recording"])("returns the visible origin after a rejected first start: %s", (reason) => {
    if (reason === "one item") { unregister?.(); draw(1); }
    else if (reason === "no owner") noteKeyboardOwner({ kind: "none" });
    else useUiStore.setState({ recordingShortcut: true });
    input();
    flushFrames();
    expect(useUiStore.getState().cycle).toBeNull();
    expect(events).toEqual([]);
    expect(ended).toEqual([1]);
    expect(pageCommands).toEqual([["local\u0000/fixture", "d1", "focus"]]);
  });

  it("processes a queued start and immediate release into one commit, without restoring over the chosen page", () => {
    // The host queues these on one channel before renderer delivery. Preview
    // does not select a tab; release commits exactly once after initialization.
    const queued = [() => input(), () => input("keyUp", "Control")];
    expect(useUiStore.getState().cycle).toBeNull();
    queued[0]!();
    expect(useUiStore.getState().cycle?.kind).toBe("area");
    expect(events).toEqual([]);
    queued[1]!();
    flushFrames();
    expect(useUiStore.getState().cycle).toBeNull();
    expect(events).toEqual([expect.objectContaining({ kind: "view_layout", payload: expect.objectContaining({ action: "focus", display_id: "d2" }) })]);
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: "local\u0000/fixture", displayId: "d2", from: null });
    expect(ended).toEqual([1]);
    expect(pageCommands).toEqual([]);
    input("keyUp", "Control");
    flushFrames();
    expect(events).toHaveLength(1);
    expect(pageCommands).toEqual([]);
  });

  it("a release back on the origin commits nothing and returns the keyboard to the page", () => {
    input();
    input();
    input("keyUp", "Control");
    flushFrames();
    expect(useUiStore.getState().cycle).toBeNull();
    expect(events).toEqual([]);
    expect(ended).toEqual([1]);
    expect(pageCommands).toEqual([["local\u0000/fixture", "d1", "focus"]]);
  });

  it("a hold whose area shrinks to one tab ends with the keyboard back on the page", () => {
    input();
    unregister?.();
    draw(1);
    const cycle = useUiStore.getState().cycle!;
    useUiStore.getState().setCycle(reconcileHeldCycle(cycle, useShellStore.getState().rest));
    flushFrames();
    expect(useUiStore.getState().cycle).toBeNull();
    expect(events).toEqual([]);
    expect(ended).toEqual([1]);
    expect(pageCommands).toEqual([["local\u0000/fixture", "d1", "focus"]]);
  });

  it("Escape restores the captured origin with no commit", () => {
    input();
    input("keyDown", "Escape");
    flushFrames();
    expect(useUiStore.getState().cycle).toBeNull();
    expect(events).toEqual([]);
    expect(ended).toEqual([1]);
    expect(pageCommands).toEqual([["local\u0000/fixture", "d1", "focus"]]);
  });
});
