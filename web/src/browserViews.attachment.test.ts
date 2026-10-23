// @vitest-environment jsdom
import { act, createElement, Fragment } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { BrowserHost } from "./BrowserDisplay";
import { hostKey, registerBrowserSlot, syncBrowserFront, useBrowserStore } from "./browserViews";
import type { BrowserHostEvent, BrowserSync, HostBridge } from "./host";
import { TooltipProvider } from "./components/ui/tooltip";
import type { SnapshotRest, ViewDisplaySnapshot, ViewLayoutSnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { workspaceKey } from "./viewLayout";
import { DisplayTab } from "./ViewAreas";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});

// Only the platform bridge and frame clock are fakes; sync, stores and subscriptions stay real.
const listeners = new Set<(event: BrowserHostEvent) => void>();
const frames: FrameRequestCallback[] = [];
const sent: BrowserSync[] = [];
const nativeAttached = new Set<string>();
let replayAttachments = false;
let lastNativeEpoch: string | undefined;
const bridge: HostBridge = {
  kind: "electron", platform: "darwin", onCommand: () => () => {}, reportBindings() {}, reportLanguage() {},
  revealPath() {}, pickFolder: async () => null, probePaths: async () => [], openPath() {}, notify() {}, onNotificationOpen: () => () => {}, withdrawNotifications() {}, notificationPermission: async () => "unsupported", onWindowChrome: () => () => undefined, editorAvailable: async () => false, openInEditor: () => undefined, restartHerdr: async () => "unavailable", openNotificationSettings() {}, reportBadge() {},
  updates: { onStatus: () => () => {}, check() {}, restart() {}, setPrerelease() {} },
  browser: {
    sync: (state: BrowserSync) => {
      sent.push(state);
      if (!replayAttachments || state.attachment_epoch === undefined || state.attachment_epoch === lastNativeEpoch) return;
      lastNativeEpoch = state.attachment_epoch;
      for (const row of state.retained) {
        const event: BrowserHostEvent = { kind: "attached", workspace: row.workspace, id: row.id, attached: nativeAttached.has(hostKey(row.workspace, row.id)) };
        for (const listener of listeners) listener(event);
      }
    },
    capture: async () => null, endCycle() {}, command() {},
    onEvent: (listener: (event: BrowserHostEvent) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  },
};
const actions = createActions(() => {});
let root: Root;
let container: HTMLDivElement;
const local = { device_id: "local", path: "/fixture/a" };
const other = { device_id: "local", path: "/fixture/b" };
const remote = { device_id: "ssh", path: "/fixture/a" };
const inventory = [local, other, remote].map((workspace) => ({ ...workspace, view_id: "d1", area_id: "a1" }));
const layout = {
  root: { area: { id: "a1", active: "d1", displays: [{ id: "d1", kind: "browser", url: "https://example.test/", load: 1 }] } },
} as unknown as ViewLayoutSnapshot;

function emit(event: BrowserHostEvent): void {
  act(() => { for (const listener of listeners) listener(event); });
}

function attach(workspace = local, attached = true): void {
  emit({ kind: "attached", workspace: workspaceKey(workspace), id: "d1", attached });
}

function flush(): void {
  act(() => { for (const callback of frames.splice(0)) callback(0); });
}

function latestEpoch(): string {
  const epoch = sent.at(-1)?.attachment_epoch;
  expect(epoch).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i);
  return epoch!;
}

