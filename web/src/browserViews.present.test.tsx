// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { BrowserHost, PresentedPage } from "./BrowserDisplay";
import { presentBrowserPage, registerBrowserSlot } from "./browserViews";
import { TooltipProvider } from "./components/ui/tooltip";
import type { BrowserHostEvent, BrowserSync, HostBridge } from "./host";
import type { SnapshotRest, ViewLayoutSnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { workspaceKey } from "./viewLayout";

// Desk's tab panel shows a session's page as the live page (PRD
// session-docs-and-assistant D-40, docs/BROWSER_DISPLAYS.md "The page and its
// limits"): the host is told to place that page, named in its own checkout,
// over Desk's slot, so the one native page moves rather than loading twice.
// Only the platform bridge and the frame clock are fakes.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});

const frames: FrameRequestCallback[] = [];
const sent: BrowserSync[] = [];
const listeners = new Set<(event: BrowserHostEvent) => void>();
const bridge = {
  kind: "electron",
  platform: "darwin",
  browser: {
    sync: (state: BrowserSync) => { sent.push(state); },
    capture: async () => null,
    endCycle() {},
    command() {},
    onEvent: (listener: (event: BrowserHostEvent) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  },
} as unknown as HostBridge;
const events: { kind: string; payload: Record<string, unknown> }[] = [];
const actions = createActions((event) => { events.push(event as (typeof events)[number]); return true; });

// The checkout in front shows a file, `d0`, with a page `d1` behind it; another checkout, where a session opened
// its page, holds a `d1` of its own and a second page, `d2`.
const front = { device_id: "local", path: "/fixture/front" };
const session = { device_id: "local", path: "/fixture/session" };
const frontLayout = {
  active_area: "a1",
  root: { area: { id: "a1", active: "d0", displays: [{ id: "d0", kind: "file", path: "/fixture/front/a.md" }, { id: "d1", kind: "browser", url: "https://front.test/", load: 1 }] } },
} as unknown as ViewLayoutSnapshot;
const inventory = [
  { ...front, view_id: "d1", area_id: "a1", url: "https://front.test/", load: 1 },
  { ...session, view_id: "d1", area_id: "a9", url: "https://github.com/with-hide/hide/pull/1", load: 2 },
  { ...session, view_id: "d2", area_id: "a9", url: "file:///fixture/session/report.html", load: 3 },
];
const deskRect = { x: 600, y: 80, width: 520, height: 700 };

let root: Root;
let container: HTMLDivElement;

function flush(): void {
  act(() => { for (const callback of frames.splice(0)) callback(0); });
}

function last(): BrowserSync {
  const sync = sent.at(-1);
  if (!sync) throw new Error("no sync sent");
  return sync;
}

/** The window with Desk's tab panel showing the session's page, or showing nothing. */
function render(desk: boolean, workspace = session): void {
  const url = inventory.find((row) => row.path === workspace.path && row.view_id === "d1")!.url;
  const page = desk ? createElement(PresentedPage, { workspace, id: "d1", url, label: "PR 1", actions }) : null;
  act(() => root.render(createElement(TooltipProvider, null, createElement(BrowserHost, { actions }), page)));
}

function showInDesk(): void {
  render(true);
}

function emit(event: BrowserHostEvent): void {
  act(() => { for (const listener of listeners) listener(event); });
}

beforeAll(() => {
  window.hideHost = bridge;
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const rect = this.hasAttribute("data-browser-slot") || this.hasAttribute("data-test-slot") ? deskRect : { x: 0, y: 0, width: 0, height: 0 };
    return { ...rect, left: rect.x, top: rect.y, right: rect.x + rect.width, bottom: rect.y + rect.height, toJSON: () => ({}) } as DOMRect;
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

beforeEach(() => {
  act(() => root.render(null));
  const rest = { workspace_view: { ...front, layout: frontLayout }, browser_views: inventory, browser_scopes: [] } as unknown as SnapshotRest;
  useShellStore.setState({ connection: "live", rest, revision: 1 });
  render(false);
  flush();
  sent.length = 0;
  events.length = 0;
});

afterAll(() => {
  act(() => root.unmount());
  container.remove();
  delete window.hideHost;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  browserCanvas.restore();
});

describe("a page Desk's tab panel shows", () => {
  it("is placed over Desk's slot in its own checkout, alone, and goes back behind its checkout when Desk lets go, telling the core nothing", () => {
    showInDesk();
    flush();
    expect(last().workspace).toBe(workspaceKey(session));
    // Its checkout's other page is not placed, so it neither shows behind Desk's slot nor loads for it.
    expect(last().displays).toEqual([{ id: "d1", area_id: "a9", url: inventory[1]!.url, load: 2, rect: deskRect, visible: true }]);
    // Every page stays in the inventory, so none is closed by the move.
    expect(last().retained.map((row) => [row.workspace, row.id])).toEqual([[workspaceKey(front), "d1"], [workspaceKey(session), "d1"], [workspaceKey(session), "d2"]]);
    // The operator pressing into the page moves no View area of its checkout.
    emit({ kind: "focus", workspace: workspaceKey(session), id: "d1" });

    render(false);
    flush();
    expect(last().workspace).toBe(workspaceKey(front));
    expect(last().displays).toEqual([{ id: "d1", area_id: "a1", url: "https://front.test/", load: 1, rect: null, visible: false }]);
    // The move is the window's alone: the core's focus, visible tabs, attached tabs and selection see no event.
    expect(events).toEqual([]);
  });

  it("places nothing once its checkout no longer keeps it, never the front checkout's page of the same id", () => {
    const slot = document.createElement("div");
    slot.setAttribute("data-test-slot", "");
    document.body.append(slot);
    const unregister = registerBrowserSlot("d1", slot);
    flush();
    expect(last().displays[0]?.rect).toEqual(deskRect);

    const release = presentBrowserPage({ workspace: workspaceKey({ ...session, path: "/fixture/closed" }), id: "d1" });
    flush();
    expect(last().displays).toEqual([]);

    release();
    unregister();
    slot.remove();
  });

  it("taking the keyboard in Desk leaves the View areas of its checkout, the one in front, as they are", () => {
    render(true, front);
    flush();
    expect(last().displays).toEqual([{ id: "d1", area_id: "a1", url: "https://front.test/", load: 1, rect: deskRect, visible: true }]);
    emit({ kind: "focus", workspace: workspaceKey(front), id: "d1" });
    render(false);
    flush();
    expect(events).toEqual([]);
  });

  it("sends a typed address to the page in its own checkout", () => {
    showInDesk();
    flush();
    const field = container.querySelector<HTMLInputElement>("[data-browser-address]")!;
    act(() => {
      field.focus();
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      set.call(field, "example.com/next");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => { field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(events.filter((event) => event.kind === "view_layout").map((event) => event.payload)).toEqual([
      { workspace: session, action: "navigate", display_id: "d1", url: "https://example.com/next" },
    ]);
  });
});
