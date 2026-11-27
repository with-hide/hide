// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Peer } from "./pairing";
import { RemoteStage, SLOW_OPENING_MS } from "./RemoteStage";
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

// The screen draws with this window's View menu choices (UI_BEHAVIOR.md,
// Device rail, Another computer's screen; operator decision 2026-11-25): the
// show carries them, and a choice changed while the screen shows reaches it
// at once, without closing the screen (which would close its connection).
it("carries this window's display choices with the show, and a changed choice to the open screen without closing it", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  const shows: unknown[] = [];
  const choose = (agent_logo_mode: "shown" | "hidden") => act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [peer({ connected: true })] }, ui_state: { agent_logo_mode } } } as never));
  try {
    window.hideHost = { kind: "electron", platform: "darwin", onWindowChrome: () => () => undefined, remoteScreens: { show: (view: unknown) => shows.push(view), onState: () => () => undefined } } as never;
    await choose("hidden");
    await act(async () => useUiStore.setState({ remoteScreen: "mini" }));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><RemoteStage actions={createActions(() => true)} /></TooltipProvider>));
    const displays = () => shows.map((view) => (view as { display?: unknown } | null)?.display ?? null);
    expect(displays()).toEqual([{ agent_logo_mode: "hidden", pane_turn_row_mode: "one_line" }]);

    await choose("shown");
    expect(displays()).toEqual([{ agent_logo_mode: "hidden", pane_turn_row_mode: "one_line" }, { agent_logo_mode: "shown", pane_turn_row_mode: "one_line" }]);
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    delete (window as { hideHost?: unknown }).hideHost;
    vi.unstubAllGlobals();
  }
});

// While another computer's screen opens, the stage draws a loading ring, that
// computer's name and `화면을 여는 중…` (UI_BEHAVIOR.md, Device rail, Another
// computer's screen; operator decision 2026-11-27, mission-107 option A).
// Past SLOW_OPENING_MS one more line names the relay, and only on a relayed
// connection; the loading goes the moment the screen shows.
it("draws a ring and the computer's name while its screen opens, names the relay only past 5 s on a relayed connection, and goes once the screen shows", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.useFakeTimers();
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  let report: ((state: { device: string; state: "opening" | "shown" | "failed"; reason?: string }) => void) | null = null;
  const show = (row: Peer) => act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [row] } } } as never));
  const stage = () => document.querySelector("[data-remote-stage]");
  const ring = () => stage()?.querySelector("svg.lucide-loader-circle") ?? null;
  const slow = () => stage()?.querySelector("[data-remote-slow]") ?? null;
  const wait = (ms: number) => act(async () => { vi.advanceTimersByTime(ms); });
  try {
    window.hideHost = { kind: "electron", platform: "darwin", onWindowChrome: () => () => undefined, remoteScreens: { show: () => undefined, onState: (listener: typeof report) => { report = listener; return () => { report = null; }; } } } as never;
    await show(peer({ connected: true, path: "relay" }));
    await act(async () => useUiStore.setState({ remoteScreen: "mini" }));
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><RemoteStage actions={createActions(() => true)} /></TooltipProvider>));

    // Opening: the ring, which turns only where motion is allowed, the name and the line.
    expect(stage()?.getAttribute("data-remote-state")).toBe("opening");
    expect(ring()?.getAttribute("class")).toContain("motion-safe:animate-spin");
    expect(ring()?.getAttribute("class")).not.toMatch(/(^|\s)animate-spin/);
    expect(stage()?.textContent).toContain("Mac-mini");
    expect(stage()?.textContent).toContain("Opening its screen…");

    // A relayed screen says nothing more before SLOW_OPENING_MS, and names the relay after it.
    await wait(SLOW_OPENING_MS - 1);
    expect(slow()).toBeNull();
    await wait(1);
    expect(slow()?.textContent).toBe("It connects through the relay, so this can take a little longer");

    // The moment the screen shows, the stage draws nothing over it.
    await act(async () => report?.({ device: "mini", state: "shown" }));
    expect(ring()).toBeNull();
    expect(slow()).toBeNull();
    expect(stage()?.textContent).toBe("");

    // A direct, Tailscale or local connection never names the relay, however long it takes.
    await act(async () => report?.({ device: "mini", state: "opening" }));
    for (const path of ["direct", "tailscale", "local", null] as const) {
      await show(peer({ connected: true, path }));
      await wait(SLOW_OPENING_MS * 3);
      expect(ring()).not.toBeNull();
      expect(slow()).toBeNull();
    }

    // A state that cannot show keeps its own words, with no ring.
    await show(peer({ connected: false, path: "relay" }));
    await wait(SLOW_OPENING_MS * 2);
    expect(stage()?.getAttribute("data-remote-state")).toBe("not_connected");
    expect(ring()).toBeNull();
    expect(slow()).toBeNull();
    expect(stage()?.textContent).toBe("Mac-miniNot connectedOpens on its own once it is reached");

    // Coming back to opening counts the 5 s again from zero.
    await show(peer({ connected: true, path: "relay" }));
    await wait(SLOW_OPENING_MS - 1);
    expect(ring()).not.toBeNull();
    expect(slow()).toBeNull();
    await wait(1);
    expect(slow()).not.toBeNull();

    // Another relayed computer chosen while this one opens counts its own 5 s from zero.
    await act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [peer({ connected: true, path: "relay" }), peer({ key: "cd".repeat(32), device_id: "studio", name: "Studio", connected: true, path: "relay" })] } } } as never));
    await act(async () => useUiStore.setState({ remoteScreen: "studio" }));
    expect(stage()?.textContent).toContain("Studio");
    expect(ring()).not.toBeNull();
    expect(slow()).toBeNull();
    await wait(SLOW_OPENING_MS - 1);
    expect(slow()).toBeNull();
    await wait(1);
    expect(slow()).not.toBeNull();
    // The relay line sits in a status region that stood from the start, so it is read out when it appears.
    expect(slow()?.closest("[role=status]")).not.toBeNull();
    await act(async () => root.unmount());
  } finally {
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    delete (window as { hideHost?: unknown }).hideHost;
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});

// A machine no longer paired leaves no screen to come back to: the window shows its own again and says so after the
// change, so a look chosen on this machine counts again (docs/status-model.md, Hide owns the read axis).
it("shows this window's own screen again, and says so, when the machine on it is no longer paired", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const sent: { kind: string; payload: unknown }[] = [];
  const actions = createActions((event) => { sent.push(event as never); return true; });
  try {
    window.hideHost = { kind: "electron", platform: "darwin", onWindowChrome: () => () => undefined } as never;
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: { navigator: { devices: [], peers: [peer()] } } } as never);
      useUiStore.setState({ remoteScreen: "mini" });
    });
    actions.noteWindowScreen();
    const root = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => root.render(<TooltipProvider><RemoteStage actions={actions} /></TooltipProvider>));
    await act(async () => useShellStore.setState({ rest: { navigator: { devices: [], peers: [] } } } as never));
    expect(useUiStore.getState().remoteScreen).toBeNull();
    expect(sent.filter((event) => event.kind === "window_screen").map((event) => event.payload)).toEqual([{ screen: "computer" }, { screen: "own" }]);
    await act(async () => root.unmount());
  } finally {
    window.hideHost = undefined;
    useUiStore.setState({ remoteScreen: null });
  }
});
