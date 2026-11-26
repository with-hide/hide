import { emptyScope } from "../../test/legacyAgentScope";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { NOTHING_UNDER_WAY, type PairingResult, type PairingView, type Peer } from "../pairing";
import type { Device } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { DispatchFn } from "../ws";
import { DeviceMoveWatch } from "./DeviceSwitch";
import { DevicesTab } from "./DevicesTab";
import { SWITCH_ANSWER_WITHIN_MS } from "./switchWays";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const LOCAL: Device = { agent_scope: emptyScope(), id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
const host = (platform: string) => ({ platform }) as Device["host"];
/** A device this computer reaches over SSH. */
const helper = (over: Partial<Device> = {}): Device => ({ agent_scope: emptyScope(), id: "mini", label: "Mac mini", kind: "remote", state: "ready", message: null, ssh_alias: "mini", agent_count: 0, test: null, host: host("macos aarch64"), ...over });
const KEY = "ab".repeat(32);
/** The same device once it was switched: its machine's own Hide, under the id it had. */
const moved = (over: Partial<Device> = {}): Device => helper({ ssh_alias: null, peer_key: KEY, switched: true, ...over });
const peer = (over: Partial<Peer> = {}): Peer => ({ key: KEY, device_id: "mini", name: "Mac mini", opens: "active", opened_by: null, connected: true, path: "direct", reason: null, update: null, ...over });

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(devices: Device[], over: { peers?: Peer[]; pairing?: PairingView | null } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  const rest = (listed: Device[], peers: Peer[]) => ({
    navigator: { focused_device_id: "local", devices: [LOCAL, ...listed], peers, workspaces: [] },
    status: { remote: listed.filter((device) => device.state === "ready").map((device) => ({ target_id: device.id, state: "connected", message: null })) },
    ui_state: { workspace_registrations: [{ id: "workspace:app", label: "app", path: "/home/me/app", device_id: "mini", pinned: false, primary_checkout_id: null }] },
  });
  await act(async () => {
    // This computer's own window unless told otherwise: hided sent it the `pairing` frame.
    useShellStore.setState({ connection: "live", pairing: over.pairing === undefined ? NOTHING_UNDER_WAY : over.pairing, pairingResult: null, rest: rest(devices, over.peers ?? []) } as never);
    root.render(<TooltipProvider><DeviceMoveWatch /><DevicesTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const asked = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload as Record<string, unknown>);
  /** hided's answer to the last ask of `verb`. */
  const answer = async (verb: "look_switch" | "switch", result: Partial<PairingResult>) => {
    const last = asked(`pairing_${verb}`).at(-1);
    await act(async () => { useShellStore.setState({ pairingResult: { verb, ok: true, request_id: last?.request_id as string, ...result } }); });
  };
  const press = async (selector: string) => { await act(async () => { q(selector)?.click(); }); };
  const show = async (listed: Device[], peers: Peer[] = []) => { await act(async () => { useShellStore.setState({ rest: rest(listed, peers) } as never); }); };
  const unmount = async () => {
    await act(async () => root.unmount());
    useUiStore.setState({ switchedDevices: [], devicesOpen: [], switchOutcomes: {} });
    useShellStore.setState(saved, true);
  };
  return { q, asked, answer, press, show, unmount };
}

const FOLD = '[data-device-row="mini"] [data-row-fold]';
const SWITCH = '[data-device-switch="mini"]';

it("opens an SSH device's row on its two ways and reads its machine once, changing nothing there", async () => {
  const { q, asked, answer, press, unmount } = await mount([helper()]);
  expect(q('[data-device-way="mini:ssh"]')?.textContent).toBe("SSH");
  // Folded, the row says nothing of a condition and asks the machine nothing.
  expect(q("[data-device-ways]")).toBeNull();
  expect(asked("pairing_look_switch")).toEqual([]);
  await press(FOLD);
  expect(asked("pairing_look_switch")).toHaveLength(1);
  expect(asked("pairing_look_switch")[0]?.device).toBe("mini");
  expect(q('[data-device-way-row="direct"]')?.getAttribute("data-device-way-now")).toBe("true");
  expect(q('[data-device-way-row="server"]')?.getAttribute("data-device-way-now")).toBe("false");
  // Until the machine answers there is nothing to press.
  expect(q('[data-device-condition="mini"]')?.textContent).toBe("…Checking");
  expect(q(SWITCH)).toBeNull();
  await answer("look_switch", { device_id: "mini", condition: { kind: "auto_login_on" }, user: "me" });
  expect(q('[data-device-condition-kind="auto_login_on"]')?.textContent).toContain("Automatic login on");
  expect(q(SWITCH)?.hasAttribute("disabled")).toBe(false);
  // What was read stays while the row is open; opening it again reads again.
  await press(FOLD);
  await press(FOLD);
  expect(asked("pairing_look_switch")).toHaveLength(2);
  expect(asked("pairing_switch")).toEqual([]);
  await unmount();
});

/** The words a card's rows end on while this computer sleeps, the moment the two ways differ. */
const asleep = (q: (selector: string) => HTMLElement | null, card: "direct" | "server") =>
  Object.fromEntries(
    ["agents", "missions", "automations"].map((row) => [row, q(`[data-device-way-row="${card}"] [data-device-keep="${row}"] [data-keep-asleep]`)?.textContent]),
  );

it("lays the two ways side by side and says what each keeps doing while this computer sleeps", async () => {
  const { q, answer, press, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_on" }, user: "me" });
  const direct = '[data-device-way-row="direct"]';
  const server = '[data-device-way-row="server"]';
  expect(q(`${direct} [data-device-way-headline]`)?.textContent).toBe("When this computer sleeps,missions and automations stop too");
  expect(q(`${server} [data-device-way-headline]`)?.textContent).toBe("Even when this computer sleeps,it keeps running on Mac mini");
  // Over SSH the agents go on in their terminals, while missions and automations wait for this computer.
  expect(asleep(q, "direct")).toEqual({ agents: "●Keep doing their work", missions: "‖Stop", automations: "‖Stop" });
  expect(asleep(q, "server")).toEqual({ agents: "●Keep running", missions: "●Keep running", automations: "●Keep running" });
  // What the device itself offers does not move with the beat.
  expect(q(`${direct} [data-device-keep="there"]`)?.textContent).toBe("On Mac miniNo Hide screen");
  expect(q(`${server} [data-device-keep="there"]`)?.textContent).toBe("On Mac miniInstall the app to use it there");
  expect(q(`${direct} [data-device-way-picture="direct"]`)?.getAttribute("aria-label")).toBe("Hide runs only on this computer. When it sleeps, that device keeps only a small tool.");
  expect(q(`${server} [data-device-way-picture="server"]`)).not.toBeNull();
  // The button and what it costs sit with the server card.
  expect(q(`${server} ${SWITCH}`)).not.toBeNull();
  expect(q(server)?.textContent).toContain("In exchange, memory, standing instructions and the phone connection are set up on that device separately");
  expect(q(direct)?.textContent).toContain("That device only gets a small tool");
  await unmount();
});

it("draws no sleep picture for a WSL distribution, which runs on this PC and sleeps with it", async () => {
  const { q, answer, press, unmount } = await mount([helper({ ssh_alias: null, wsl_distro: "Ubuntu", host: host("linux x86_64") })]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "linger_on" } });
  expect(q('[data-device-way-row="direct"]')?.textContent).toContain("Connect directly over WSL");
  expect(q("[data-device-way-picture]")).toBeNull();
  expect(q("[data-device-way-keeps]")).toBeNull();
  expect(q("[data-device-way-headline]")).toBeNull();
  expect(q('[data-device-way-row="server"]')?.textContent).toContain("For a device that runs a Hide of its own");
  expect(q(SWITCH)).not.toBeNull();
  await unmount();
});

it("warns about a Mac that does not log in by itself and still lets it be switched, with the steps only where macOS offers them", async () => {
  const { q, answer, press, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_off", how: true }, user: "me" });
  expect(q('[data-device-auto-login="off"]')?.textContent).toContain("Automatic login is off · after Mac mini restarts, log in to that Mac once and it connects again");
  expect(q(SWITCH)?.hasAttribute("disabled")).toBe(false);
  await press("[data-device-auto-login-how]");
  expect(q("[data-device-auto-login-steps]")?.textContent).toContain("Under Automatically log in as, choose me");
  await unmount();
  // FileVault: macOS offers no automatic login, so there is nothing to show how to turn on.
  const vault = await mount([helper()]);
  await vault.press(FOLD);
  await vault.answer("look_switch", { condition: { kind: "auto_login_off", how: false }, user: "me" });
  expect(vault.q('[data-device-auto-login="off"]')).not.toBeNull();
  expect(vault.q("[data-device-auto-login-how]")).toBeNull();
  expect(vault.q(SWITCH)?.hasAttribute("disabled")).toBe(false);
  await vault.unmount();
});

it("turns the button off for a Mac nobody is logged in to, and draws none where nothing there changes it", async () => {
  const { q, answer, press, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "signed_out" } });
  expect(q('[data-device-condition-kind="signed_out"]')?.textContent).toContain("Nobody is logged in to Mac mini");
  expect(q(SWITCH)?.hasAttribute("disabled")).toBe(true);
  await press(FOLD);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "no_systemd" } });
  expect(q('[data-device-condition-kind="no_systemd"]')?.textContent).toBe("This device has no systemd, so it cannot be connected as a server");
  expect(q(SWITCH)).toBeNull();
  // A login that asks for an answer was never read, and is never switched.
  await press(FOLD);
  await press(FOLD);
  await answer("look_switch", { ok: false, reason: "ssh_not_reached" });
  expect(q('[data-device-condition-kind="password"]')?.textContent).toBe("Its SSH login asks for a password, so it cannot be moved to a server");
  expect(q(SWITCH)).toBeNull();
  await unmount();
});

