// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ListScreen } from "./App";
import type { ConnectableHost, Watch } from "./Gathered";
import { HOSTS_KEY } from "./hosts";
import type { Presence } from "./presence";
import type { AgentGroup, PhoneAgent } from "./protocol";
import { patch, usePhone } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const MAC = { room: "a".repeat(43), name: "MacBook" };
const MINI = { room: "b".repeat(43), name: "맥미니" };
const PC = { room: "c".repeat(43), name: "Windows PC" };
const BOX = { room: "d".repeat(43), name: "build-box" };

const initial = usePhone.getState();
let container: HTMLDivElement;
let root: Root;

/** The saved pairings exactly as a phone before the gathered view stored them (`hide.phone.hosts.v1`), which the new app reads as its computers (D-57). */
function pair(...hosts: { room: string; name: string }[]): void {
  localStorage.setItem(HOSTS_KEY, JSON.stringify(hosts.map((host) => ({ ...host, noiseKey: "k".repeat(43), credential: `credential-${host.name}`, protocol: 5, lastSeen: null }))));
}

function agent(device: string, pane: string, changed: number, over: Partial<PhoneAgent> = {}): { kind: "agent" } & PhoneAgent {
  return { kind: "agent", device_id: device, pane_id: pane, root_pane_id: pane, group: "needs_you", mark: "turn", wait: null, descendants_working: 0, agent_kind: "claude", title: pane, place: "shop-api", device_label: null, changed_at_unix_ms: changed, line: null, status_code: "waiting", demand: "none", emphasized: true, holds_notification: true, ...over };
}

/** A stand-in watcher the test drives: which computers are watched now, and a way to say what each one did. */
function watcher() {
  const watched = new Map<string, (presence: Presence) => void>();
  const opened: string[] = [];
  const watch: Watch = (host: ConnectableHost, changed) => {
    opened.push(host.name);
    watched.set(host.room, changed);
    return () => { watched.delete(host.room); };
  };
  const tell = async (room: string, presence: Presence) => { await act(async () => watched.get(room)!(presence)); };
  return { watch, watched, opened, tell };
}

async function render(watch: Watch): Promise<void> {
  await act(async () => root.render(<ListScreen hosted gathered={{ room: MAC.room, watch }} />));
  // The saved computers are read from storage after the first paint.
  await act(async () => { await Promise.resolve(); });
}

const assign = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("location", { ...window.location, pathname: "/p/5/", search: `?host=${MAC.room}`, hash: "", assign });
  container = document.createElement("div");
  root = createRoot(container);
  const groups: AgentGroup[] = [
    { group: "needs_you", count: 1, rows: [agent("mac", "mac-ask", 100)] },
    { group: "working", count: 1, rows: [agent("mac", "mac-work", 300, { group: "working", mark: "quiet" })] },
  ];
  patch({ screen: "app", connected: true, groups, devices: [], listScope: "all" });
});
afterEach(async () => {
  await act(async () => root.unmount());
  usePhone.setState(initial, true);
  localStorage.clear();
  assign.mockReset();
  vi.unstubAllGlobals();
});

const rows = () => [...container.querySelectorAll<HTMLElement>("[data-phone-gathered]")];
const chip = (row: HTMLElement) => row.querySelector("[data-phone-computer]")?.getAttribute("data-phone-computer");
const line = (room: string) => container.querySelector(`[data-phone-host-room="${room}"]`);

