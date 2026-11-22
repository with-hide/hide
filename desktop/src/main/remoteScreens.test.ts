import { once } from "node:events";
import type { AddressInfo } from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import WebSocket, { WebSocketServer } from "ws";
import { REMOTE_VIEWER_CHANNEL, REMOTE_SOCKET_EVENT_CHANNEL, REMOTE_SOCKET_OPEN_CHANNEL, REMOTE_SOCKET_SEND_CHANNEL, REMOTE_WORK_CHANNEL, REMOTE_WORK_DONE_CHANNEL, REMOTE_WORK_RESULT_CHANNEL } from "../channel";
import type { HostLog } from "./log";
import { MAX_SEND_BYTES, pageAddress } from "./remotePage";
import { HAND_WITHIN_MS } from "../../../web/src/remoteView";
import { RemoteScreens } from "./remoteScreens";

const ipc = vi.hoisted(() => new Map<string, (event: unknown, value?: unknown) => void>());
/** What each `new WebContentsView()` gave the host, newest last. */
const views = vi.hoisted(() => [] as { webContents: { send: (channel: string, value: unknown) => void; loaded: string[]; closed: boolean; handlers: Map<string, (...args: unknown[]) => void> } }[]);
/** Each session partition the host asked for, in order. */
const partitions = vi.hoisted(() => [] as string[]);
/** What the host sent the shell's own window, in order. */
const shellSent = vi.hoisted(() => [] as { channel: string; value: unknown }[]);
/** Ends each page load the host started, in order. */
const loads = vi.hoisted(() => [] as (() => void)[]);
vi.mock("electron", () => {
  const fakeSession = {
    protocol: { handle: () => undefined },
    setPermissionRequestHandler: () => undefined,
    setPermissionCheckHandler: () => undefined,
    webRequest: { onBeforeRequest: () => undefined },
    on: () => undefined,
  };
  return {
    ipcMain: { on: (channel: string, listener: (event: unknown, value?: unknown) => void) => ipc.set(channel, listener), handle: () => undefined },
    clipboard: { writeText: () => undefined },
    protocol: {},
    session: {
      fromPartition: (partition: string) => {
        partitions.push(partition);
        return fakeSession;
      },
    },
    WebContentsView: function WebContentsView() {
      const view = {
        setBackgroundColor: () => undefined,
        setVisible: () => undefined,
        setBounds: () => undefined,
        webContents: {
          sent: [] as { channel: string; value: unknown }[],
          loaded: [] as string[],
          closed: false,
          send(channel: string, value: unknown) {
            this.sent.push({ channel, value });
          },
          loadURL(url: string) {
            this.loaded.push(url);
            // A load ends only when a test ends it (`loads`).
            return new Promise<void>((resolve) => loads.push(resolve));
          },
          close() {
            this.closed = true;
          },
          isDestroyed() {
            return this.closed;
          },
          isFocused: () => false,
          focus: () => undefined,
          handlers: new Map<string, (...args: unknown[]) => void>(),
          on(event: string, handler: (...args: unknown[]) => void) {
            this.handlers.set(event, handler);
          },
          setWindowOpenHandler: () => undefined,
        },
      };
      views.push(view);
      return view;
    },
  };
});

const DEVICE = "b2b2b2b2-0000-4000-8000-0000000000bb";
const KEY = "a".repeat(64);
const REPLACEMENT = "b".repeat(64);
const RECT = { x: 0, y: 0, width: 800, height: 600 };

/** The page's socket events as the host sent them to the screen's page. */
function pageEvents(view: (typeof views)[number]): { op: string; code?: number; reason?: string }[] {
  const contents = view.webContents as unknown as { sent: { channel: string; value: { op: string; code?: number; reason?: string } }[] };
  return contents.sent.filter((sent) => sent.channel === REMOTE_SOCKET_EVENT_CHANNEL).map((sent) => sent.value);
}

async function until(what: string, ready: () => boolean): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!ready()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

let server: WebSocketServer | null = null;
afterEach(async () => {
  for (const client of server?.clients ?? []) client.terminate();
  await new Promise((resolve) => server?.close(resolve) ?? resolve(undefined));
  server = null;
  ipc.clear();
  views.length = 0;
  partitions.length = 0;
  loads.length = 0;
  shellSent.length = 0;
});

