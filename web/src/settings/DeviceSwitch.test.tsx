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
import type { DispatchFn } from "../ws";
import { DeviceSwitchDialogs, DeviceSwitchStrip } from "./DeviceSwitch";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const LOCAL: Device = { agent_scope: emptyScope(), id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
const helper = (over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id: "mini", label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 0, test: null, ...over });

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
  await act(async () => {
    useShellStore.setState({ connection: "live", pairing: over.pairing === undefined ? NOTHING_UNDER_WAY : over.pairing, pairingResult: null, rest: rest(device) } as never);
    root.render(<TooltipProvider><DeviceSwitchStrip actions={actions} /><DeviceSwitchDialogs actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const show = async (next: Device) => { await act(async () => { useShellStore.setState({ rest: rest(next) } as never); }); };
  // hided's answer to the last press.
  const answer = async (result: Record<string, unknown>) => {
    const request = switches(events).at(-1)?.payload.request_id;
    await act(async () => { useShellStore.setState({ pairingResult: { verb: "switch", request_id: request, ...result } } as never); });
  };
  return { events, q, show, answer, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const switches = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "pairing_switch");

it("offers the switch over a helper device's screen, and a press asks hided for that device once (B35)", async () => {
  const { events, q, unmount } = await mount(helper());
  expect(q('[data-device-switch-strip="mini"]')?.textContent).toBe("Switch Mac mini to the new connection · running terminals carry on" + "Switch");
  await act(async () => { q('[data-device-switch="mini"]')?.click(); });
  expect(switches(events).map((event) => ({ device: event.payload.device, own_folder: event.payload.own_folder }))).toEqual([{ device: "mini", own_folder: undefined }]);
  await unmount();
});

it("draws no strip over this computer's own screen, a paired machine's, or on a screen that is not this computer's own window (B52)", async () => {
  for (const [device, over] of [
    [helper(), { front: "local" }],
    [helper({ peer_key: "a".repeat(64) }), {}],
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
  expect(dialog.textContent).toContain("Switch Mac mini?");
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
