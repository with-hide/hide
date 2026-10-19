// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BACKOFF_START_MS, nextBackoff, type ConnectionState } from "./connection";
import { configureFileBytes, requestFileBytes } from "./fileBytes";
import { useShellStore } from "./store";
import { connectShell } from "./ws";

type Listener = (event: unknown) => void;

/** A server end the test drives: what the shell sent, and the events it hears. */
class FakeSocket {
  static readonly OPEN = 1;
  static made: FakeSocket[] = [];
  readyState = 0;
  binaryType = "";
  readonly sent: string[] = [];
  private readonly listeners = new Map<string, Listener[]>();

  constructor(readonly url: string) {
    FakeSocket.made.push(this);
  }

  addEventListener(type: string, listener: Listener) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  send(data: unknown) {
    this.sent.push(String(data));
  }

  close() {}

  open() {
    this.readyState = FakeSocket.OPEN;
    this.emit("open", {});
  }

  frame(frame: unknown) {
    this.emit("message", { data: JSON.stringify(frame) });
  }

  closeWith(code: number, reason: string) {
    this.readyState = 3;
    this.emit("close", { code, reason });
  }

  handshake(): { have_revision: number } {
    return JSON.parse(this.sent[0] ?? "{}") as { have_revision: number };
  }

  private emit(type: string, event: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

const saved = useShellStore.getState();

function made(index: number): FakeSocket {
  const socket = FakeSocket.made[index];
  if (!socket) throw new Error(`no socket ${index}`);
  return socket;
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSocket.made = [];
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true })));
  window.location.hash = "#token=t";
});

