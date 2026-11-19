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
import { PairedRows } from "./PairingRows";

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
    useShellStore.setState({ connection: "live", pairing, pairingResult: null, rest: { navigator: { devices, peers, guest_panes }, ui_state: {} } } as never);
    root.render(
      <TooltipProvider>
        <PairedRows actions={actions} />
        <PairingDialogs actions={actions} />
      </TooltipProvider>,
    );
  });
  const q = (selector: string) => document.body.querySelector(selector) as HTMLElement | null;
  const press = async (selector: string) => act(async () => q(selector)?.click());
  return { events, q, press, unmount: async () => { await act(async () => root.unmount()); useShellStore.setState(saved, true); } };
}

const last = (events: Parameters<DispatchFn>[0][]) => events.at(-1);

it("makes a link on a press and shows it with the time it has left (B1)", async () => {
  vi.useFakeTimers({ now: 19_000, toFake: ["Date"] });
  const idleScreen = await mount(idle);
  expect(idleScreen.q("[data-pair-share]")?.dataset.pairShare).toBe("idle");
  await idleScreen.press("[data-pair-share-make]");
  expect(last(idleScreen.events)).toMatchObject({ kind: "pairing_share", payload: { language: "en" } });
  await idleScreen.unmount();

  const shared = await mount({ ...idle, offer: { link: "hide://pair#v=1&id=ab&s=cd", expires_unix_ms: 9 * 60_000 } });
  expect(shared.q("[data-pair-link]")?.textContent).toBe("hide://pair#v=1&id=ab&s=cd");
  expect(shared.q("[data-pair-share]")?.textContent).toContain("8:41 left · works once");
  expect(shared.q("[data-pair-share-make]")).toBeNull();
  await shared.press("[data-pair-share-end]");
  expect(last(shared.events)).toMatchObject({ kind: "pairing_end_share" });
  await shared.unmount();
});

it("lists a paired machine with its state, its direction and the one action its state offers (B21, B24, B50)", async () => {
  const guest = peer({ key: "cd".repeat(32), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active", path: "relay" });
  const lost = peer({ connected: false, path: null, reason: "relay_unreachable" });
  const { q, press, events, unmount } = await mount({ ...idle, requests: [] }, [guest, lost]);
  const row = (id: string) => q(`[data-peer-row="${id}"]`);
  expect(row("macbook")?.textContent).toContain("MacBook");
  expect(row("macbook")?.textContent).toContain("Relay");
  expect(row("macbook")?.querySelector("[data-peer-direction]")?.textContent).toBe("it opens this computer");
  expect(row("windows-pc")?.textContent).toContain("Not connected · the relay cannot be reached");
  await press('[data-peer-retry="windows-pc"]');
  // An ask names the machine by its connection id, never by the device id it says of itself.
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: lost.key } });
  // The machine that opens this one is asked to be opened too.
  expect(q('[data-peer-request-go="macbook"]')?.textContent).toBe("Open MacBook too");
  await press('[data-peer-request-go="macbook"]');
  expect(last(events)).toMatchObject({ kind: "pairing_request", payload: { device: guest.key } });
  expect(q('[data-peer-request-go="windows-pc"]')).toBeNull();
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
  expect(q("[data-pair-share]")).toBeNull();
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

it("says under a machine's row what the Hide started there over SSH still needs, with its command (B30)", async () => {
  const peers = [peer({ device_id: "build-box", name: "build-box" }), peer({ key: "cd".repeat(32), device_id: "mac-mini", name: "Mac mini" }), peer({ key: "ef".repeat(32), device_id: "laptop", name: "Laptop" })];
  const devices: Partial<Device>[] = [
    { id: "build-box", kind: "remote", needs: { kind: "linger", command: "sudo loginctl enable-linger hy" } },
    { id: "mac-mini", kind: "remote", needs: { kind: "auto_login" } },
    { id: "laptop", kind: "remote" },
  ];
  const { q, unmount } = await mount(idle, peers, [], devices);
  const linger = q('[data-peer-row="build-box"] [data-device-needs="build-box:linger"]');
  expect(linger?.textContent).toContain("Needs sudo");
  expect(linger?.querySelector("code")?.textContent).toBe("sudo loginctl enable-linger hy");
  expect(q('[data-peer-row="mac-mini"] [data-device-needs="mac-mini:auto_login"]')?.textContent).toBe("Turn on automatic login there to stay connected after a restart");
  // A machine whose Hide needs nothing has no line.
  expect(q('[data-peer-row="laptop"] [data-device-needs]')).toBeNull();
  await unmount();
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
