// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import type { UpdatePhase, UpdateStatus } from "./host";
import type { PairingView, Peer } from "./pairing";
import { PeerUpdateNotice } from "./PeerUpdateNotice";
import { useShellStore } from "./store";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
afterEach(() => { document.body.innerHTML = ""; });

const peer = (over: Partial<Peer> = {}): Peer => ({ key: "ab".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active", connected: false, path: null, reason: null, update: null, ...over });
const idle: PairingView = { offer: null, asks: [], join: null, requests: [] };
const status = (phase: UpdatePhase, more: Partial<UpdateStatus> = {}): UpdateStatus => ({ phase, prerelease: false, installOnQuit: true, dismissed: null, required: false, ...more });
const notice = () => document.querySelector<HTMLElement>("[data-peer-update-notice]");
const now = () => document.querySelector<HTMLButtonElement>("[data-peer-update-now]");

const sent: { kind: string; payload: unknown }[] = [];
const actions = createActions((event) => { sent.push(event as never); return true; });
const Screen = () => <TooltipProvider><PeerUpdateNotice actions={actions} /></TooltipProvider>;

// One test, in order: the page subscribes to the bridge once for its life (`updates.ts`).
it("says on the computer that must update which computer waits, updates on a press, and by itself when that computer asked (B22)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState();
  // `connect`: whether the build offers connections between two Hides, as hided's hello says.
  const show = (pairing: PairingView | null, peers: Peer[], connect = true) => act(async () => useShellStore.setState({ daemon: { device_connect: connect }, pairing, rest: { navigator: { devices: [], peers, guest_panes: [] }, ui_state: {} } } as never));
  try {
    // A browser tab or the phone updates nothing: no notice.
    await show(idle, [peer({ update: "here" })]);
    const tab = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => tab.render(<Screen />));
    expect(notice()).toBeNull();
    await act(async () => tab.unmount());

    let deliver: (next: UpdateStatus) => void = () => undefined;
    const bridge = { onStatus: vi.fn((listener: (next: UpdateStatus) => void) => { deliver = listener; listener(status({ kind: "idle", checkedAt: null })); return () => undefined; }), check: vi.fn(), restart: vi.fn(), setPrerelease: vi.fn(), dismiss: vi.fn() };
    window.hideHost = { kind: "electron", platform: "darwin", updates: bridge } as never;
    const app = createRoot(document.body.appendChild(document.createElement("div")));
    await act(async () => app.render(<Screen />));
    const phase = (next: UpdateStatus) => act(async () => deliver(next));

    // This computer is the older one and nobody asked: the notice, and a press looks for the release.
    expect(notice()?.textContent).toContain("Update Hide to connect with MacBook");
    // A button that stands out without the full lime (operator review 2026-11-22).
    expect(now()?.dataset.variant).toBe("tint");
    expect(bridge.check).not.toHaveBeenCalled();
    await act(async () => now()!.click());
    expect(bridge.check).toHaveBeenCalledTimes(1);
    await phase(status({ kind: "downloading", version: "1.3.0", percent: 10 }));
    expect(now()).toBeNull();
    expect(notice()?.textContent).toContain("Getting the update");
    await phase(status({ kind: "ready", version: "1.3.0" }));
    expect(bridge.restart).not.toHaveBeenCalled();
    await act(async () => now()!.click());
    expect(bridge.restart).toHaveBeenCalledTimes(1);

    // Another computer's view of this one offers nothing.
    await show(null, [peer({ update: "here" })]);
    expect(notice()).toBeNull();
    await show(null, [peer({ update: "there" })]);
    expect(notice()).toBeNull();

    // The computer that waits: the other one must update, and a press asks it to.
    await show(idle, [peer({ update: "there" })]);
    expect(notice()?.getAttribute("data-peer-update-side")).toBe("there");
    expect(notice()?.textContent).toContain("MacBook needs an update before it can be opened");
    expect(document.querySelector<HTMLElement>("[data-peer-update-ask]")?.dataset.variant).toBe("secondary");
    await act(async () => document.querySelector<HTMLButtonElement>("[data-peer-update-ask]")!.click());
    expect(sent).toEqual([expect.objectContaining({ kind: "pairing_update", payload: { device: "ab".repeat(32) } })]);
    // Once asked, the row says so and the notice goes; a closed notice stays closed for that computer.
    await show(idle, [peer({ update: "there_asked" })]);
    expect(notice()).toBeNull();
    await show(idle, [peer({ update: "there" })]);
    await act(async () => document.querySelector<HTMLButtonElement>("[data-peer-update-notice] [aria-label]")!.click());
    expect(notice()).toBeNull();

    // A build that does not offer connections between two Hides shows none of it and follows no ask.
    await phase(status({ kind: "idle", checkedAt: 1 }));
    await show(idle, [peer({ update: "here_asked" })], false);
    expect(notice()).toBeNull();
    expect(bridge.check).toHaveBeenCalledTimes(1);

    // The computer that opens this one asked: the app looks at once and restarts once the release is ready.
    await show(idle, [peer({ update: "here_asked" })]);
    expect(bridge.check).toHaveBeenCalledTimes(2);
    expect(bridge.restart).toHaveBeenCalledTimes(1);
    await phase(status({ kind: "ready", version: "1.3.0" }));
    expect(bridge.restart).toHaveBeenCalledTimes(2);

    // Where the restart asks for the system password it waits for the operator, and the notice stays.
    await show(idle, []);
    await phase(status({ kind: "ready", version: "1.3.0" }, { installOnQuit: false }));
    await show(idle, [peer({ update: "here_asked" })]);
    expect(bridge.restart).toHaveBeenCalledTimes(2);
    expect(now()).not.toBeNull();

    // A development build takes no update: the line, with nothing to press.
    await phase(status({ kind: "off", reason: "development" }));
    expect(notice()).not.toBeNull();
    expect(now()).toBeNull();
    await act(async () => app.unmount());
  } finally {
    delete window.hideHost;
    useShellStore.setState(saved, true);
  }
});
