// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";

// The phone's socket to hided with only the browser's WebSocket and clock
// substituted: what a half-open socket to a sleeping Mac looks like from the
// page is a socket that stays open and says nothing.
class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static all: FakeSocket[] = [];
  readyState = FakeSocket.CONNECTING;
  sent: { type: string; id?: number }[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }
  send(data: string): void {
    this.sent.push(JSON.parse(data) as { type: string; id?: number });
  }
  close(): void {
    this.closed = true;
    this.readyState = FakeSocket.CLOSING;
  }
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }
  receive(frame: object): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
  pings(): number[] {
    return this.sent.filter((message) => message.type === "ping").map((message) => message.id ?? -1);
  }
}

const HELLO = { type: "hello", mac_name: "Mac", phone_id: "p", name: "iPhone", vapid_public_key: "k", notifications: "unasked" };

let visibility: "visible" | "hidden" = "visible";

function setVisibility(next: "visible" | "hidden"): void {
  visibility = next;
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  FakeSocket.all = [];
  visibility = "visible";
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", FakeSocket);
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
  window.localStorage.setItem("hide.phone.credential", "c".repeat(64));
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  window.localStorage.clear();
  vi.resetModules();
});

/** A fresh copy of the module (it keeps its socket in module state) with its store, started and greeted. */
async function connectedPhone() {
  vi.resetModules();
  const connection = await import("./connection");
  const { usePhone } = await import("./store");
  connection.start();
  const socket = FakeSocket.all[0]!;
  socket.open();
  socket.receive(HELLO);
  expect(usePhone.getState().connected).toBe(true);
  return { socket, usePhone };
}

it("asks a quiet hided after 20 s, gives the socket up 10 s later, shows the unreachable state and retries", async () => {
  const { socket, usePhone } = await connectedPhone();
  await vi.advanceTimersByTimeAsync(19_999);
  expect(socket.pings()).toEqual([]);
  await vi.advanceTimersByTimeAsync(1);
  expect(socket.pings()).toHaveLength(1);
  expect(usePhone.getState().connected).toBe(true);
  await vi.advanceTimersByTimeAsync(9_999);
  expect(usePhone.getState().connected).toBe(true);
  await vi.advanceTimersByTimeAsync(1);
  // Not waiting for the browser's close of a half-open socket.
  expect(socket.closed).toBe(true);
  expect(usePhone.getState().connected).toBe(false);
  expect(usePhone.getState().unreachable).toBe(true);
  expect(FakeSocket.all).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1_000);
  expect(FakeSocket.all).toHaveLength(2);
  // The abandoned socket closing late changes nothing about the new one.
  const next = FakeSocket.all[1]!;
  next.open();
  next.receive(HELLO);
  socket.onclose?.();
  expect(usePhone.getState().connected).toBe(true);
  expect(usePhone.getState().unreachable).toBe(false);
});

it("stays connected while hided answers, whether with a pong or any other frame", async () => {
  const { socket, usePhone } = await connectedPhone();
  let answered = 0;
  for (let second = 0; second < 150; second += 1) {
    await vi.advanceTimersByTimeAsync(1_000);
    const pings = socket.pings();
    if (pings.length > answered) {
      answered = pings.length;
      socket.receive(second % 2 === 0 ? { type: "pong", id: pings[pings.length - 1] } : { type: "meta", push_mode: "off", other_phones: 0 });
    }
  }
  expect(usePhone.getState().connected).toBe(true);
  expect(FakeSocket.all).toHaveLength(1);
  // 150 s of silence between answers costs one ping per 20 s of quiet, no more.
  expect(answered).toBeGreaterThanOrEqual(5);
  expect(answered).toBeLessThanOrEqual(8);
});

it("asks at once on returning to the foreground and does not ask a page nobody looks at", async () => {
  const { socket, usePhone } = await connectedPhone();
  setVisibility("hidden");
  await vi.advanceTimersByTimeAsync(120_000);
  expect(socket.pings()).toEqual([]);
  expect(usePhone.getState().connected).toBe(true);
  setVisibility("visible");
  expect(socket.pings()).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(9_999);
  expect(usePhone.getState().connected).toBe(true);
  await vi.advanceTimersByTimeAsync(1);
  expect(usePhone.getState().connected).toBe(false);
  expect(usePhone.getState().unreachable).toBe(true);
});

it("keeps a socket that answers the foreground check", async () => {
  const { socket, usePhone } = await connectedPhone();
  await vi.advanceTimersByTimeAsync(5_000);
  setVisibility("hidden");
  setVisibility("visible");
  const [id] = socket.pings();
  expect(id).toBeDefined();
  socket.receive({ type: "pong", id });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(usePhone.getState().connected).toBe(true);
  expect(socket.closed).toBe(false);
});

it("gives up a new socket that never says hello after 15 s and tries again", async () => {
  vi.resetModules();
  const connection = await import("./connection");
  const { usePhone } = await import("./store");
  connection.start();
  const socket = FakeSocket.all[0]!;
  await vi.advanceTimersByTimeAsync(14_999);
  expect(socket.closed).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  expect(socket.closed).toBe(true);
  expect(usePhone.getState().unreachable).toBe(true);
  await vi.advanceTimersByTimeAsync(1_000);
  expect(FakeSocket.all).toHaveLength(2);
});
