// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { UpdatePhase, UpdateStatus } from "./host";
import type { Peer } from "./pairing";
import { RemoteStage } from "./RemoteStage";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { document.body.innerHTML = ""; });

const peer = (over: Partial<Peer> = {}): Peer => ({ key: "ab".repeat(32), device_id: "mini", name: "Mac-mini", opens: "active", opened_by: "active", connected: false, path: null, reason: null, update: null, ...over });
const status = (phase: UpdatePhase): UpdateStatus => ({ phase, prerelease: false, installOnQuit: true, dismissed: null, required: false });
const update = () => document.querySelector<HTMLButtonElement>("[data-remote-update]");
const settings = () => document.querySelector<HTMLButtonElement>("[data-remote-settings]");

// One test, in order: the page subscribes to the update bridge once for its life (`updates.ts`).
it("offers Update now on a paired machine's screen while this computer must update, asked or not, and only where this app can update (B22)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const show = (row: Peer) => act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [row] } } } as never));
  try {
    let deliver: (next: UpdateStatus) => void = () => undefined;
    const bridge = { onStatus: vi.fn((listener: (next: UpdateStatus) => void) => { deliver = listener; listener(status({ kind: "idle", checkedAt: null })); return () => undefined; }), check: vi.fn(), restart: vi.fn(), setPrerelease: vi.fn(), dismiss: vi.fn() };
    window.hideHost = { kind: "electron", platform: "darwin", updates: bridge, onWindowChrome: () => () => undefined } as never;
    await show(peer({ update: "here_asked" }));
    await act(async () => useUiStore.setState({ remoteScreen: "mini" }));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><RemoteStage actions={createActions(() => true)} /></TooltipProvider>));

    // Asked to update, this computer is still the one that must: the screen says so and a press looks for the release.
    expect(document.querySelector("[data-remote-stage]")?.getAttribute("data-remote-state")).toBe("update_here");
    expect(settings()).toBeNull();
    await act(async () => update()!.click());
    expect(bridge.check).toHaveBeenCalledTimes(1);

    // A build that cannot replace itself offers no update, only the device settings.
    await act(async () => deliver(status({ kind: "off", reason: "development" })));
    expect(update()).toBeNull();
    expect(settings()).not.toBeNull();

    // The other computer must update: nothing to update here.
    await act(async () => deliver(status({ kind: "idle", checkedAt: null })));
    await show(peer({ update: "there" }));
    expect(update()).toBeNull();
    expect(settings()).not.toBeNull();
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    delete (window as { hideHost?: unknown }).hideHost;
    vi.unstubAllGlobals();
  }
});
