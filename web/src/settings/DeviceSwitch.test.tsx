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
import type { DispatchFn } from "../ws";
import { DeviceSwitchDialogs, DeviceSwitchStrip } from "./DeviceSwitch";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const LOCAL: Device = { agent_scope: emptyScope(), id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
const helper = (over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id: "mini", label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 0, test: null, switchable: true, ...over });

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(device: Device, over: { front?: string; pairing?: PairingView | null; panes?: number } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const rest = (next: Device) => ({
    navigator: { focused_device_id: over.front ?? next.id, devices: [LOCAL, next], workspaces: [{ id: "w", device_id: next.id, removal: { pane_count: over.panes ?? 0, running_agent_count: 0 }, checkouts: [] }] },
    status: { remote: [] },
    ui_state: {},
  });
  // `place` is where the window draws the strip: the main screen, or the
  // remote screen once a machine's own Hide shows (App, RemoteStage).
  const draw = (place: string) => root.render(<TooltipProvider><DeviceSwitchStrip key={place} actions={actions} /><DeviceSwitchDialogs actions={actions} /></TooltipProvider>);
  await act(async () => {
    useShellStore.setState({ connection: "live", pairing: over.pairing === undefined ? NOTHING_UNDER_WAY : over.pairing, pairingResult: null, rest: rest(device) } as never);
    draw("main");
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const show = async (next: Device) => { await act(async () => { useShellStore.setState({ rest: rest(next) } as never); }); };
  // hided's answer to the last press.
  const answer = async (result: Record<string, unknown>) => {
    const request = switches(events).at(-1)?.payload.request_id;
    await act(async () => { useShellStore.setState({ pairingResult: { verb: "switch", request_id: request, ...result } } as never); });
  };
  const moveStrip = async (place: string) => { await act(async () => draw(place)); };
  return { events, q, show, answer, moveStrip, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const switches = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "pairing_switch");

it("offers the switch over a helper device's screen, and a press asks hided for that device once (B35)", async () => {
  const { events, q, unmount } = await mount(helper());
  expect(q('[data-device-switch-strip="mini"]')?.textContent).toBe("Switch Mac mini to the new connection · running terminals carry on" + "Connect the new way");
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  expect(switches(events).map((event) => ({ device: event.payload.device, own_folder: event.payload.own_folder }))).toEqual([{ device: "mini", own_folder: undefined }]);
  await unmount();
});

it("draws no strip over this computer's own screen, a paired machine's, or on a screen that is not this computer's own window (B52)", async () => {
  for (const [device, over] of [
    [helper(), { front: "local" }],
    [helper({ peer_key: "a".repeat(64), switchable: false }), {}],
    [helper(), { pairing: null }],
  ] as const) {
    const { q, unmount } = await mount(device, over);
    expect(q("[data-device-switch-strip]")).toBeNull();
    await unmount();
  }
});

it("shows the step the switch is on in place of the button, then why it stopped with Retry (B36, B41)", async () => {
  const { events, q, show, unmount } = await mount(helper({ switch: { state: "checking" } }));
  expect(q('[data-device-switch-state="mini:checking"]')?.textContent).toBe("Checking→Connecting→Done");
  expect(q('[data-device-switch="mini"]')).toBeNull();
  await show(helper({ switch: { state: "connecting" } }));
  expect(q('[data-device-switch-state="mini:connecting"]')?.textContent).toBe("Installing→Connecting→Done");
  expect(q('[data-device-switch-state="mini:connecting"] [aria-current="step"]')?.textContent).toBe("Connecting");
  await show(helper({ switch: { state: "failed", reason: "ssh_not_reached" } }));
  expect(q('[data-device-switch-state="mini:failed"]')?.textContent).toBe("!SSH did not reach it" + "Retry");
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  expect(switches(events)).toHaveLength(1);
  await unmount();
});

it("asks before a later computer's switch closes its terminals, and goes on with a folder of its own only on Switch (B69)", async () => {
  const { events, q, answer, unmount } = await mount(helper(), { panes: 3 });
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  await answer({ ok: true, switch: "held", by: "Office Mac", device_id: "mini" });
  const dialog = q('[data-device-switch-held="mini"]')!;
  expect(dialog.textContent).toContain("Connect Mac mini the new way?");
  expect(dialog.textContent).toContain("Office Mac switched this device to the new connection first · 3 running terminals close · AI conversations can be resumed");
  await act(async () => { q('[data-device-switch-held-go="true"]')?.click(); });
  expect(switches(events).map((event) => event.payload.own_folder)).toEqual([undefined, true]);
  expect(q("[data-device-switch-held]")).toBeNull();
  await unmount();
});

it("leaves the device as it was when the question is cancelled, and does not ask again for the same answer (B69)", async () => {
  const { events, q, answer, unmount } = await mount(helper());
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  await answer({ ok: true, switch: "held", by: "Office Mac", device_id: "mini" });
  // No terminal of this computer runs there: the line names none.
  expect(q('[data-device-switch-held="mini"]')?.textContent).toContain("Office Mac switched this device to the new connection first · AI conversations can be resumed");
  await act(async () => { (document.body.querySelector('[data-device-switch-held="mini"] button') as HTMLElement).click(); });
  expect(q("[data-device-switch-held]")).toBeNull();
  expect(switches(events)).toHaveLength(1);
  await unmount();
});

it("goes on without removing a device added for its switch when the held question is answered with Switch, and removes it on Cancel (B41, B69)", async () => {
  const removed = (events: { kind: string; payload: Record<string, unknown> }[]) => events.filter((event) => event.kind === "remove_device").map((event) => event.payload.device_id);
  const go = await mount(helper({ switch_first: true }));
  await act(async () => { go.q('[data-device-switch="mini"]')?.click(); });
  await go.answer({ ok: true, switch: "held", by: "Office Mac", device_id: "mini" });
  await act(async () => { go.q('[data-device-switch-held-go="true"]')?.click(); });
  expect(go.q("[data-device-switch-held]")).toBeNull();
  expect(switches(go.events).map((event) => event.payload.own_folder)).toEqual([undefined, true]);
  expect(removed(go.events)).toEqual([]);
  await go.unmount();

  const left = await mount(helper({ switch_first: true }));
  await act(async () => { left.q('[data-device-switch="mini"]')?.click(); });
  await left.answer({ ok: true, switch: "held", by: "Office Mac", device_id: "mini" });
  await act(async () => { (document.body.querySelector('[data-device-switch-held="mini"] [data-slot="alert-dialog-cancel"]') as HTMLElement).click(); });
  expect(left.q("[data-device-switch-held]")).toBeNull();
  expect(switches(left.events)).toHaveLength(1);
  expect(removed(left.events)).toEqual(["mini"]);
  await left.unmount();
});

it("says what would be put on a machine with no Hide of its own, and starts it only on Allow and install (B36, D-45)", async () => {
  const { events, q, answer, unmount } = await mount(helper());
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  await answer({ ok: true, switch: "install_needed", system: "linux", device_id: "mini" });
  const dialog = q('[data-device-switch-install="mini"]')!;
  expect(dialog.textContent).toContain("Install Hide on Mac mini?");
  // Linger is named for a Linux machine only.
  expect(q("[data-device-switch-install-parts]")?.getAttribute("data-device-switch-install-parts")).toBe("core,autostart,linger");
  expect(dialog.textContent).toContain("This computer's Hide program and agent hooks are installed in that account");
  expect(switches(events)).toHaveLength(1);
  await act(async () => { q('[data-device-switch-install-go="true"]')?.click(); });
  expect(switches(events).map((event) => event.payload.install)).toEqual([undefined, true]);
  expect(q("[data-device-switch-install]")).toBeNull();
  await unmount();
});

it("offers a WSL distribution's switch as an SSH device's, and names no linger in what it installs (B35, B36, D-27)", async () => {
  const distribution = helper({ id: "wsl-ubuntu-1a2b3c4d", label: "WSL Ubuntu", ssh_alias: null, wsl_distro: "Ubuntu" });
  const { events, q, answer, show, unmount } = await mount(distribution);
  expect(q('[data-device-switch-strip="wsl-ubuntu-1a2b3c4d"]')?.textContent).toContain("Switch WSL Ubuntu to the new connection");
  await act(async () => { q('[data-device-switch="wsl-ubuntu-1a2b3c4d"]')?.click(); });
  expect(switches(events).map((event) => event.payload.device)).toEqual(["wsl-ubuntu-1a2b3c4d"]);
  await answer({ ok: true, switch: "install_needed", system: "linux", device_id: "wsl-ubuntu-1a2b3c4d" });
  expect(q("[data-device-switch-install-parts]")?.getAttribute("data-device-switch-install-parts")).toBe("core,autostart");
  await act(async () => { q('[data-device-switch-install-go="true"]')?.click(); });
  // WSL could not run in it: the distribution stays a helper, with why and Try again (B41).
  await show({ ...distribution, switch: { state: "failed", reason: "wsl_not_reached" } });
  expect(q('[data-device-switch-state="wsl-ubuntu-1a2b3c4d:failed"]')?.textContent).toContain("WSL could not open this distribution");
  await unmount();
});

it("asks for the switch of a distribution added for it once its helper is connected (B41)", async () => {
  const ready = { host: { consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: null, granted_at_unix_ms: 1, state: "ready", message: null, platform: "linux x86_64", helper_path: null } } as Partial<Device>;
  const distribution = helper({ id: "wsl-ubuntu-1a2b3c4d", label: "WSL Ubuntu", ssh_alias: null, wsl_distro: "Ubuntu", state: "unavailable", switch_first: true });
  const { events, show, unmount } = await mount(distribution);
  expect(switches(events)).toHaveLength(0);
  await show({ ...distribution, state: "ready", ...ready });
  expect(switches(events).map((event) => event.payload.device)).toEqual(["wsl-ubuntu-1a2b3c4d"]);
  await unmount();
});

it("asks for the switch of a device added over SSH once its helper is connected, and only once (B41)", async () => {
  const ready = { host: { consent: "granted", helper_root: "~/.hide/host-helper", cli_dir: "~/.local/bin", contract: 3, bound_identity: null, granted_at_unix_ms: 1, state: "ready", message: null, platform: "linux x86_64", helper_path: null } } as Partial<Device>;
  // The core keeps that it was added for its switch; it has not connected yet, so it is not offered.
  const { events, show, unmount } = await mount(helper({ state: "unavailable", switch_first: true, switchable: false }));
  expect(switches(events)).toHaveLength(0);
  await show(helper({ ...ready, switch_first: true }));
  expect(switches(events).map((event) => ({ device: event.payload.device, install: event.payload.install }))).toEqual([{ device: "mini", install: undefined }]);
  await show(helper({ ...ready, switch_first: true, message: "again" }));
  expect(switches(events)).toHaveLength(1);
  // A helper device that was there before is never asked on its own.
  await show(helper({ ...ready, id: "older" }));
  expect(switches(events)).toHaveLength(1);
  // Taken off and added again under the id it had: it is a new device, asked again.
  await show(helper({ ...ready, id: "mini", switch_first: true }));
  expect(switches(events).map((event) => event.payload.device)).toEqual(["mini", "mini"]);
  await unmount();
});

it("takes a device added for its switch off again when the install question is left, and leaves an older one as it was (B41)", async () => {
  for (const [switchFirst, removed] of [[true, ["mini"]], [false, []]] as const) {
    const { events, q, answer, unmount } = await mount(helper({ switch_first: switchFirst }));
    await act(async () => { q('[data-device-switch="mini"]')?.click(); });
    await answer({ ok: true, switch: "install_needed", system: "linux", device_id: "mini" });
    await act(async () => { (Array.from(q('[data-device-switch-install="mini"]')?.querySelectorAll("button") ?? []).find((button) => !button.hasAttribute("data-device-switch-install-go") && button.textContent === "Cancel") as HTMLElement | undefined)?.click(); });
    expect(q("[data-device-switch-install]")).toBeNull();
    expect(events.filter((event) => event.kind === "remove_device").map((event) => event.payload.device_id)).toEqual(removed);
    await unmount();
  }
});

it("keeps a landed switch's line while the strip moves from the main screen to the machine's own screen (D-48)", async () => {
  const machine = helper({ ssh_alias: null, peer_key: "ab".repeat(32), switch: null });
  const { q, show, moveStrip, unmount } = await mount(helper({ switch: { state: "connecting" } }));
  await show(machine);
  // The switch landing brings the machine's own screen, which draws the strip anew.
  await moveStrip("remote");
  expect(q('[data-device-switched="mini"]')?.textContent).toBe("Memory and standing instructions start fresh on this machine");
  await unmount();
});

it("says a landed switch's line only over that machine's screen, and ends it when another computer's screen opens (D-48)", async () => {
  const machine = helper({ ssh_alias: null, peer_key: "ab".repeat(32), switch: null });
  const { q, show, moveStrip, unmount } = await mount(helper({ switch: { state: "connecting" } }));
  await show(machine);
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

it("says once how many terminals a landed switch could not take over (B36)", async () => {
  const { q, show, unmount } = await mount(helper({ switch: { state: "connecting" } }));
  await show(helper({ ssh_alias: null, peer_key: "a".repeat(64), switchable: false, not_carried: 2 }));
  expect(q('[data-device-switched="mini"] [data-device-not-carried="2"]')?.textContent).toContain("2 terminals could not be taken over · AI conversations can be resumed");
  await unmount();
});

it("says once over a machine's screen what starts fresh there after its switch landed in this window (D-48)", async () => {
  const machine = helper({ ssh_alias: null, peer_key: "ab".repeat(32), switch: null });
  const { q, show, unmount } = await mount(helper({ switch: { state: "connecting" } }));
  expect(q("[data-device-switched]")).toBeNull();
  await show(machine);
  expect(q('[data-device-switched="mini"]')?.textContent).toBe("Memory and standing instructions start fresh on this machine");
  // Once: leaving the machine's screen ends it, and coming back says nothing.
  await act(async () => { useShellStore.setState({ rest: { navigator: { focused_device_id: "local", devices: [LOCAL, machine], workspaces: [] }, status: { remote: [] }, ui_state: {} } } as never); });
  await show(machine);
  expect(q("[data-device-switched]")).toBeNull();
  expect(useUiStore.getState().switchedDevices).toEqual([]);
  await unmount();
  // A machine switched before this window watched it says nothing.
  const later = await mount(machine);
  expect(later.q("[data-device-switched]")).toBeNull();
  await later.unmount();
});
