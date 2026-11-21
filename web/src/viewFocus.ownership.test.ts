import { afterEach, expect, it, vi } from "vitest";
import type { BrowserHostEvent } from "./host";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { installKeyboardOwner, keyboardCommandOwner, keyboardOwner, noteCommandDelivered } from "./viewFocus";

const native = vi.hoisted(() => ({ listener: null as ((event: BrowserHostEvent) => void) | null }));
vi.mock("./host", () => ({ browserBridge: () => ({ onEvent: (listener: (event: BrowserHostEvent) => void) => { native.listener = listener; return () => { native.listener = null; }; } }) }));
afterEach(() => vi.unstubAllGlobals());

it("consecutive commands use the page during handback and the terminal after delivery", () => {
  const events = new Map<string, (event: unknown) => void>();
  class ElementBoundary {
    closest(selector: string) {
      if (selector === "[data-workspace-screen]") return { dataset: { workspaceScreen: "c" } };
      if (selector === "[data-pane-view]") return { dataset: { paneView: "p" } };
      return null;
    }
  }
  const terminal = new ElementBoundary();
  vi.stubGlobal("Element", ElementBoundary);
  vi.stubGlobal("document", { activeElement: terminal, body: {} });
  vi.stubGlobal("window", {
    addEventListener: (type: string, listener: (event: unknown) => void) => events.set(type, listener),
    removeEventListener: (type: string) => events.delete(type),
  });
  useShellStore.setState({ rest: {
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", workspaces: [{ id: "w", checkouts: [{ id: "c", path: "/fixture" }] }] },
    workspace_view: { device_id: "local", path: "/fixture", open: true, tray: { tab_id: "t1", shown: true, count: 0 }, tool: null, layout: { root: { area: { id: "a1", active: "b", displays: [{ id: "b" }] } } } },
  } as unknown as SnapshotRest });
  const remove = installKeyboardOwner();
  try {
    const focusTerminal = () => events.get("focusin")!({ type: "focusin", target: terminal });
    focusTerminal();
    native.listener!({ kind: "focus", workspace: "local\u0000/fixture", id: "b" });
    focusTerminal();
    expect(keyboardCommandOwner()).toEqual({ kind: "view", workspace: "c", areaId: "a1" });
    noteCommandDelivered();
    expect(keyboardOwner()).toEqual({ kind: "pane", workspace: "c", paneId: "p" });
    expect(keyboardCommandOwner()).toEqual(keyboardOwner());
  } finally { remove(); }
});

it("a page pressed on focus mode's desk takes the keyboard though the lane itself is closed, and one under a pane drawn alone does not (D-53, D-57)", () => {
  const events = new Map<string, (event: unknown) => void>();
  class ElementBoundary {
    closest(selector: string) {
      if (selector === "[data-workspace-screen]") return { dataset: { workspaceScreen: "c" } };
      if (selector === "[data-pane-view]") return { dataset: { paneView: "p" } };
      return null;
    }
  }
  const terminal = new ElementBoundary();
  vi.stubGlobal("Element", ElementBoundary);
  vi.stubGlobal("document", { activeElement: terminal, body: {} });
  vi.stubGlobal("window", {
    addEventListener: (type: string, listener: (event: unknown) => void) => events.set(type, listener),
    removeEventListener: (type: string) => events.delete(type),
  });
  // Entered by ⌘. from a closed lane: the desk stands beside the pane while the lane's own state stays closed.
  const rest = (alone: boolean) => ({
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", workspaces: [{ id: "w", checkouts: [{ id: "c", path: "/fixture" }] }] },
    workspace_view: { device_id: "local", path: "/fixture", open: false, tray: { tab_id: "t1", shown: false, count: 1, pair: { tab_id: "t1", pane_id: "p", alone, shown: 1 } }, tool: null, layout: { root: { area: { id: "a1", active: "b", displays: [{ id: "b" }] } } } },
  }) as unknown as SnapshotRest;
  const remove = installKeyboardOwner();
  try {
    events.get("focusin")!({ type: "focusin", target: terminal });
    useShellStore.setState({ rest: rest(true) });
    native.listener!({ kind: "focus", workspace: "local\u0000/fixture", id: "b" });
    expect(keyboardOwner(), "no desk on screen beside a pane drawn alone").toEqual({ kind: "pane", workspace: "c", paneId: "p" });
    useShellStore.setState({ rest: rest(false) });
    native.listener!({ kind: "focus", workspace: "local\u0000/fixture", id: "b" });
    expect(keyboardOwner()).toEqual({ kind: "view", workspace: "c", areaId: "a1" });
    expect(keyboardCommandOwner()).toEqual({ kind: "view", workspace: "c", areaId: "a1" });
  } finally { remove(); }
});