beforeAll(() => {
  window.hideHost = bridge;
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
beforeEach(() => {
  replayAttachments = false;
  nativeAttached.clear();
  lastNativeEpoch = undefined;
  act(() => root.render(null));
  act(() => {
    useShellStore.setState({ connection: "live", rest: null, revision: 0 });
    root.render(createElement(BrowserHost, { actions }));
  });
  syncBrowserFront(null, null, []);
  syncBrowserFront(local, layout, inventory);
  sent.length = 0;
});
afterAll(() => {
  act(() => root.unmount());
  container.remove();
  delete window.hideHost;
  browserCanvas.restore();
  vi.unstubAllGlobals();
});

describe("native browser attachment facts", () => {
  it("does not alias display IDs across checkouts or devices", () => {
    attach();
    expect(useBrowserStore.getState().attached).toEqual({ [hostKey(workspaceKey(local), "d1")]: true });
    attach(other);
    attach(remote);
    attach(local, false);
    expect(useBrowserStore.getState().attached).toEqual({
      [hostKey(workspaceKey(other), "d1")]: true,
      [hostKey(workspaceKey(remote), "d1")]: true,
    });
  });

  it("publishes true/false transitions once and rejects unknown page attachments", () => {
    const publish = vi.fn();
    const unsubscribe = useBrowserStore.subscribe(publish);
    try {
      attach();
      attach();
      emit({ kind: "attached", workspace: workspaceKey(local), id: "d1", attached: "true" } as unknown as BrowserHostEvent);
      attach({ device_id: "local", path: "/not-retained" });
      expect(publish).toHaveBeenCalledTimes(1);
      attach(local, false);
      attach(local, false);
      expect(publish).toHaveBeenCalledTimes(2);
      expect(useBrowserStore.getState().attached).toEqual({});
    } finally {
      unsubscribe();
    }
  });

  it("clears an evicted native page even while its display remains in inventory", () => {
    attach();
    emit({ kind: "gone", workspace: workspaceKey(local), id: "d1", load: 1, url: "https://example.test/" });
    expect(useBrowserStore.getState().attached).toEqual({});
  });

  it("keeps the attachment fact when its authoritative View area changes", () => {
    attach();
    const publish = vi.fn();
    const unsubscribe = useBrowserStore.subscribe(publish);
    try {
      syncBrowserFront(local, { root: { area: { id: "a2", active: "d1", displays: [{ id: "d1", kind: "browser", url: "https://example.test/", load: 1 }] } } } as unknown as ViewLayoutSnapshot,
        inventory.map((row) => row.path === local.path && row.device_id === local.device_id ? { ...row, area_id: "a2" } : row));
      expect(useBrowserStore.getState().attached[hostKey(workspaceKey(local), "d1")]).toBe(true);
      expect(publish).not.toHaveBeenCalled();
    } finally {
      unsubscribe();
    }
  });

  it("keeps hidden retained attachments and prunes inventory or display removal", () => {
    attach();
    attach(other);
    syncBrowserFront(other, layout, inventory);
    expect(useBrowserStore.getState().attached[hostKey(workspaceKey(local), "d1")]).toBe(true);
    syncBrowserFront(other, layout, inventory.filter((row) => row.path === other.path));
    expect(useBrowserStore.getState().attached).toEqual({ [hostKey(workspaceKey(other), "d1")]: true });
    syncBrowserFront(other, { root: { area: { id: "a1", active: null, displays: [] } } } as unknown as ViewLayoutSnapshot, inventory);
    attach(other);
    expect(useBrowserStore.getState().attached).toEqual({});
  });

  it("drops facts on app disconnect and ignores late attachment events", () => {
    attach();
    act(() => useShellStore.getState().setConnection("gone"));
    attach();
    expect(useBrowserStore.getState().attached).toEqual({});
    act(() => useShellStore.getState().setConnection("live"));
    expect(useBrowserStore.getState().attached).toEqual({});
    attach();
    expect(useBrowserStore.getState().attached[hostKey(workspaceKey(local), "d1")]).toBe(true);
  });

  it("syncs authoritative areas for both displayed and retained inventory", () => {
    flush();
    expect(sent.at(-1)).toEqual({
      workspace: workspaceKey(local),
      displays: [{ id: "d1", area_id: "a1", url: "https://example.test/", load: 1, rect: null, visible: false }],
      retained: inventory.map((row) => ({ workspace: workspaceKey(row), id: row.view_id, area_id: row.area_id })),
      authorized_scopes: [],
      attachment_epoch: expect.any(String),
    });
    latestEpoch();
    expect(listeners.size).toBe(1);
  });

  it("forwards positive empty and hidden area scopes with no front Workspace or browser inventory", () => {
    const scopes = [local, other, remote].map((workspace) => ({ ...workspace, area_id: "empty-area", incarnation: 0 }));
    syncBrowserFront(null, null, [], scopes);
    flush();
    expect(sent.at(-1)).toMatchObject({
      workspace: null,
      displays: [],
      retained: [],
      authorized_scopes: [
        { workspace: "local\u0000/fixture/a", area_id: "empty-area", incarnation: 0 },
        { workspace: "local\u0000/fixture/b", area_id: "empty-area", incarnation: 0 },
        { workspace: "ssh\u0000/fixture/a", area_id: "empty-area", incarnation: 0 },
      ],
    });
  });

  it("syncs only core-positive scope changes through snapshot frames without changing the attachment epoch", () => {
    const scopes = [
      { ...local, area_id: "empty-area", incarnation: 5 },
      { ...other, area_id: "a1", incarnation: 6 },
      { ...remote, area_id: "a1", incarnation: 7 },
    ];
    const rest = { workspace_view: { ...local, open: true, documents: true, tool: null, pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", layout }, browser_views: inventory, browser_scopes: scopes } as SnapshotRest;
    act(() => useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 1, rest } }));
    flush();
    expect(sent.at(-1)?.authorized_scopes).toEqual([
      { workspace: "local\u0000/fixture/a", area_id: "empty-area", incarnation: 5 },
      { workspace: "local\u0000/fixture/b", area_id: "a1", incarnation: 6 },
      { workspace: "ssh\u0000/fixture/a", area_id: "a1", incarnation: 7 },
    ]);
    // The visible/inventory area a1 is deliberately absent from local's positive authority.
    const epoch = latestEpoch();
    const displays = sent.at(-1)?.displays;
    const retained = sent.at(-1)?.retained;
    const count = sent.length;
    act(() => useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 2, rest: structuredClone(rest) } }));
    flush();
    expect(sent).toHaveLength(count);
    act(() => useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 3, rest: { ...rest, browser_scopes: [scopes[0]!] } } }));
    flush();
    expect(sent).toHaveLength(count + 1);
    expect(sent.at(-1)?.authorized_scopes).toEqual([{ workspace: "local\u0000/fixture/a", area_id: "empty-area", incarnation: 5 }]);
    expect(sent.at(-1)?.displays).toEqual(displays);
    expect(sent.at(-1)?.retained).toEqual(retained);
    expect(latestEpoch()).toBe(epoch);
    expect(listeners.size).toBe(1);
    // Revoke/regrant may be coalesced by the core: the same area identity must send its new incarnation.
    act(() => useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 4, rest: { ...rest, browser_scopes: [{ ...scopes[0]!, incarnation: 8 }] } } }));
    flush();
    expect(sent).toHaveLength(count + 2);
    expect(sent.at(-1)?.authorized_scopes).toEqual([{ workspace: "local\u0000/fixture/a", area_id: "empty-area", incarnation: 8 }]);
    expect(sent.at(-1)?.displays).toEqual(displays);
    expect(sent.at(-1)?.retained).toEqual(retained);
    expect(latestEpoch()).toBe(epoch);
  });

  it("grants no area authority when core scopes are missing or removed while manual placement remains", () => {
    const manual: SnapshotRest = { workspace_view: { ...local, open: true, documents: true, tool: null, pinned: true, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", layout } as SnapshotRest["workspace_view"], browser_views: inventory };
    const scopes = [{ ...local, area_id: "empty-area", incarnation: 5 }];
    act(() => useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 1, rest: { ...manual, browser_scopes: scopes } } }));
    flush();
    expect(sent.at(-1)?.authorized_scopes).toEqual([{ workspace: "local\u0000/fixture/a", area_id: "empty-area", incarnation: 5 }]);
    act(() => useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 2, rest: manual } }));
    flush();
    expect(sent.at(-1)?.authorized_scopes).toEqual([]);
    expect(sent.at(-1)?.workspace).toBe("local\u0000/fixture/a");
    expect(sent.at(-1)?.displays).toEqual([{ id: "d1", area_id: "a1", url: "https://example.test/", load: 1, rect: null, visible: false }]);
    expect(sent.at(-1)?.retained).toHaveLength(inventory.length);
    act(() => useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 3, rest: { ...manual, browser_scopes: scopes } } }));
    flush();
    act(() => useShellStore.getState().applyFrame({ type: "delta", payload: { revision: 4, rest: { ...manual, browser_scopes: [] } } }));
    flush();
    expect(sent.at(-1)?.authorized_scopes).toEqual([]);
  });

  it("shows the existing badge only on the attached browser's own Workspace tab", () => {
    const display = { id: "d1", kind: "browser", path: "", label: "한글 브라우저 검증", url: "https://example.test/", state: "open", tab_id: null, preview: false } as ViewDisplaySnapshot;
    const interaction = { selected: true, areaActive: true, fit: "titled" as const, dragging: false, press() {}, select() {} };
    act(() => root.render(createElement(Fragment, null,
      createElement(BrowserHost, { actions }),
      createElement(TooltipProvider, null,
        createElement(DisplayTab, { display, workspace: local, interaction, actions }),
        createElement(DisplayTab, { display, workspace: other, interaction, actions }),
        createElement(DisplayTab, { display: { ...display, kind: "file" }, workspace: local, interaction, actions }),
      ))));
    // Mounting the host first initializes its inventory before the real event arrives.
    syncBrowserFront(local, layout, inventory);
    attach();
    const tabs = [...container.querySelectorAll('[role="tab"]')];
    expect(tabs.map((tab) => tab.querySelector('[data-browser-attached]') !== null)).toEqual([true, false, false]);
    expect(tabs[0]?.getAttribute("aria-label")).toContain("Agent attached");
    expect(tabs[1]?.getAttribute("aria-label")).not.toContain("Agent attached");
    attach(local, false);
    expect(container.querySelector('[data-browser-attached]')).toBeNull();
    expect(tabs[0]?.getAttribute("aria-label")).not.toContain("Agent attached");
  });

  it("unsubscribes and clears facts when the host leaves the app", () => {
    attach();
    act(() => root.render(null));
    expect(listeners.size).toBe(0);
    expect(useBrowserStore.getState().attached).toEqual({});
  });

  it("keeps the mount epoch across geometry and duplicate live notifications", () => {
    flush();
    const epoch = latestEpoch();
    const count = sent.length;
    syncBrowserFront(local, layout, inventory);
    flush();
    act(() => useShellStore.getState().setConnection("live"));
    flush();
    expect(sent).toHaveLength(count);
    syncBrowserFront(local, { root: { area: { id: "a2", active: "d1", displays: [{ id: "d1", kind: "browser", url: "https://example.test/", load: 1 }] } } } as unknown as ViewLayoutSnapshot,
      inventory.map((row) => row.path === local.path && row.device_id === local.device_id ? { ...row, area_id: "a2" } : row));
    flush();
    expect(latestEpoch()).toBe(epoch);
    expect(sent.at(-1)?.displays[0]?.area_id).toBe("a2");
    const slot = document.createElement("div");
    container.append(slot);
    let width = 320;
    const measure = vi.spyOn(slot, "getBoundingClientRect").mockImplementation(() => ({ x: 0, y: 0, left: 0, top: 0, width, height: 240, right: width, bottom: 240, toJSON: () => ({}) }));
    const unregister = registerBrowserSlot("d1", slot);
    try {
      flush();
      expect(sent.at(-1)?.displays[0]?.rect?.width).toBe(320);
      width = 480;
      syncBrowserFront(local, layout, inventory);
      flush();
      expect(sent.at(-1)?.displays[0]?.rect?.width).toBe(480);
      expect(latestEpoch()).toBe(epoch);
    } finally {
      unregister();
      measure.mockRestore();
      slot.remove();
    }
  });

  it("rehydrates actual retained attachment facts on reconnect with unchanged geometry", () => {
    replayAttachments = true;
    nativeAttached.add(hostKey(workspaceKey(local), "d1"));
    flush();
    const mountedEpoch = latestEpoch();
    expect(useBrowserStore.getState().attached).toEqual({ [hostKey(workspaceKey(local), "d1")]: true });
    const geometry = sent.at(-1)?.displays;
    act(() => useShellStore.getState().setConnection("reconnecting"));
    expect(useBrowserStore.getState().attached).toEqual({});
    act(() => useShellStore.getState().setConnection("gone"));
    const count = sent.length;
    flush();
    expect(sent).toHaveLength(count);
    act(() => useShellStore.getState().setConnection("live"));
    flush();
    const reconnectEpoch = latestEpoch();
    expect(reconnectEpoch).not.toBe(mountedEpoch);
    expect(sent.at(-1)?.displays).toEqual(geometry);
    expect(useBrowserStore.getState().attached).toEqual({ [hostKey(workspaceKey(local), "d1")]: true });
    act(() => useShellStore.getState().setConnection("live"));
    flush();
    expect(latestEpoch()).toBe(reconnectEpoch);
    act(() => useShellStore.getState().setConnection("gone"));
    nativeAttached.clear();
    act(() => useShellStore.getState().setConnection("live"));
    flush();
    expect(latestEpoch()).not.toBe(reconnectEpoch);
    expect(useBrowserStore.getState().attached).toEqual({});
  });

  it("creates a new epoch on remount and releases connection observation on unmount", () => {
    replayAttachments = true;
    nativeAttached.add(hostKey(workspaceKey(local), "d1"));
    flush();
    const mountedEpoch = latestEpoch();
    act(() => root.render(null));
    flush();
    expect(listeners.size).toBe(0);
    expect(useBrowserStore.getState().attached).toEqual({});
    const count = sent.length;
    act(() => {
      useShellStore.getState().setConnection("gone");
      useShellStore.getState().setConnection("live");
    });
    flush();
    expect(sent).toHaveLength(count);
    act(() => root.render(createElement(BrowserHost, { actions })));
    syncBrowserFront(local, layout, inventory);
    flush();
    expect(latestEpoch()).not.toBe(mountedEpoch);
    expect(listeners.size).toBe(1);
    expect(useBrowserStore.getState().attached).toEqual({ [hostKey(workspaceKey(local), "d1")]: true });
  });

  it("mounts with a fresh epoch while disconnected and renews it once on becoming live", () => {
    act(() => root.render(null));
    act(() => useShellStore.getState().setConnection("gone"));
    act(() => root.render(createElement(BrowserHost, { actions })));
    syncBrowserFront(local, layout, inventory);
    flush();
    const mountedEpoch = latestEpoch();
    act(() => useShellStore.getState().setConnection("live"));
    flush();
    expect(latestEpoch()).not.toBe(mountedEpoch);
  });
});