it("asks a Linux account to linger only on the press, and names the command once only an administrator can", async () => {
  const { q, asked, answer, press, unmount } = await mount([helper({ host: host("linux x86_64") })]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "linger_off" } });
  expect(q('[data-device-condition-kind="linger_off"]')?.textContent).toContain("Hide tries to turn it on when it connects");
  await press(SWITCH);
  expect(asked("pairing_switch")).toEqual([expect.objectContaining({ device: "mini", install: true })]);
  // The press read the machine again and could not turn lingering on: the device stays as it is.
  await answer("switch", { switch: "left", condition: { kind: "linger_admin", command: "sudo loginctl enable-linger me" } });
  expect(q('[data-device-linger-command="mini"]')?.textContent).toBe("sudo loginctl enable-linger me");
  expect(q(SWITCH)?.hasAttribute("disabled")).toBe(true);
  // A look cannot tell who may turn it on: while it still reads off, the command stays.
  await press('[data-device-condition-read="mini"]');
  await answer("look_switch", { condition: { kind: "linger_off" } });
  expect(q('[data-device-condition-kind="linger_admin"]')).not.toBeNull();
  await press('[data-device-condition-read="mini"]');
  await answer("look_switch", { condition: { kind: "linger_on" } });
  expect(q('[data-device-condition-kind="linger_on"]')).not.toBeNull();
  expect(q(SWITCH)?.hasAttribute("disabled")).toBe(false);
  await unmount();
});

