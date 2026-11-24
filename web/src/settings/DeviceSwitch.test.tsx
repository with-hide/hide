import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "../components/ui/tooltip";
import { NOTHING_UNDER_WAY, type PairingView } from "../pairing";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { DeviceMovedStrip, DeviceMoveWatch } from "./DeviceSwitch";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const LOCAL: Device = { agent_scope: emptyScope(), id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
/** A device this computer reaches over SSH. */
const helper = (over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id: "mini", label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 0, test: null, ...over });
/** The same device once it moved: its machine's own Hide, under the id it had. */
const moved = (over: Partial<Device> = {}): Device => helper({ ssh_alias: null, peer_key: "ab".repeat(32), ...over });

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(device: Device, over: { front?: string; pairing?: PairingView | null } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const rest = (next: Device) => ({
    navigator: { focused_device_id: over.front ?? next.id, devices: [LOCAL, next], workspaces: [] },
    status: { remote: [] },
    ui_state: {},
  });
  // `place` is where the window draws the strip: the main screen, or the
  // remote screen once a machine's own Hide shows (App, RemoteStage). The
  // watch is the window's, mounted once whichever screen shows.
  const draw = (place: string) => root.render(<TooltipProvider><DeviceMoveWatch /><div key={place}><DeviceMovedStrip /></div></TooltipProvider>);
  await act(async () => {
    useShellStore.setState({ connection: "live", pairing: over.pairing === undefined ? NOTHING_UNDER_WAY : over.pairing, pairingResult: null, rest: rest(device) } as never);
    draw("main");
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const show = async (next: Device) => { await act(async () => { useShellStore.setState({ rest: rest(next) } as never); }); };
  const moveStrip = async (place: string) => { await act(async () => draw(place)); };
  return { q, show, moveStrip, unmount: async () => { await act(async () => root.unmount()); useUiStore.setState({ switchedDevices: [] }); useShellStore.setState(saved, true); } };
}

it("says once over a machine's screen what starts fresh there after it moved while this window watched (D-48)", async () => {
  const { q, show, unmount } = await mount(helper());
  expect(q("[data-device-switched]")).toBeNull();
  await show(moved());
  expect(q('[data-device-switched="mini"]')?.textContent).toBe("Memory and standing instructions start fresh on this machine");
  // Once: leaving the machine's screen ends it, and coming back says nothing.
  await act(async () => { useShellStore.setState({ rest: { navigator: { focused_device_id: "local", devices: [LOCAL, moved()], workspaces: [] }, status: { remote: [] }, ui_state: {} } } as never); });
  await show(moved());
  expect(q("[data-device-switched]")).toBeNull();
  expect(useUiStore.getState().switchedDevices).toEqual([]);
  await unmount();
  // A machine that moved before this window watched it says nothing.
  const later = await mount(moved());
  expect(later.q("[data-device-switched]")).toBeNull();
  await later.unmount();
});

it("says nothing over a device that was taken off and added again as its machine's own Hide", async () => {
  const { q, show, unmount } = await mount(helper());
  await show(helper({ id: "other", ssh_alias: "other" }));
  await show(moved());
  expect(q("[data-device-switched]")).toBeNull();
  await unmount();
});

it("keeps a moved machine's line while the strip moves from the main screen to the machine's own screen (D-48)", async () => {
  const { q, show, moveStrip, unmount } = await mount(helper());
  await show(moved());
  // The move landing brings the machine's own screen, which draws the strip anew.
  await moveStrip("remote");
  expect(q('[data-device-switched="mini"]')?.textContent).toBe("Memory and standing instructions start fresh on this machine");
  await unmount();
});

it("says a moved machine's line only over that machine's screen, and ends it when another computer's screen opens (D-48)", async () => {
  const { q, show, moveStrip, unmount } = await mount(helper());
  await show(moved());
  await act(async () => { useUiStore.setState({ remoteScreen: "mini" }); });
  await moveStrip("remote");
  expect(q('[data-device-switched="mini"]')).not.toBeNull();
  // Another paired computer's screen opens from its tile; the core's focus stays on the machine.
  await act(async () => { useUiStore.setState({ remoteScreen: "other" }); });
  expect(q("[data-device-switched]")).toBeNull();
  expect(useUiStore.getState().switchedDevices).toEqual([]);
  await act(async () => { useUiStore.setState({ remoteScreen: null }); });
  await unmount();
});

it("says once how many terminals a move could not take over (B36)", async () => {
  const { q, show, unmount } = await mount(helper());
  await show(moved({ not_carried: 2 }));
  expect(q('[data-device-switched="mini"] [data-device-not-carried="2"]')?.textContent).toContain("2 terminals could not be taken over · AI conversations can be resumed");
  await unmount();
});

it("draws no moved line on a screen that is not this computer's own window (B52)", async () => {
  const { q, show, unmount } = await mount(helper(), { pairing: null });
  await show(moved());
  expect(q("[data-device-switched]")).toBeNull();
  await unmount();
});
