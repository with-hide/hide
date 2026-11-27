// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { PairingDialogs } from "../PairingDialogs";
import type { PairingView, Peer } from "../pairing";
import type { Device, GuestPanes } from "../snapshot";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { forgetFirewallAsk } from "./firewall";
import { DevicesTab } from "./DevicesTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

const peer = (over: Partial<Peer> = {}): Peer => ({ key: "ab".repeat(32), device_id: "windows-pc", name: "Windows PC", opens: "active", opened_by: null, connected: true, path: "direct", reason: null, update: null, ...over });
const idle: PairingView = { offer: null, asks: [], join: null, requests: [] };
const WORDS = ["바다", "연필", "구름", "사과"];

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
});

async function mount(pairing: PairingView | null, peers: Peer[] = [], guest_panes: GuestPanes[] = [], devices: Partial<Device>[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    // This computer's own row carries the share; the paired machines are the rows under it.
    const own = { id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
    useShellStore.setState({ connection: "live", pairing, pairingResult: null, rest: { navigator: { focused_device_id: "local", devices: [own, ...devices], peers, guest_panes }, ui_state: {} } } as never);
    root.render(
      <TooltipProvider>
        <DevicesTab actions={actions} />
        <PairingDialogs actions={actions} />
      </TooltipProvider>,
    );
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const press = async (selector: string) => act(async () => q(selector)?.click());
  return { events, q, press, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const last = (events: Parameters<DispatchFn>[0][]) => events.at(-1);

it("makes a link when the share window opens, walks its steps, and allows a pasted link in its last step (B1, B2)", async () => {
  vi.useFakeTimers({ now: 19_000, toFake: ["Date"] });
  const idleScreen = await mount(idle);
  expect(idleScreen.q("[data-pair-share]"), "the window opens on a press").toBeNull();
  await idleScreen.press("[data-pair-share-make]");
  expect(last(idleScreen.events)).toMatchObject({ kind: "pairing_share", payload: { language: "en" } });
  expect(idleScreen.q("[data-pair-share]")?.dataset.pairShare).toBe("making");
  await idleScreen.unmount();

  const offer = { link: "hide://pair#v=1&id=ab&s=cd", expires_unix_ms: 9 * 60_000 };
  const shared = await mount({ ...idle, offer });
  await shared.press("[data-pair-share-make]");
  // A link already shared is shown again, not made again.
  expect(shared.events.filter((event) => event.kind === "pairing_share")).toHaveLength(0);
  expect(shared.q("[data-pair-link]")?.textContent).toBe("hide://pair#v=1&id=ab&s=cd");
  expect(shared.q("[data-pair-share]")?.textContent).toContain("8:41 left · works once");
  const steps = () => [...document.body.querySelectorAll("[data-pair-step]")].map((step) => step.getAttribute("data-pair-step-state"));
  expect(steps()).toEqual(["on", "later", "later"]);
  expect(shared.q('[data-pair-step="2"]')?.textContent).toContain("Settings › Devices › Add device › A computer with Hide");

  // A machine pastes the link: its words show in the last step, and the allow window stays out of the way.
  const ask = { id: 7, key: "cd".repeat(32), device: "macbook", name: "MacBook", kind: "link" as const, words: WORDS, expires_unix_ms: 2 * 60_000 };
  await act(async () => useShellStore.setState({ pairing: { ...idle, asks: [ask] } }));
  expect(shared.q("[data-pair-share]")?.dataset.pairShare).toBe("words");
  expect(steps()).toEqual(["done", "done", "on"]);
  expect(shared.q('[data-pair-step="3"] [data-pair-words]')?.getAttribute("data-pair-words")).toBe(WORDS.join(" "));
  expect(shared.q("[data-pair-allow]")).toBeNull();
  await shared.press("[data-pair-allow-go]");
  expect(last(shared.events)).toMatchObject({ kind: "pairing_answer", payload: { ask: 7, allow: true } });
  expect(shared.q("[data-pair-share]"), "allowing closes the window").toBeNull();
  await shared.unmount();

  // Ending the link from the window.
  const ending = await mount({ ...idle, offer });
  await ending.press("[data-pair-share-make]");
  await ending.press("[data-pair-share-end]");
  expect(last(ending.events)).toMatchObject({ kind: "pairing_end_share" });
  expect(ending.q("[data-pair-share]")).toBeNull();
  await ending.unmount();
});

it("lists a paired machine with its state, its direction and the one action its state offers (B21, B24, B50)", async () => {
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active", path: "relay" });
  const lost = peer({ connected: false, path: null, reason: "relay_unreachable" });
  const { q, press, events, unmount } = await mount({ ...idle, requests: [] }, [guest, lost]);
  const row = (id: string) => q(`[data-peer-row="${id}"]`);
  expect(row("macbook")?.textContent).toContain("MacBook");
  // Connected says only that; the path and who opens whom are Connection details'.
  expect(q('[data-peer-status="macbook"]')?.textContent).toBe("●Connected");
  expect(row("macbook")?.querySelector("[data-peer-direction]")).toBeNull();
  expect(q('[data-peer-status="windows-pc"]')?.textContent).toBe("!Not connected");
  expect(q('[data-peer-reason="windows-pc"]')?.textContent).toBe("the relay cannot be reached");
  await press('[data-peer-retry="windows-pc"]');
  // An ask names the machine by its connection id, never by the device id it says of itself.
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: lost.key } });
  // The machine that opens this one is asked to be opened too.
  expect(q('[data-peer-request-go="macbook"]')?.textContent).toBe("Ask to open MacBook");
  await press('[data-peer-request-go="macbook"]');
  expect(last(events)).toMatchObject({ kind: "pairing_request", payload: { device: guest.key } });
  expect(q('[data-peer-request-go="windows-pc"]')).toBeNull();
  await unmount();
});

it("names a paired machine's system as its device reports it, and none before that device exists", async () => {
  const linux = peer({ key: "cd".repeat(32), device_id: "office", name: "Office Linux" });
  const fresh = peer();
  const { q, unmount } = await mount(idle, [linux, fresh], [], [{ id: "office", label: "Office Linux", kind: "remote", state: "ready", peer_key: linux.key, host: { platform: "linux x86_64" } as Device["host"] }]);
  expect(q('[data-peer-system="office"]')?.textContent).toBe("Linux");
  // This computer's own system is never said of another machine.
  expect(q('[data-peer-system="windows-pc"]')).toBeNull();
  await unmount();
});

it("lists no row for a machine that is still a helper device of this computer: its device's row is its one row", async () => {
  const unmoved = peer({ helper_device: "windows-pc" });
  const other = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook" });
  const { q, unmount } = await mount(idle, [unmoved, other], [], [{ id: "windows-pc", label: "Windows PC", kind: "remote", state: "ready", ssh_alias: "win" }]);
  expect(q('[data-peer-row="windows-pc"]')).toBeNull();
  expect(q('[data-peer-row="macbook"]')).not.toBeNull();
  await unmount();
});

it("names a machine a device was switched onto by that device's name, as the rail does (B36)", async () => {
  const switched = peer({ name: "DESKTOP-7Q2" });
  const { q, unmount } = await mount(idle, [switched], [], [{ id: "build-box", label: "빌드 서버", kind: "remote", state: "ready", peer_key: switched.key }]);
  expect(q('[data-peer-row="windows-pc"]')?.textContent).toContain("빌드 서버");
  expect(q('[data-peer-row="windows-pc"]')?.textContent).not.toContain("DESKTOP-7Q2");
  await unmount();
});

it("says the Tailscale tip only on a row connected through the relay, and keeps the path and direction in Connection details (D-86)", async () => {
  const relayed = peer({ key: "cd".repeat(32), device_id: "office", name: "Office Linux", path: "relay" });
  const direct = peer({ path: "tailscale" });
  const { q, press, unmount } = await mount(idle, [relayed, direct]);
  expect(q('[data-peer-relay-tip="office"]')?.textContent).toBe("Connected through the relay · turn on Tailscale on both computers and they connect directly, which is faster");
  expect(q('[data-peer-relay-tip="windows-pc"]')).toBeNull();
  await act(async () => { q('[data-peer-menu="windows-pc"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await press('[data-peer-details="windows-pc"]');
  const details = q('[data-peer-details-dialog="windows-pc"]');
  expect(details?.querySelector("[data-peer-route]")?.textContent).toBe("Tailscale");
  expect(details?.querySelector("[data-peer-direction]")?.textContent).toBe("this computer opens it");
  await unmount();
});

it("says on a relayed row why the SSH route waits, naming the machine and never the alias, and Retry sends the machine's retry (mission-107)", async () => {
  const parked = peer({ key: "cd".repeat(32), device_id: "office", name: "Office Linux", path: "relay", ssh: { blocked: "update_there" } });
  const { q, press, events, unmount } = await mount(idle, [parked]);
  const tip = q('[data-peer-relay-tip="office"]');
  expect(tip?.getAttribute("data-peer-ssh-blocked")).toBe("update_there");
  expect(tip?.textContent).toContain("Connected through the relay · update Hide on Office Linux and it connects over SSH");
  await press('[data-peer-ssh-retry="office"]');
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: parked.key } });
  await unmount();
});

it("renames a paired machine through its pairing, by its connection id (c2)", async () => {
  const { q, press, events, unmount } = await mount(idle, [peer()]);
  await act(async () => { q('[data-peer-menu="windows-pc"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await press('[data-peer-rename="windows-pc"]');
  const input = q('[data-device-rename-input="windows-pc"]') as HTMLInputElement;
  expect(input.value).toBe("Windows PC");
  expect((q('[data-device-rename-go="windows-pc"]') as HTMLButtonElement).disabled, "the same name saves nothing").toBe(true);
  await act(async () => {
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    set?.call(input, "  Office PC ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await press('[data-device-rename-go="windows-pc"]');
  expect(last(events)).toMatchObject({ kind: "pairing_rename", payload: { device: "ab".repeat(32), name: "Office PC" } });
  expect(q('[data-device-rename-dialog="windows-pc"]')).toBeNull();
  await unmount();
});

it("takes a chosen name back to the machine's own with an empty name, and not one it gives itself", async () => {
  const { q, press, events, unmount } = await mount(idle, [peer({ name: "회사 PC", named: true })]);
  await act(async () => { q('[data-peer-menu="windows-pc"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await press('[data-peer-rename="windows-pc"]');
  const input = q('[data-device-rename-input="windows-pc"]') as HTMLInputElement;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "  ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(q('[data-device-rename-go="windows-pc"]')?.hasAttribute("disabled")).toBe(false);
  await press('[data-device-rename-go="windows-pc"]');
  expect(last(events)).toMatchObject({ kind: "pairing_rename", payload: { device: "ab".repeat(32), name: "" } });
  await unmount();

  const own = await mount(idle, [peer()]);
  await act(async () => { own.q('[data-peer-menu="windows-pc"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await own.press('[data-peer-rename="windows-pc"]');
  const field = own.q('[data-device-rename-input="windows-pc"]') as HTMLInputElement;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "");
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(own.q('[data-device-rename-go="windows-pc"]')?.hasAttribute("disabled")).toBe(true);
  await own.unmount();
});

it("offers to update a machine too old to connect only from the computer that opens it (B22)", async () => {
  const older = peer({ connected: false, path: null, update: "there" });
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active", connected: false, path: null, update: "there" });
  const { q, press, events, unmount } = await mount({ ...idle, requests: [] }, [older, guest]);
  expect(q('[data-peer-row="windows-pc"]')?.textContent).toContain("Update needed on that computer");
  expect(q('[data-peer-update="windows-pc"]')?.textContent).toBe("Update Windows PC");
  // The computer that only opens this one is told, and asks for nothing.
  expect(q('[data-peer-row="macbook"]')?.textContent).toContain("Update needed on that computer");
  expect(q('[data-peer-update="macbook"]')).toBeNull();
  await press('[data-peer-update="windows-pc"]');
  expect(last(events)).toMatchObject({ kind: "pairing_update", payload: { device: older.key } });
  await unmount();
});

it("acts on the row that was pressed when two machines say one device id", async () => {
  const first = peer({ key: "11".repeat(32), name: "Office", connected: false, path: null });
  const second = peer({ key: "22".repeat(32), name: "Imposter", connected: false, path: null });
  const { events, unmount } = await mount(idle, [first, second]);
  const retries = document.body.querySelectorAll<HTMLElement>('[data-peer-retry="windows-pc"]');
  expect(retries).toHaveLength(2);
  await act(async () => retries[1]?.click());
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: second.key } });
  await act(async () => retries[0]?.click());
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: first.key } });
  await unmount();
});

it("lists the paired machines and offers nothing on a screen that is not this computer's own window (B52)", async () => {
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active" });
  const lost = peer({ connected: false, path: null });
  // hided sends such a screen no `pairing` frame.
  const { q, unmount } = await mount(null, [guest, lost]);
  expect(q('[data-peer-row="macbook"]')?.textContent).toContain("MacBook");
  expect(q('[data-peer-row="windows-pc"]')?.textContent).toContain("Not connected");
  expect(q("[data-pair-share-make]")).toBeNull();
  expect(document.body.querySelector("[data-peer-row] button")).toBeNull();
  await unmount();
});

it("asks before removing a machine that opens this computer, with its work left by default (B26)", async () => {
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active" });
  const { q, press, events, unmount } = await mount(idle, [guest]);
  // Radix opens the menu on Enter at its trigger.
  await act(async () => { q('[data-peer-menu="macbook"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await press('[data-peer-remove-incoming="macbook"]');
  const confirm = q('[data-peer-remove-confirm="macbook:incoming"]');
  expect(confirm?.textContent).toContain("What already ran here and the files it read cannot be undone.");
  // The work is left by default, and backing out is Cancel, never that choice's name.
  expect(q('[data-peer-remove-work="keep"]')?.textContent).toBe("The work it ran on this computerLeave it runningIt stays, and shows on this computer's screen only");
  expect([...(confirm?.querySelectorAll("button") ?? [])].map((button) => button.textContent)).toEqual(["Cancel", "Remove"]);
  expect(events.filter((event) => event.kind === "pairing_remove")).toHaveLength(0);
  await press("[data-peer-remove-go]");
  expect(last(events)).toMatchObject({ kind: "pairing_remove", payload: { device: guest.key, direction: "incoming" } });
  // With no terminal of that machine here there is nothing to end, and nothing says so.
  expect(last(events)?.payload).not.toHaveProperty("end_work");
  expect(q("[data-peer-remove-confirm]")).toBeNull();
  await unmount();
});

it("offers to end a removed machine's work only while it has terminals here, and sends that choice (B26, B60)", async () => {
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active" });
  const panes = [1, 2].map((n) => ({ id: `guest:macbook:pane:w1:p${n}`, pane: `w1:p${n}`, since_unix_ms: n }));
  const { q, press, events, unmount } = await mount(idle, [guest], [{ device_id: "macbook", key: guest.key, panes }]);
  await act(async () => { q('[data-peer-menu="macbook"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  await press('[data-peer-remove-incoming="macbook"]');
  // Leaving the work is still what a plain Remove does.
  expect(q("[data-peer-remove-work]")?.dataset.peerRemoveWork).toBe("keep");
  expect(q('[data-peer-remove-choice="end"]')?.textContent).toBe("End itIts 2 terminals here close now, even while that computer is away");
  await press('[data-peer-remove-choice="end"] [role="radio"]');
  expect(q("[data-peer-remove-work]")?.dataset.peerRemoveWork).toBe("end");
  await press("[data-peer-remove-go]");
  expect(last(events)).toMatchObject({ kind: "pairing_remove", payload: { device: guest.key, direction: "incoming", end_work: true } });
  await unmount();
});

it("says what came of a request and offers to ask again (B50)", async () => {
  const guest = peer({ device_id: "macbook", name: "MacBook", opens: null, opened_by: "active" });
  const { q, press, events, unmount } = await mount({ ...idle, requests: [{ key: guest.key, state: "declined" }] }, [guest]);
  expect(q("[data-peer-request]")?.dataset.peerRequest).toBe("declined");
  expect(q('[data-peer-request-go="macbook"]')?.textContent).toBe("Ask again");
  await press('[data-peer-request-go="macbook"]');
  expect(events.slice(-2).map((event) => event.kind)).toEqual(["pairing_dismiss_request", "pairing_request"]);
  await unmount();
});

it("shows the allow window with the words and what the machine would be allowed, and a close is a no (B2, D-76)", async () => {
  const ask = { id: 7, key: "cd".repeat(32), device: "macbook", name: "MacBook", kind: "link" as const, words: WORDS, expires_unix_ms: Date.now() + 120_000 };
  const { q, press, events, unmount } = await mount({ ...idle, asks: [ask] });
  expect(q("[data-pair-allow]")?.dataset.pairAllow).toBe("link");
  expect(q("[data-pair-words]")?.dataset.pairWords).toBe(WORDS.join(" "));
  expect(q("[data-pair-grants]")?.textContent).toBe("It can run commands and agents on this computer and see its screen · MacBook's Hide program and agent hooks are installed in this account");
  await press("[data-pair-allow-go]");
  expect(last(events)).toMatchObject({ kind: "pairing_answer", payload: { ask: 7, allow: true } });
  await press("[data-pair-refuse]");
  expect(last(events)).toMatchObject({ kind: "pairing_answer", payload: { ask: 7, allow: false } });
  await unmount();
});

it("asks a request over the connection without words (B50)", async () => {
  const ask = { id: 9, key: "cd".repeat(32), device: "windows-pc", name: "Windows PC", kind: "request" as const, words: null, expires_unix_ms: Date.now() + 120_000 };
  const { q, unmount } = await mount({ ...idle, asks: [ask] });
  expect(q("[data-pair-allow]")?.dataset.pairAllow).toBe("request");
  expect(q("[data-pair-words]")).toBeNull();
  expect(q("[data-pair-grants]")?.textContent).toContain("Windows PC's Hide program and agent hooks");
  await unmount();
});

it("shows the pasted link's words with They match and They differ, then how it ended (B2-B6)", async () => {
  const join = { id: 3, host: "ab".repeat(32), phase: "comparing" as const, failure: null, name: "Windows PC", words: WORDS, expires_unix_ms: null };
  const comparing = await mount({ ...idle, join });
  expect(comparing.q("[data-pair-join]")?.dataset.pairJoin).toBe("comparing");
  expect(comparing.q("[data-pair-words]")?.dataset.pairWords).toBe(WORDS.join(" "));
  await comparing.press("[data-pair-same]");
  expect(last(comparing.events)).toMatchObject({ kind: "pairing_compare", payload: { same: true } });
  await comparing.press("[data-pair-different]");
  expect(last(comparing.events)).toMatchObject({ kind: "pairing_compare", payload: { same: false } });
  await comparing.unmount();

  const full = await mount({ ...idle, join: { ...join, phase: "failed", failure: "host_full", words: null } });
  expect(full.q("[data-pair-join]")?.textContent).toContain("Too many devices are connected · remove one that is not used");
  await full.press("[data-pair-join-close]");
  expect(last(full.events)).toMatchObject({ kind: "pairing_dismiss_join" });
  await full.unmount();
});

/** A desktop host on Windows whose firewall rules answer `state` until an ask returns `answer`. */
function windowsHost(state: "allowed" | "missing" | "no_port", answer: "allowed" | "declined") {
  let now: "allowed" | "missing" | "no_port" = state;
  const host = {
    kind: "electron",
    platform: "win32",
    firewallState: vi.fn(async () => now),
    allowFirewall: vi.fn(async () => {
      if (answer === "allowed") now = "allowed";
      return answer;
    }),
  };
  window.hideHost = host as never;
  return host;
}

it("asks Windows once for the firewall rules when this PC is shared, and a no leaves the line with Allow (B32)", async () => {
  forgetFirewallAsk();
  const host = windowsHost("missing", "declined");
  const screen = await mount(idle);
  expect(screen.q("[data-pair-firewall]"), "nothing is shared or paired yet").toBeNull();
  await screen.press("[data-pair-share-make]");
  expect(host.allowFirewall).toHaveBeenCalledTimes(1);
  await screen.unmount();

  const shared = await mount({ ...idle, offer: { link: "hide://pair#v=1&id=ab&s=cd", expires_unix_ms: Date.now() + 60_000 } });
  await act(async () => {});
  expect(shared.q("[data-pair-firewall]")?.textContent).toContain("The firewall blocks direct connections");
  await shared.press("[data-pair-share-make]");
  await shared.press("[data-pair-share-end]");
  await shared.unmount();

  const again = await mount(idle);
  await again.press("[data-pair-share-make]");
  expect(host.allowFirewall, "a second share asks nothing on its own").toHaveBeenCalledTimes(1);
  await again.unmount();
  delete window.hideHost;
});

it("asks Windows on the first share before the folder has a port, since the ask chooses it (B32)", async () => {
  forgetFirewallAsk();
  const host = windowsHost("no_port", "allowed");
  const screen = await mount(idle);
  await screen.press("[data-pair-share-make]");
  expect(host.allowFirewall).toHaveBeenCalledTimes(1);
  await screen.unmount();
  delete window.hideHost;
});

it("asks Windows again from the line, and the line goes once the rules are in (B32)", async () => {
  forgetFirewallAsk();
  const host = windowsHost("missing", "allowed");
  const screen = await mount(idle, [peer()]);
  await act(async () => {});
  expect(screen.q("[data-pair-firewall]")).not.toBeNull();
  await screen.press("[data-pair-firewall-allow]");
  await act(async () => {});
  expect(host.allowFirewall).toHaveBeenCalledTimes(1);
  expect(screen.q("[data-pair-firewall]"), "the rules are in").toBeNull();
  await screen.unmount();
  delete window.hideHost;
});

it("shows no firewall line on another computer's screen of this one, and reads no rules there (B52)", async () => {
  forgetFirewallAsk();
  const host = windowsHost("missing", "declined");
  const screen = await mount(null, [peer()]);
  await act(async () => {});
  expect(screen.q("[data-pair-firewall]")).toBeNull();
  expect(host.firewallState).not.toHaveBeenCalled();
  await screen.unmount();
  delete window.hideHost;
});

it("shows no firewall line where the host keeps no rules", async () => {
  forgetFirewallAsk();
  const screen = await mount(idle, [peer()]);
  await act(async () => {});
  expect(screen.q("[data-pair-firewall]")).toBeNull();
  await screen.unmount();
});

it("keeps under a machine's row how many terminals its switch could not take over (B36)", async () => {
  const peers = [peer({ device_id: "build-box", name: "build-box" }), peer({ key: "ef".repeat(32), device_id: "laptop", name: "Laptop" })];
  const devices: Partial<Device>[] = [
    { id: "build-box", kind: "remote", not_carried: 2 },
    { id: "laptop", kind: "remote" },
  ];
  const { q, unmount } = await mount(idle, peers, [], devices);
  expect(q('[data-peer-row="build-box"] [data-device-not-carried="build-box:2"]')?.textContent).toBe("2 terminals could not be taken over · AI conversations can be resumed");
  expect(q('[data-peer-row="laptop"] [data-device-not-carried]')).toBeNull();
  await unmount();
});