/** A host with the fakes above, its daemon at `server`'s port. */
function screensOn(port: number, log: { event: ReturnType<typeof vi.fn> }, now?: () => number, language: () => "ko" | "ja" = () => "ko"): RemoteScreens {
  const window = {
    isDestroyed: () => false,
    isFocused: () => false,
    webContents: { getZoomFactor: () => 1, focus: () => undefined, send: (channel: string, value: unknown) => shellSent.push({ channel, value }) },
    contentView: { addChildView: () => undefined, removeChildView: () => undefined },
  };
  const screens = new RemoteScreens({
    window: () => window as never,
    daemon: () => ({ origin: `http://127.0.0.1:${port}`, token: "t" }),
    registry: () => [],
    background: "#000000",
    language,
    log: log as unknown as HostLog,
    now,
  });
  screens.listen();
  return screens;
}

/** What the screen's page would send: its own top frame on its own origin. */
function fromPage(view: (typeof views)[number]) {
  return { sender: view.webContents, senderFrame: { parent: null, url: pageAddress(DEVICE) } };
}

describe("a remote screen's socket", () => {
  // PRD device-connect-iroh D-67: a host that stops reading never grows this
  // process's queue past one largest frame and 8 MiB; the socket is cut at
  // once and the page is told, so it connects again.
  it("closes at once, queue_full, when the host stops reading", async () => {
    server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
    await once(server, "listening");
    const held = new Promise<WebSocket>((resolve) => server!.once("connection", (socket) => resolve(socket)));
    const log = { event: vi.fn() };
    const screens = screensOn((server.address() as AddressInfo).port, log);
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    const view = views.at(-1)!;
    const page = fromPage(view);

    ipc.get(REMOTE_SOCKET_OPEN_CHANNEL)!(page);
    const host = await held;
    // The host reads nothing more: what the page sends stays queued here.
    (host as unknown as { _socket: { pause(): void } })._socket.pause();
    await until("the page told the socket is open", () => pageEvents(view).some((event) => event.op === "open"));
    const ended = new Promise<number>((resolve) => host.once("close", (code) => resolve(code)));

    const frame = new Uint8Array(MAX_SEND_BYTES);
    for (let sent = 0; sent < 4 && !pageEvents(view).some((event) => event.op === "close"); sent += 1) {
      ipc.get(REMOTE_SOCKET_SEND_CHANNEL)!(page, frame);
    }

    expect(pageEvents(view).filter((event) => event.op === "close")).toEqual([{ op: "close", code: 1013, reason: "queue_full" }]);
    expect(log.event).toHaveBeenCalledWith("remote.send_refused", { device: DEVICE, reason: "queue_full" });
    // The socket was cut, not closed politely behind its queue: the host, reading again, sees no close
    // frame after the queued bytes, only the connection gone (1006).
    (host as unknown as { _socket: { resume(): void } })._socket.resume();
    expect(await ended).toBe(1006);
    // A send after the cut reaches no socket.
    ipc.get(REMOTE_SOCKET_SEND_CHANNEL)!(page, new Uint8Array(1));
    expect(pageEvents(view).filter((event) => event.op === "close")).toHaveLength(1);
  }, 30_000);

  // Security pre-review S1: a page is the screen of the key its machine was
  // paired under when it loaded. When another host takes the same device id
  // under a new key, the shell shows that device under the new key: the
  // earlier page is closed, with nothing it stored, before the new host's
  // page loads, and only the new page's socket opens, naming the new key.
  it("closes a page loaded under one key when its device is shown under another, before the other's socket opens", async () => {
    server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
    await once(server, "listening");
    const asked: string[] = [];
    server.on("connection", (_socket, request) => asked.push(request.url ?? ""));
    const log = { event: vi.fn() };
    const screens = screensOn((server.address() as AddressInfo).port, log);

    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    const earlier = views.at(-1)!;
    ipc.get(REMOTE_SOCKET_OPEN_CHANNEL)!(fromPage(earlier));
    await until("the first key's socket", () => asked.length === 1);
    expect(asked).toEqual([`/peer-screen/${DEVICE}/ws?key=${KEY}`]);
    screens.show(null);

    screens.show({ device: DEVICE, key: REPLACEMENT, rect: RECT, name: null });
    const later = views.at(-1)!;
    expect(later).not.toBe(earlier);
    expect(earlier.webContents.closed).toBe(true);
    expect(later.webContents.loaded).toEqual([pageAddress(DEVICE)]);
    expect(new Set(partitions).size).toBe(2);
    expect(log.event).toHaveBeenCalledWith("remote.key_changed", { device: DEVICE });

    // The earlier page asks for a socket again and reaches nothing; the later page's names its own key.
    ipc.get(REMOTE_SOCKET_OPEN_CHANNEL)!(fromPage(earlier));
    ipc.get(REMOTE_SOCKET_OPEN_CHANNEL)!(fromPage(later));
    await until("the replacement key's socket", () => asked.length >= 2);
    expect(asked).toEqual([`/peer-screen/${DEVICE}/ws?key=${KEY}`, `/peer-screen/${DEVICE}/ws?key=${REPLACEMENT}`]);
    expect(pageEvents(earlier).filter((event) => event.op === "open")).toHaveLength(1);
  }, 30_000);
});

