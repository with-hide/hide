// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { DeviceRail } from "./components/device-rail";
import { TooltipProvider } from "./components/ui/tooltip";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { document.body.innerHTML = ""; });

const KEY = "ab".repeat(32);

/** This Mac and the paired Mac-mini it opens, with what Mac-mini told of its own work. */
function world(told: { needs_you: number; done: number } | null): SnapshotRest {
  return {
    navigator: {
      focused_device_id: "local",
      devices: [
        { id: "local", label: "This Mac", kind: "local", state: "local", message: null },
        { id: "mini", label: "Mac-mini", kind: "remote", state: "ready", message: null, peer_key: KEY },
      ],
      peers: [{ key: KEY, device_id: "mini", name: "Mac-mini", opens: "active", opened_by: "active", connected: true, path: null, reason: null, update: null }],
      peer_work: told ? [{ device_id: "mini", key: KEY, rows: [], ...told }] : [],
    },
    status: { remote: [] },
  } as unknown as SnapshotRest;
}

// PRD device-connect-iroh D-50: a paired machine's tile carries the Needs You and Done that machine told of its own work.
it("draws on a paired machine's tile the Needs You count that machine told, and nothing while it told none", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState();
  try {
    await act(async () => useShellStore.setState({ rest: world({ needs_you: 2, done: 1 }), connection: "live" }));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><DeviceRail actions={createActions(() => true)} /></TooltipProvider>));
    const tile = () => document.querySelector<HTMLElement>('[data-rail-tile="mini"]');
    expect(tile()?.querySelector('[data-rail-badge="needs_you"]')?.textContent).toBe("2");
    await act(async () => useShellStore.setState({ rest: world(null) }));
    expect(tile()?.querySelector("[data-rail-badge]")).toBeNull();
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
  }
});
