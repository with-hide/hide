import { once } from "node:events";
import type { AddressInfo } from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import WebSocket, { WebSocketServer } from "ws";
import { REMOTE_SOCKET_EVENT_CHANNEL, REMOTE_SOCKET_OPEN_CHANNEL, REMOTE_SOCKET_SEND_CHANNEL } from "../channel";
import type { HostLog } from "./log";
import { MAX_SEND_BYTES, pageAddress } from "./remotePage";
import { RemoteScreens } from "./remoteScreens";

const ipc = vi.hoisted(() => new Map<string, (event: unknown, value?: unknown) => void>());
/** What each `new WebContentsView()` gave the host, newest last. */
const views = vi.hoisted(() => [] as { webContents: { send: (channel: string, value: unknown) => void; loaded: string[]; closed: boolean } }[]);
/** Each session partition the host asked for, in order. */
const partitions = vi.hoisted(() => [] as string[]);
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
            return new Promise(() => undefined);
          },
          close() {
            this.closed = true;
          },
          isDestroyed() {
            return this.closed;
          },
          isFocused: () => false,
          focus: () => undefined,
          on: () => undefined,
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
});

/** A host with the fakes above, its daemon at `server`'s port. */
function screensOn(port: number, log: { event: ReturnType<typeof vi.fn> }): RemoteScreens {
  const window = {
    isDestroyed: () => false,
    isFocused: () => false,
    webContents: { getZoomFactor: () => 1, focus: () => undefined, send: () => undefined },
    contentView: { addChildView: () => undefined, removeChildView: () => undefined },
  };
  const screens = new RemoteScreens({
    window: () => window as never,
    daemon: () => ({ origin: `http://127.0.0.1:${port}`, token: "t" }),
    registry: () => [],
    background: "#000000",
    log: log as unknown as HostLog,
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
    screens.show({ device: DEVICE, key: KEY, rect: RECT });
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

    screens.show({ device: DEVICE, key: KEY, rect: RECT });
    const earlier = views.at(-1)!;
    ipc.get(REMOTE_SOCKET_OPEN_CHANNEL)!(fromPage(earlier));
    await until("the first key's socket", () => asked.length === 1);
    expect(asked).toEqual([`/peer-screen/${DEVICE}/ws?key=${KEY}`]);
    screens.show(null);

    screens.show({ device: DEVICE, key: REPLACEMENT, rect: RECT });
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
