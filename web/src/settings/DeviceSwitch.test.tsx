import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { NOTHING_UNDER_WAY, type PairingView } from "../pairing";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { AUTO_LOGIN_DOCS, DeviceMovedStrip, DeviceMoveWatch, DeviceNeedsLine } from "./DeviceSwitch";

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

async function mount(device: Device, over: { front?: string; pairing?: PairingView | null; moved?: boolean } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const actions = createActions(() => true);
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
  const draw = (place: string) => root.render(<TooltipProvider><DeviceMoveWatch /><div key={place}><DeviceMovedStrip /></div><DeviceNeedsLine deviceId={device.id} actions={actions} moved={over.moved} /></TooltipProvider>);
  await act(async () => {
    useShellStore.setState({ connection: "live", pairing: over.pairing === undefined ? NOTHING_UNDER_WAY : over.pairing, pairingResult: null, rest: rest(device) } as never);
    draw("main");
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const show = async (next: Device) => { await act(async () => { useShellStore.setState({ rest: rest(next) } as never); }); };
  const moveStrip = async (place: string) => { await act(async () => draw(place)); };
  return { q, show, moveStrip, unmount: async () => { await act(async () => root.unmount()); useUiStore.setState({ switchedDevices: [] }); useShellStore.setState(saved, true); } };
}

it("says faintly what a Mac reached over SSH needs to work without this computer, with where to turn it on (B30)", async () => {
  const { q, show, unmount } = await mount(helper({ needs: { kind: "auto_login" } }));
  const line = q('[data-device-needs="mini:auto_login"]')!;
  expect(line.textContent).toBe("Turn on automatic login there, and it can work on its own without this computer · How");
  expect(q('[data-device-needs-docs="mini"]')?.getAttribute("href")).toBe(AUTO_LOGIN_DOCS);
  // The setting is on at the next connection: the core clears the line.
  await show(helper());
  expect(q("[data-device-needs]")).toBeNull();
  await unmount();
});

it("gives a Linux machine reached over SSH the one command to run there, with Copy (B30)", async () => {
  const command = "sudo loginctl enable-linger hy";
  const { q, unmount } = await mount(helper({ id: "box", label: "build-box", ssh_alias: "box", needs: { kind: "linger", command } }));
  const line = q('[data-device-needs="box:linger"]')!;
  expect(line.textContent).toBe("Run this there once, and it can work on its own without this computer" + command);
  expect(line.querySelector("button")?.getAttribute("aria-label")).toBe("Copy");
  await unmount();
});

it("says nothing under a device whose machine needs nothing", async () => {
  const { q, unmount } = await mount(helper());
  expect(q("[data-device-needs]")).toBeNull();
  await unmount();
});

it("warns under a machine that moved what the Hide started there still needs", async () => {
  const mac = await mount(moved({ needs: { kind: "auto_login" } }), { moved: true });
  expect(mac.q('[data-device-needs="mini:auto_login"]')?.textContent).toBe("Turn on automatic login there to stay connected after a restart");
  expect(mac.q("[data-device-needs-docs]")).toBeNull();
  await mac.unmount();
  const linux = await mount(moved({ needs: { kind: "linger", command: "sudo loginctl enable-linger hy" } }), { moved: true });
  expect(linux.q('[data-device-needs="mini:linger"]')?.textContent).toBe("Needs sudo" + "sudo loginctl enable-linger hy");
  await linux.unmount();
  const wsl = await mount(moved({ needs: { kind: "open_distribution" } }), { moved: true });
  expect(wsl.q('[data-device-needs="mini:open_distribution"]')?.textContent).toBe("After Windows restarts, open this distribution again");
  await wsl.unmount();
});

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