it("reads no machine it cannot switch or reach: a Windows device says so in one line, and one that is not connected waits", async () => {
  const windows = await mount([helper({ host: host("windows x86_64") })]);
  await windows.press(FOLD);
  expect(windows.q('[data-device-ways="mini:windows"]')?.textContent).toBe("Windows cannot be connected as a server with a button yet · to use it as a server, install Hide on that PC, remove this device, then connect from Add device");
  expect(windows.asked("pairing_look_switch")).toEqual([]);
  await windows.unmount();
  const down = await mount([helper({ state: "unavailable" })]);
  await down.press(FOLD);
  expect(down.q('[data-device-condition="mini"]')?.textContent).toBe("Checked once it is connected");
  expect(down.asked("pairing_look_switch")).toEqual([]);
  await down.unmount();
});

it("draws the steps the core says a switch is at in the button's place, with nothing to cancel", async () => {
  const { q, answer, press, show, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_on" } });
  await show([helper({ switch_step: "starting" })]);
  expect(q(SWITCH)).toBeNull();
  // The steps take the server card's picture and rows; the card in use says how it ends.
  expect(q('[data-device-way-row="server"] [data-switch-step="starting:on"]')).not.toBeNull();
  expect(q("[data-device-way-picture]")).toBeNull();
  expect(q("[data-device-way-keeps]")).toBeNull();
  expect(q('[data-device-way-row="direct"]')?.getAttribute("data-device-way-now")).toBe("true");
  expect(q('[data-device-way-row="direct"]')?.textContent).toContain("It becomes a server once this finishes");
  expect([...document.querySelectorAll("[data-switch-step]")].map((step) => step.getAttribute("data-switch-step"))).toEqual(["checking:done", "starting:on", "pairing:later", "connecting:later", "projects:later"]);
  expect(q('[data-switch-step="starting:on"]')?.textContent).toContain("Start Hide on Mac mini");
  // Over, with the device not switched: the button is back.
  await show([helper()]);
  expect(q(SWITCH)).not.toBeNull();
  await unmount();
});

it("says why a press did not switch the device and offers it again", async () => {
  const { q, asked, answer, press, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_on" } });
  await press(SWITCH);
  await answer("switch", { switch: "held", by: "Office MacBook", terminals: 3 });
  expect(q('[data-device-switch-outcome="mini:held"]')?.textContent).toContain("Office MacBook moved Mac mini to a server first · close the 3 terminals this computer runs on Mac mini, then try again");
  // Retry is the same press.
  expect(q(SWITCH)?.textContent).toBe("Retry");
  await press(SWITCH);
  expect(asked("pairing_switch")).toHaveLength(2);
  await answer("switch", { ok: false, reason: "ssh_core_not_started" });
  expect(q('[data-device-switch-outcome="mini:failed"]')?.textContent).toContain("Hide could not be started on Mac mini · try again");
  expect(q('[data-device-way="mini:ssh"]')).not.toBeNull();
  await unmount();
});

it("ends a press whose login was cut or did not sign in as a failed press to retry, never as the password line only the row's read says", async () => {
  const { q, asked, answer, press, unmount } = await mount([helper({ host: host("linux x86_64") })]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "linger_off" } });
  // The login ended while the machine accepted the pairing, and then the retry's own read did not sign in.
  for (const reason of ["switch_cut", "ssh_not_reached"]) {
    await press(SWITCH);
    await answer("switch", { ok: false, reason });
    expect(q('[data-device-switch-outcome="mini:failed"]')?.textContent).toContain("The Hide on Mac mini could not be connected · try again");
    expect(q('[data-device-condition-kind="password"]')).toBeNull();
    expect(q(SWITCH)?.textContent).toBe("Retry");
  }
  expect(asked("pairing_switch")).toHaveLength(2);
  expect(q('[data-device-way="mini:ssh"]')).not.toBeNull();
  // Folded and opened again, the row reads the machine again, and what it says now replaces the failed press.
  await press(FOLD);
  await press(FOLD);
  await answer("look_switch", { ok: false, reason: "ssh_not_reached" });
  expect(q('[data-device-condition-kind="password"]')?.textContent).toBe("Its SSH login asks for a password, so it cannot be moved to a server");
  expect(q('[data-device-switch-outcome="mini:failed"]')).toBeNull();
  expect(q(SWITCH)).toBeNull();
  await unmount();
});