// PRD device-connect-iroh B55, B62, D-69.
it("gathers every computer's 내 차례 with its mark, and gives each other computer its own line", async () => {
  pair(MAC, MINI, PC, BOX);
  const { watch, tell } = watcher();
  await render(watch);
  expect([...container.querySelectorAll("[data-phone-computer-choice]")].map((item) => item.textContent)).toEqual(["모두", "MacBook", "맥미니", "Windows PC", "build-box"].map((name) => (name === "모두" ? expect.any(String) : name)));
  expect(container.querySelector("[data-phone-computers]")!.getAttribute("data-phone-computers")).toBe("all");
  // Before any other computer answers, this computer's rows are there and each other one is connecting: nobody waits for all.
  expect(rows().map(chip)).toEqual(["MacBook"]);
  expect(line(MINI.room)!.getAttribute("data-phone-host-line")).toBe("connecting");
  // 작업 중 is this computer's list alone.
  expect(container.querySelector('[data-phone-agent="mac|mac-work"]')).toBeNull();

  await tell(MINI.room, { online: true, lastSeen: 1, gathered: [{ group: "needs_you", count: 1, rows: [agent("mini", "mini-ask", 200)] }, { group: "done", count: 1, rows: [agent("mini", "mini-done", 50, { group: "done", mark: "done" })] }] });
  await tell(BOX.room, { online: false, lastSeen: null });
  await tell(PC.room, { online: false, lastSeen: null, protocolMismatch: 6 });

  const needs = container.querySelector('[data-phone-group="needs_you"]')!;
  expect(needs.querySelector("[data-phone-group-count]")!.textContent).toBe("2");
  expect([...needs.querySelectorAll<HTMLElement>("[data-phone-gathered]")].map((row) => [row.querySelector("[data-phone-agent]")!.getAttribute("data-phone-agent"), chip(row)]))
    .toEqual([["mini|mini-ask", "맥미니"], ["mac|mac-ask", "MacBook"]]);
  expect(container.querySelector('[data-phone-group="done"] [data-phone-agent="mini|mini-done"]')).not.toBeNull();
  expect(line(MINI.room)).toBeNull();
  expect(line(BOX.room)!.getAttribute("data-phone-host-line")).toBe("offline");
  expect(line(PC.room)!.getAttribute("data-phone-host-line")).toBe("update");
  expect(line(PC.room)!.textContent).toContain("Windows PC");

  // Another computer's row opens that computer on that agent, as its notification would.
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="mini|mini-ask"]')!.click());
  expect(assign).toHaveBeenCalledWith(`/?host=${MINI.room}&open=${encodeURIComponent("mini|mini-ask")}`);

  // A computer that goes away takes its rows with it (D-50) and says so.
  await tell(MINI.room, { online: false, lastSeen: 2 });
  expect(rows().map(chip)).toEqual(["MacBook"]);
  expect(line(MINI.room)!.getAttribute("data-phone-host-line")).toBe("offline");
});

// B62: other computers are attached only while the gathered view is on screen.
it("watches the other computers only while 모두 is on screen and the page is visible", async () => {
  pair(MAC, MINI);
  const { watch, watched, opened } = watcher();
  await render(watch);
  expect([...watched.keys()]).toEqual([MINI.room]);
  expect(opened).toEqual(["맥미니"]);

  // This computer's chip shows its own list and lets the other one go.
  await act(async () => container.querySelector<HTMLElement>(`[data-phone-computer-choice="${MAC.room}"]`)!.click());
  expect(usePhone.getState().listScope).toBe("one");
  expect(watched.size).toBe(0);
  expect(container.querySelector('[data-phone-agent="mac|mac-work"]')).not.toBeNull();

  await act(async () => container.querySelector<HTMLElement>('[data-phone-computer-choice="all"]')!.click());
  expect([...watched.keys()]).toEqual([MINI.room]);

  // A hidden page holds no connection to another computer, and one comes back with the page.
  Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(watched.size).toBe(0);
  Object.defineProperty(document, "hidden", { configurable: true, get: () => false });
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect([...watched.keys()]).toEqual([MINI.room]);

  // Another computer's chip opens that computer's own list.
  await act(async () => container.querySelector<HTMLElement>(`[data-phone-computer-choice="${MINI.room}"]`)!.click());
  expect(assign).toHaveBeenCalledWith(`/?host=${MINI.room}&one=1`);
});

// B56, D-57: a phone paired with one computer opens as it did, with no computer row and no other connection.
it("keeps a phone paired with one computer on that computer's own list", async () => {
  pair(MAC);
  const { watch, opened } = watcher();
  await render(watch);
  expect(container.querySelector("[data-phone-computers]")).toBeNull();
  expect(container.querySelector('[data-phone-agent="mac|mac-work"]')).not.toBeNull();
  expect(opened).toEqual([]);
});
