// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { PairingDialogs } from "../PairingDialogs";
import type { PairingView, Peer } from "../pairing";
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

async function mount(pairing: PairingView, peers: Peer[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({ connection: "live", pairing, pairingResult: null, rest: { navigator: { devices: [], peers }, ui_state: {} } } as never);
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
  expect(last(events)).toMatchObject({ kind: "pairing_retry", payload: { device: "windows-pc" } });
  // The machine that opens this one is asked to be opened too.
  expect(q('[data-peer-request-go="macbook"]')?.textContent).toBe("Open MacBook too");
  await press('[data-peer-request-go="macbook"]');
  expect(last(events)).toMatchObject({ kind: "pairing_request", payload: { device: "macbook" } });
  expect(q('[data-peer-request-go="windows-pc"]')).toBeNull();
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

it("shows no firewall line where the host keeps no rules", async () => {
  forgetFirewallAsk();
  const screen = await mount(idle, [peer()]);
  await act(async () => {});
  expect(screen.q("[data-pair-firewall]")).toBeNull();
  await screen.unmount();
});