it("ends a press nothing answered as one that failed, when the daemon went away or the daemon's own bound passed", async () => {
  const { q, answer, press, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_on" } });
  await press(SWITCH);
  expect(q(SWITCH)).toBeNull();
  expect(q('[data-switch-step="checking:on"]')).not.toBeNull();
  await act(async () => { useShellStore.setState({ connection: "reconnecting" } as never); });
  expect(q("[data-switch-step]")).toBeNull();
  expect(q('[data-device-switch-outcome="mini:failed"]')?.textContent).toContain("It could not be connected as a server · try again");
  expect(q(SWITCH)?.textContent).toBe("Retry");
  await unmount();
  // The daemon stays, and the answer never comes: the steps do not turn for ever.
  const quiet = await mount([helper()]);
  await quiet.press(FOLD);
  await quiet.answer("look_switch", { condition: { kind: "auto_login_on" } });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    await quiet.press(SWITCH);
    await act(async () => { vi.advanceTimersByTime(SWITCH_ANSWER_WITHIN_MS - 1); });
    expect(quiet.q('[data-switch-step="checking:on"]')).not.toBeNull();
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(quiet.q('[data-device-switch-outcome="mini:failed"]')).not.toBeNull();
    // An answer in time ends the wait: nothing fails later.
    await quiet.press(SWITCH);
    await quiet.answer("switch", { switch: "held", by: "Office MacBook", terminals: 1 });
    await act(async () => { vi.advanceTimersByTime(SWITCH_ANSWER_WITHIN_MS); });
    expect(quiet.q('[data-device-switch-outcome="mini:held"]')).not.toBeNull();
  } finally {
    vi.useRealTimers();
  }
  await quiet.unmount();
});

