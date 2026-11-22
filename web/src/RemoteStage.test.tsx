// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Peer } from "./pairing";
import { RemoteStage } from "./RemoteStage";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { document.body.innerHTML = ""; });

const peer = (over: Partial<Peer> = {}): Peer => ({ key: "ab".repeat(32), device_id: "mini", name: "Mac-mini", opens: "active", opened_by: "active", connected: false, path: null, reason: null, update: null, ...over });
const settings = () => document.querySelector<HTMLButtonElement>("[data-remote-settings]");

it("says which computer must update on a paired machine's screen and leaves the press to the window's notice (B22)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const show = (row: Peer) => act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [row] } } } as never));
  const said = () => document.querySelector("[data-remote-stage]")?.textContent ?? "";
  try {
    window.hideHost = { kind: "electron", platform: "darwin", onWindowChrome: () => () => undefined } as never;
    await show(peer({ update: "here_asked" }));
    await act(async () => useUiStore.setState({ remoteScreen: "mini" }));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><RemoteStage actions={createActions(() => true)} /></TooltipProvider>));

    // Asked to update, this computer is still the one that must: the screen says so, with nothing to press on it.
    expect(document.querySelector("[data-remote-stage]")?.getAttribute("data-remote-state")).toBe("update_here");
    expect(said()).toContain("Update needed on this computer");
    expect(document.querySelector("[data-remote-stage] button")).toBeNull();

    // The other computer must update: the screen says that, again with nothing to press.
    await show(peer({ update: "there" }));
    expect(said()).toContain("Update needed on that computer");
    expect(document.querySelector("[data-remote-stage] button")).toBeNull();

    // Any other state that cannot show still offers the device settings.
    await show(peer({ opens: "paused" }));
    expect(settings()).not.toBeNull();
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    delete (window as { hideHost?: unknown }).hideHost;
    vi.unstubAllGlobals();
  }
});
