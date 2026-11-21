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
  const shell = connectShell({ onChunks: (_chunks, isFull) => full.push(isFull), onCopy: () => {} });
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
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
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

// What a node's daemon says of its own link once the core draws the screen
// (PRD device-connect-iroh B45) stays through the snapshots that follow, and
// goes when the link waits again, which is a new connection's try.
it("keeps why a node was not split while the core draws its screen, until its link waits again", () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "core_link", payload: { phase: "live", split_failed: "split_refused" } });
  first.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().coreSplit).toBe("split_refused");
  expect(useShellStore.getState().coreLink).toBeNull();
  first.frame({ type: "core_link", payload: { phase: "connecting", machine: null, alias: "mini" } });
  expect(useShellStore.getState().coreSplit).toBeNull();
  first.frame({ type: "core_link", payload: { phase: "live", split_failed: null } });
  expect(useShellStore.getState().coreSplit).toBeNull();
  shell.close();
});

// A node's held screen sends only where its link stands; the window turns
// live, and counts as connected, with the first snapshot the core draws.
it("a held node's page reports access when its first snapshot draws it", () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "core_link", payload: { phase: "waiting", machine: null, alias: "mini" } });
  expect(useShellStore.getState().connection).toBe("connecting");
  first.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  expect(first.sent.map((entry) => JSON.parse(entry)).filter((entry) => entry.kind === "product_analytics_connected")).toEqual([
    { schema_version: 2, kind: "product_analytics_connected", payload: {} },
  ]);
  shell.close();
});

// PRD device-connect-iroh B45: a machine whose core ran elsewhere becomes its
// own core in the same process. Its sockets close, and the page that
// reconnects reads the daemon's role and drops the node from its own address,
// so a reload opens this machine's own screen.
it("a page whose daemon became its machine's own core drops the node from its address", async () => {
  const { addressForRole } = await import("./ws");
  expect(addressForRole("#token=t&node=mbp", { role: "core" })).toBe("#token=t");
  expect(addressForRole("#token=t&node=mbp", { role: "node" })).toBeNull();
  expect(addressForRole("#token=t", { role: "core" })).toBeNull();
  // A health that names no role changes nothing.
  expect(addressForRole("#token=t&node=mbp", {})).toBeNull();

  window.location.hash = "#token=t&node=mbp";
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ role: "core" }) })));
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 3, rest: {} } });
  first.closeWith(1012, "role_ended");
  await Promise.resolve();
  await vi.runOnlyPendingTimersAsync();
  expect(useShellStore.getState().connection).toBe("reconnecting");
  expect(new URLSearchParams(window.location.hash.slice(1)).get("node")).toBeNull();
  expect(new URLSearchParams(window.location.hash.slice(1)).get("token")).toBe("t");
  shell.close();
});

/** The address changes under an open page, as the host's next attach or a followed link does. */
function addressNowCarries(token: string) {
  window.history.replaceState(null, "", `#token=${token}`);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}

const tokenOf = (socket: FakeSocket) => (JSON.parse(socket.sent[0] ?? "{}") as { token: string }).token;

// A restarted daemon has a new token. A page that reconnected with the old one
// is refused and gone (4001); the address that carries the new token, changed
// under the open page, draws it again without a reload.
it("a new token in the address draws a page the old token's refusal left gone", async () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  const first = made(0);
  first.open();
  first.frame({ type: "snapshot", payload: { revision: 5, rest: {} } });
  first.closeWith(1012, "");
  await vi.advanceTimersByTimeAsync(nextBackoff(BACKOFF_START_MS));
  const refused = made(1);
  refused.open();
  expect(tokenOf(refused)).toBe("t");
  refused.closeWith(4001, "invalid_token");
  expect(useShellStore.getState().connection).toBe("gone");

  addressNowCarries("t2");
  expect(FakeSocket.made).toHaveLength(3);
  const again = made(2);
  again.open();
  expect(tokenOf(again)).toBe("t2");
  again.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  expect(useShellStore.getState().connection).toBe("live");
  shell.close();
});

it("a token replacement fails its old reads and a late refusal leaves the new reads alone", async () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  configureFileBytes(shell.dispatch);
  const old = made(0);
  old.open();
  const outcome = (read: Promise<Uint8Array>) => read.then(() => "answered", (error: Error) => error.message);
  const oldRead = outcome(requestFileBytes("/repo/old.bin"));
  addressNowCarries("t2");
  expect(await oldRead).toBe("socket_closed");
  const again = made(1);
  again.open();
  again.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  const newRead = outcome(requestFileBytes("/repo/new.bin"));
  const request = JSON.parse(again.sent.at(-1)!) as { payload: { request_id: string } };
  old.closeWith(4001, "invalid_token");
  await vi.advanceTimersByTimeAsync(nextBackoff(BACKOFF_START_MS));
  expect(useShellStore.getState().connection).toBe("live");
  expect(FakeSocket.made).toHaveLength(2);
  again.frame({ type: "file_bytes_error", payload: { request_id: request.payload.request_id, reason: "read_failed" } });
  expect(await newRead).toBe("read_failed");
  shell.close();
});

it("a page whose address keeps its token does not open another connection", () => {
  const shell = connectShell({ onChunks: () => {}, onCopy: () => {} });
  made(0).open();
  addressNowCarries("t");
  expect(FakeSocket.made).toHaveLength(1);
  shell.close();
});

// with-hide/hide-private#659: a program's copy reaches the window as its own
// frame, handed over once, and never as a pane's output that a redraw or a
// reconnect would replay.
it("a clipboard frame is handed to the copy handler and never to the terminals", () => {
  const copies: unknown[] = [];
  const chunks: unknown[] = [];
  const shell = connectShell({ onChunks: (taken) => chunks.push(...taken), onCopy: (copy) => copies.push(copy) });
  const socket = made(0);
  socket.open();
  socket.frame({ type: "snapshot", payload: { revision: 1, rest: {} } });
  socket.frame({ type: "clipboard", payload: { pane_id: "remote:mini:pane:w1:p1", text_base64: "Y29waWVk" } });
  expect(copies).toEqual([{ pane_id: "remote:mini:pane:w1:p1", text_base64: "Y29waWVk" }]);
  expect(chunks).toEqual([]);
  shell.close();
});