it("says once, on the row of a device switched while this window watched, what starts fresh there (D-48)", async () => {
  const { q, answer, press, show, unmount } = await mount([helper()]);
  await press(FOLD);
  await answer("look_switch", { condition: { kind: "auto_login_on" } });
  await show([moved()], [peer()]);
  // The same row, open as it was, is the machine's now.
  expect(q('[data-device-way="mini:server"]')?.textContent).toBe("Server");
  expect(q('[data-device-switched="mini"]')?.textContent).toContain("Memory and standing instructions start fresh on Mac mini");
  expect(q('[data-device-switched="mini"]')?.textContent).toContain("1 project is registered in Hide on Mac mini too");
  expect(q('[data-device-way-row="server"]')?.getAttribute("data-device-way-now")).toBe("true");
  // A switched machine has nothing to press, and says how to go back.
  expect(q(SWITCH)).toBeNull();
  expect(q('[data-device-ways-back="mini"]')?.textContent).toBe("To go back, remove this device and add it over SSH again");
  // The switched machine is still told apart from the direct way by what each keeps doing while this computer sleeps.
  expect(asleep(q, "server")).toEqual({ agents: "●Keep running", missions: "●Keep running", automations: "●Keep running" });
  await press('[data-device-switched-seen="mini"]');
  expect(q("[data-device-switched]")).toBeNull();
  expect(useUiStore.getState().switchedDevices).toEqual([]);
  await unmount();
  // A machine that was switched before this window watched it says nothing.
  const later = await mount([moved()], { peers: [peer()] });
  await later.press('[data-peer-row="mini"] [data-row-fold]');
  expect(later.q('[data-device-ways="mini:server"]')).not.toBeNull();
  expect(later.q("[data-device-switched]")).toBeNull();
  await later.unmount();
});

it("opens only a machine that was a device: one paired by a link keeps its row as it is", async () => {
  const { q, asked, unmount } = await mount([moved({ switched: false })], { peers: [peer()] });
  expect(q('[data-peer-row="mini"]')).not.toBeNull();
  expect(q('[data-peer-row="mini"] [data-row-fold]')).toBeNull();
  expect(q("[data-device-way]")).toBeNull();
  expect(asked("pairing_look_switch")).toEqual([]);
  await unmount();
});

it("says that logging in connects a switched Mac again only when its machine was read as having nobody logged in", async () => {
  const { q, asked, answer, unmount } = await mount([moved({ state: "unavailable" })], { peers: [peer({ connected: false, path: null })] });
  // Not connected: read once over the login it was switched through, without the row being opened.
  expect(asked("pairing_look_switch")).toHaveLength(1);
  expect(q('[data-device-reason="mini"]')).toBeNull();
  await answer("look_switch", { condition: { kind: "signed_out" } });
  expect(q('[data-device-reason="mini"]')?.textContent).toBe("Log in to Mac mini and it connects again");
  await unmount();
  // SSH does not reach it either: the row says only that it is not connected.
  const dark = await mount([moved({ state: "unavailable" })], { peers: [peer({ connected: false, path: null })] });
  await dark.answer("look_switch", { ok: false, reason: "ssh_not_reached" });
  expect(dark.q('[data-device-reason="mini"]')).toBeNull();
  await dark.unmount();
});

it("offers nothing to open or press on a screen that is not this computer's own window (B52)", async () => {
  const { q, asked, unmount } = await mount([helper()], { pairing: null });
  expect(q('[data-device-way="mini:ssh"]')?.textContent).toBe("SSH");
  expect(q("[data-row-fold]")).toBeNull();
  expect(asked("pairing_look_switch")).toEqual([]);
  await unmount();
});