/** The asks the host handed the screen's page. */
function handed(view: (typeof views)[number]): unknown[] {
  const contents = view.webContents as unknown as { sent: { channel: string; value: unknown }[] };
  return contents.sent.filter((sent) => sent.channel === REMOTE_WORK_CHANNEL).map((sent) => sent.value);
}

async function loaded(count: number): Promise<void> {
  await until("the page's load", () => loads.length >= count);
  loads[count - 1]!();
  // The load's answer runs on the next turn.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

/** How the host told the shell its starts ended. */
const answers = () => shellSent.filter((sent) => sent.channel === REMOTE_WORK_RESULT_CHANNEL).map((sent) => sent.value);
const start = (request: string) => ({ op: "start", request, place: { checkout: "/home/me/app" }, provider: "claude", model: null, prompt: null, profile: null }) as const;

// PRD device-connect-iroh D-73, D-78: the operator's ask of another
// computer's screen reaches its page once, after the page loaded, for the
// key it was asked under and while the operator still waits on it.
describe("an ask of a remote screen", () => {
  const work = { op: "show", pane: "w1:p2" } as const;

  it("reaches the page once it loaded, and at once when it already has", async () => {
    const screens = screensOn(9, { event: vi.fn() });
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work });
    const view = views.at(-1)!;
    expect(handed(view)).toEqual([]);
    await loaded(1);
    expect(handed(view)).toEqual([work]);

    const start = { op: "start", request: "r-1", place: { checkout: "/home/me/app" }, provider: "claude", model: null, prompt: "fix it", profile: null } as const;
    screens.go({ device: DEVICE, key: KEY, work: start });
    expect(handed(view)).toEqual([work, start]);
  });

  it("tells a loaded page what the window calls its machine and the window's language, and again when either changes", async () => {
    let language: "ko" | "ja" = "ko";
    const screens = screensOn(9, { event: vi.fn() }, undefined, () => language);
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: "build-box" });
    const view = views.at(-1)!;
    const said = () => (view.webContents as unknown as { sent: { channel: string; value: unknown }[] }).sent.filter((sent) => sent.channel === REMOTE_VIEWER_CHANNEL).map((sent) => sent.value);
    expect(said()).toEqual([]);
    await loaded(1);
    expect(said()).toEqual([{ language: "ko", name: "build-box" }]);
    language = "ja";
    screens.languageChanged();
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: "build-box" });
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: "Build box" });
    expect(said()).toEqual([{ language: "ko", name: "build-box" }, { language: "ja", name: "build-box" }, { language: "ja", name: "Build box" }]);
  });

  it("drops an ask made under another key or that waited past its bound", async () => {
    let clock = 1_000;
    const log = { event: vi.fn() };
    const screens = screensOn(9, log, () => clock);
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: REPLACEMENT, work });
    await loaded(1);
    expect(handed(views.at(-1)!)).toEqual([]);
    expect(log.event).toHaveBeenCalledWith("remote.work_dropped", { device: DEVICE, reason: "key_changed" });

    screens.closeAll();
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work });
    clock += HAND_WITHIN_MS + 1;
    await loaded(2);
    expect(handed(views.at(-1)!)).toEqual([]);
    expect(log.event).toHaveBeenCalledWith("remote.work_dropped", { device: DEVICE, reason: "late" });
  });

  // A start that never reached its page is answered to the shell, so the
  // window that handed it gives the operator's words back.
  it("answers the shell for each start that never reached its page", async () => {
    const screens = screensOn(9, { event: vi.fn() });
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: REPLACEMENT, work: start("r-1") });
    await loaded(1);
    expect(answers()).toEqual([{ device: DEVICE, request: "r-1", outcome: "not_shown" }]);

    screens.closeAll();
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work: start("r-2") });
    screens.go({ device: DEVICE, key: KEY, work: start("r-3") });
    screens.closeAll();
    expect(answers().slice(1)).toEqual([
      { device: DEVICE, request: "r-2", outcome: "not_shown" },
      { device: DEVICE, request: "r-3", outcome: "not_shown" },
    ]);
  });

  // Only the page a start was handed to says how it ended, once.
  it("passes on a page's answer only for a start handed to that page", async () => {
    const screens = screensOn(9, { event: vi.fn() });
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work: start("r-1") });
    await loaded(1);
    const view = views.at(-1)!;
    const done = ipc.get(REMOTE_WORK_DONE_CHANNEL)!;
    // Not from the page's own top frame on its own origin: nothing passes.
    done({ sender: view.webContents, senderFrame: { parent: null, url: "https://example.com/" } }, { request: "r-1", outcome: "started" });
    // A start never handed to it, and no result at all: nothing passes.
    done(fromPage(view), { request: "r-9", outcome: "started" });
    done(fromPage(view), { request: "r-1", outcome: "made_up" });
    expect(answers()).toEqual([]);
    done(fromPage(view), { request: "r-1", outcome: "refused", kind: "start_refused" });
    done(fromPage(view), { request: "r-1", outcome: "started" });
    expect(answers()).toEqual([{ device: DEVICE, request: "r-1", outcome: "refused", kind: "start_refused" }]);
  });

  // An agent still coming up is not the end; a page that went takes its
  // made starts with it, logged and unanswered, so none is started twice.
  it("passes on an agent still starting and then its end, and forgets the starts of a page that went", async () => {
    const log = { event: vi.fn() };
    const screens = screensOn(9, log);
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work: start("r-1") });
    await loaded(1);
    const view = views.at(-1)!;
    const done = ipc.get(REMOTE_WORK_DONE_CHANNEL)!;
    done(fromPage(view), { request: "r-1", outcome: "starting" });
    done(fromPage(view), { request: "r-1", outcome: "agent_failed", reason: "exited", line: null });
    expect(answers()).toEqual([
      { device: DEVICE, request: "r-1", outcome: "starting" },
      { device: DEVICE, request: "r-1", outcome: "agent_failed", reason: "exited", line: null },
    ]);

    // Each way the page goes answers its made starts `unconfirmed` once, and the page says nothing after.
    screens.go({ device: DEVICE, key: KEY, work: start("r-2") });
    view.webContents.handlers.get("did-start-navigation")!({ isMainFrame: true, isSameDocument: false });
    screens.go({ device: DEVICE, key: KEY, work: start("r-3") });
    view.webContents.handlers.get("render-process-gone")!();
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    screens.go({ device: DEVICE, key: KEY, work: start("r-4") });
    await loaded(2);
    screens.closeAll();
    done(fromPage(views.at(-1)!), { request: "r-4", outcome: "started" });
    expect(answers().slice(2)).toEqual([
      { device: DEVICE, request: "r-2", outcome: "unconfirmed" },
      { device: DEVICE, request: "r-3", outcome: "unconfirmed" },
      { device: DEVICE, request: "r-4", outcome: "unconfirmed" },
    ]);
    expect(log.event).toHaveBeenCalledWith("remote.work_unknown", { device: DEVICE, reason: "reloaded" });
    expect(log.event).toHaveBeenCalledWith("remote.work_unknown", { device: DEVICE, reason: "page_gone" });
    expect(log.event).toHaveBeenCalledWith("remote.work_unknown", { device: DEVICE, reason: "closed" });
  });

  it("answers the oldest owed start unconfirmed past the cap", async () => {
    const screens = screensOn(9, { event: vi.fn() });
    screens.show({ device: DEVICE, key: KEY, rect: RECT, name: null });
    await loaded(1);
    for (let n = 0; n <= 16; n += 1) screens.go({ device: DEVICE, key: KEY, work: start(`r-${n}`) });
    expect(answers()).toEqual([{ device: DEVICE, request: "r-0", outcome: "unconfirmed" }]);
  });
});