afterEach(() => {
  useShellStore.setState(saved, true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// R3, B17: a node closes a screen that fell behind with 1013
// `screen_fell_behind`. The shell treats it as any dropped socket: the read
// it waited for fails, it shows reconnecting (never gone) and reattaches at
// its first backoff from the revision it applied, and is drawn whole from
// the snapshot it is answered.
it("a screen closed as fallen behind fails its read and reattaches at the first attempt", async () => {
  const states: ConnectionState[] = [];
  const stop = useShellStore.subscribe((state) => states.push(state.connection));
  const full: boolean[] = [];
  const shell = connectShell({ onChunks: (_chunks, isFull) => full.push(isFull) });
  configureFileBytes(shell.dispatch);

  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 5, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  const read = requestFileBytes("/repo/big.bin");
  const failed = read.then(
    () => "answered",
    (error: Error) => error.message,
  );

  first.closeWith(1013, "screen_fell_behind");
  expect(await failed).toBe("socket_closed");
  const firstAttempt = nextBackoff(BACKOFF_START_MS);
  await vi.advanceTimersByTimeAsync(firstAttempt - 1);
  expect(FakeSocket.made).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(FakeSocket.made).toHaveLength(2);

  const again = made(1);
  again.open();
  expect(again.handshake().have_revision).toBe(5);
  again.frame({ type: "snapshot", payload: { revision: 9, rest: {} } });
  expect(full.at(-1)).toBe(true);
  expect(useShellStore.getState().revision).toBe(9);
  expect(useShellStore.getState().connection).toBe("live");
  expect(states).toContain("reconnecting");
  expect(states).not.toContain("gone");
  const accessEvents = [first, again].flatMap((socket) => socket.sent)
    .map((message) => JSON.parse(message) as { kind?: string; payload?: unknown })
    .filter((message) => message.kind === "product_analytics_connected");
  expect(accessEvents).toHaveLength(1);
  expect(accessEvents[0]?.payload).toEqual({});
  stop();
  shell.close();
});

// Q12 H1: a revision counts only on the core that numbered it. A page names
// the core of the daemon frame before the snapshot it applied, never one
// announced on a socket that died before its snapshot came.
it("a reconnect names the core its revision came from", async () => {
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "daemon", payload: { core_instance: "aaaaaaaaaaaaaaaa" } });
  first.frame({ type: "snapshot", payload: { revision: 5, rest: {} } });
  first.closeWith(1012, "");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const second = made(1);
  second.open();
  expect(JSON.parse(second.sent[0] ?? "{}")).toMatchObject({ have_revision: 5, have_core: "aaaaaaaaaaaaaaaa" });
  second.frame({ type: "daemon", payload: { core_instance: "bbbbbbbbbbbbbbbb" } });
  second.closeWith(1012, "");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const third = made(2);
  third.open();
  expect(JSON.parse(third.sent[0] ?? "{}")).toMatchObject({ have_revision: 5, have_core: "aaaaaaaaaaaaaaaa" });
  third.frame({ type: "daemon", payload: { core_instance: "bbbbbbbbbbbbbbbb" } });
  third.frame({ type: "snapshot", payload: { revision: 2, rest: {} } });
  third.closeWith(1012, "");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const fourth = made(3);
  fourth.open();
  expect(JSON.parse(fourth.sent[0] ?? "{}")).toMatchObject({ have_revision: 2, have_core: "bbbbbbbbbbbbbbbb" });
  shell.close();
});

// PRD core-host-node-move, Window handoff: a window goes through a move
// without a reload. Once the move is under way the role the window draws
// from stops and closes its socket, so the window is held as `moving` from
// that frame, through the close and the reconnect, and sends nothing but the
// move's own event (W1); the move's screen sends only the move; the screen
// closes when the role after the move takes over, and the window reconnects
// to that role, which draws it. The finished move names the node the window
// became, and the page writes it into its own address.
it("a move holds the window as moving, sends only the move, and the role after it draws the window at the node's address", async () => {
  const states: ConnectionState[] = [];
  const stop = useShellStore.subscribe((state) => states.push(state.connection));
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 3, rest: {} } });
  first.frame({ type: "core_move", payload: { state: "checking", direction: "forward", device: "mini", node: "mbp", sent: 0, total: 0, failed: [], step: "check", cause: null, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("live");
  first.frame({ type: "core_move", payload: { state: "stopping", direction: "forward", device: "mini", node: "mbp", sent: 0, total: 0, failed: [], step: "stop_core", cause: null, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("moving");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(false);
  first.closeWith(1012, "role_ended");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const moving = made(1);
  moving.open();
  moving.frame({ type: "core_move", payload: { state: "copying", direction: "forward", device: "mini", node: "mbp", sent: 1, total: 2, failed: [], step: "copy", cause: null, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("moving");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(false);
  expect(moving.sent).toHaveLength(1);
  // Giving up a wait is the one event the move's screen takes (amendment 4).
  expect(shell.dispatch({ schema_version: 2, kind: "core_move", payload: { action: "abandon" } })).toBe(true);
  expect(JSON.parse(moving.sent[1] ?? "{}")).toMatchObject({ kind: "core_move", payload: { action: "abandon" } });
  moving.frame({ type: "error", payload: { kind: "core_move", reason: "moving" }, message: "moving" });
  expect(useShellStore.getState().connection).toBe("moving");
  moving.frame({ type: "core_move", payload: { state: "done", direction: "forward", device: "mini", node: "mbp", sent: 2, total: 2, failed: [], step: "reattach", cause: null, intent: "i" } });
  expect(new URLSearchParams(window.location.hash.slice(1)).get("node")).toBe("mbp");
  expect(new URLSearchParams(window.location.hash.slice(1)).get("token")).toBe("t");
  moving.closeWith(1012, "core_moved");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const node = made(2);
  node.open();
  node.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  // Held from the stopping frame until the node draws: no reconnect between.
  const from = states.indexOf("moving");
  expect(states.slice(from, states.lastIndexOf("moving") + 1).every((state) => state === "moving")).toBe(true);
  expect(states.slice(from)).not.toContain("reconnecting");
  expect(states).not.toContain("gone");
  stop();
  shell.close();
});

// The core keeps sending state between the move's stopping frame and its
// socket's close, and the node draws the window before the move is done:
// neither releases a window the move holds, so no key reaches a role that is
// stopping or a move that has not finished (W1).
it("a frame a role sends while the move is under way keeps the window held", async () => {
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 3, rest: {} } });
  first.frame({ type: "core_move", payload: { state: "stopping", direction: "forward", device: "mini", node: "mbp", sent: 0, total: 0, failed: [], step: "stop_core", cause: null, intent: "i" } });
  first.frame({ type: "delta", payload: { revision: 4, rest: {} } });
  expect(useShellStore.getState().connection).toBe("moving");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(false);
  first.closeWith(1012, "role_ended");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const node = made(1);
  node.open();
  node.frame({ type: "core_move", payload: { state: "linking", direction: "forward", device: "mini", node: "mbp", sent: 2, total: 2, failed: [], step: "reattach", cause: null, intent: "i" } });
  node.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("moving");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(false);
  node.frame({ type: "core_move", payload: { state: "done", direction: "forward", device: "mini", node: "mbp", sent: 2, total: 2, failed: [], step: "reattach", cause: null, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("live");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(true);
  shell.close();
});

it("a new held page reports access when its first snapshot releases the hold", () => {
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "core_link", payload: { phase: "waiting", machine: null, alias: "mini" } });
  first.frame({ type: "core_move", payload: { state: "idle", direction: null, device: null, node: null, sent: 0, total: 0, failed: [], step: null, cause: null, intent: null } });
  expect(useShellStore.getState().connection).toBe("moving");
  first.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  expect(first.sent.map((entry) => JSON.parse(entry)).filter((entry) => entry.kind === "product_analytics_connected")).toEqual([
    { schema_version: 2, kind: "product_analytics_connected", payload: {} },
  ]);
  shell.close();
});

it("a move back removes the node from the address, and an unfinished move leaves it", async () => {
  const { addressAfterMove } = await import("./ws");
  const view = (state: string, direction: "forward" | "back", node: string | null) => ({ state, direction, node, device: "mini", sent: 0, total: 0, failed: [], step: null, cause: null, intent: "i" }) as never;
  expect(addressAfterMove("#token=t&node=mbp", view("done", "back", null))).toBe("#token=t");
  expect(addressAfterMove("#token=t", view("done", "forward", "mbp"))).toBe("#token=t&node=mbp");
  expect(addressAfterMove("#token=t", view("rolled_back", "forward", "mbp"))).toBeNull();
  expect(addressAfterMove("#token=t&node=mbp", view("done", "forward", "mbp"))).toBeNull();
});

// Every role sends its move frame before the first snapshot: an idle one
// holds no window, so a fresh window goes from connecting to live; a node
// that holds its screens (`core_link`) does hold a window not drawn yet.
it("an idle move frame before the first snapshot holds nothing, and a node holding its screens does", async () => {
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  const idle = { state: "idle", direction: null, device: null, node: null, sent: 0, total: 0, failed: [], step: null, cause: null, intent: null };
  first.frame({ type: "core_move", payload: idle });
  expect(useShellStore.getState().connection).not.toBe("moving");
  first.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  first.closeWith(1006, "");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();

  const held = made(1);
  held.open();
  held.frame({ type: "core_link", payload: { phase: "waiting", machine: null, alias: "mini" } });
  held.frame({ type: "core_move", payload: idle });
  expect(useShellStore.getState().connection).toBe("moving");
  expect(shell.dispatch({ schema_version: 2, kind: "pane_input", payload: {} })).toBe(false);
  held.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  shell.close();
});

// A move that is not under way leaves the window alone: a dropped socket
// shows reconnecting, and a drawn window a frame held goes back to live when
// the move turns out not to go ahead.
it("a window no move holds reconnects as reconnecting, and a move that stops short releases it", async () => {
  const shell = connectShell({ onChunks: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 3, rest: {} } });
  first.frame({ type: "core_move", payload: { state: "waiting", direction: "forward", device: "mini", node: null, sent: 0, total: 0, failed: [], step: "check", cause: null, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("moving");
  first.frame({ type: "core_move", payload: { state: "rolled_back", direction: "forward", device: "mini", node: null, sent: 0, total: 0, failed: [], step: "check", cause: { kind: "refused" }, intent: "i" } });
  expect(useShellStore.getState().connection).toBe("live");
  first.closeWith(1006, "");
  expect(useShellStore.getState().connection).toBe("reconnecting");
  shell.close();
});
