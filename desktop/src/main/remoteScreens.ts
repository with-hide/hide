// Another machine's own Hide screen in this window (PRD device-connect-iroh
// D-67, B17, B20, B63; security spec 8.3). Each paired machine's screen is a
// sandboxed WebContentsView on its own origin (`hide-remote://<device>/`) and
// its own in-memory session, whose preload carries bridge v1 and nothing
// else. Its page is fetched from this computer's daemon, which answers it
// over the connection to that machine, and its one socket is opened here,
// with this computer's token, only while the shell shows that screen: a
// hidden screen's socket closes and the host keeps nothing of it. The page
// may go nowhere, ask for no permission and reach no network; what it writes
// to the clipboard and the app menu's commands come through the bridge.

import path from "node:path";
import { clipboard, ipcMain, protocol, session, WebContentsView, type BrowserWindow, type Input, type IpcMainEvent, type IpcMainInvokeEvent, type Session } from "electron";
import WebSocket from "ws";
import type { RemoteScreenState } from "../../../web/src/host";
import { matchHost, type Command } from "../../../web/src/shortcuts";
import { COMMAND_CHANNEL, REMOTE_CLIPBOARD_CHANNEL, REMOTE_MENU_CHANNEL, REMOTE_SOCKET_CLOSE_CHANNEL, REMOTE_SOCKET_EVENT_CHANNEL, REMOTE_SOCKET_OPEN_CHANNEL, REMOTE_SOCKET_SEND_CHANNEL, REMOTE_STATE_CHANNEL, REMOTE_VISIBLE_CHANNEL } from "../channel";
import { toBounds } from "./browserSync";
import type { HostLog } from "./log";
import { fetchPath, HIDDEN_CLOSE, MAX_SEND_BYTES, ofPage, pageAddress, REMOTE_SCHEME, requestAllowed, socketAddress, type ShowRequest } from "./remotePage";

/** Registered before the app is ready, as Electron requires of a privileged scheme. */
export function registerRemoteScheme(): void {
  protocol.registerSchemesAsPrivileged([{ scheme: REMOTE_SCHEME, privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true, codeCache: true } }]);
}

/** How many machines' screens stay loaded; past it, the one hidden longest is closed and loads again when shown. */
const KEPT = 3;

/** This computer's daemon as the window is attached to it. */
export type Daemon = { origin: string; token: string };

type Screen = {
  device: string;
  view: WebContentsView;
  socket: WebSocket | null;
  visible: boolean;
  /** Where its page stands as last told to the shell: a failed one loads again when shown again. */
  state: RemoteScreenState["state"];
  /** When it last showed, to close the one hidden longest past `KEPT`. */
  shownAt: number;
};

export type RemoteScreensDeps = {
  window: () => BrowserWindow | null;
  daemon: () => Daemon | null;
  /** The resolved shortcuts, to give ⌥⌘n back to the shell's rail. */
  registry: () => readonly Command[];
  /** The window's own color, which a screen wears until its page paints. */
  background: string;
  log: HostLog;
};

export class RemoteScreens {
  private readonly screens = new Map<string, Screen>();
  private readonly sessions = new Set<string>();
  private front: string | null = null;
  private clock = 0;

  constructor(private readonly deps: RemoteScreensDeps) {}

  listen(): void {
    ipcMain.on(REMOTE_SOCKET_OPEN_CHANNEL, (event: IpcMainEvent) => {
      const screen = this.screenOf(event);
      if (screen) this.openSocket(screen);
    });
    ipcMain.on(REMOTE_SOCKET_SEND_CHANNEL, (event: IpcMainEvent, data: unknown) => {
      const screen = this.screenOf(event);
      const socket = screen?.socket;
      if (!screen || !socket || socket.readyState !== WebSocket.OPEN) return;
      if (typeof data === "string") {
        if (data.length > MAX_SEND_BYTES) return this.refused(screen, "send_too_large");
        socket.send(data);
      } else if (data instanceof Uint8Array) {
        if (data.byteLength > MAX_SEND_BYTES) return this.refused(screen, "send_too_large");
        socket.send(data, { binary: true });
      }
    });
    ipcMain.on(REMOTE_SOCKET_CLOSE_CHANNEL, (event: IpcMainEvent) => {
      const screen = this.screenOf(event);
      if (screen) this.closeSocket(screen, null);
    });
    ipcMain.handle(REMOTE_CLIPBOARD_CHANNEL, (event: IpcMainInvokeEvent, text: unknown) => {
      const screen = this.screenOf(event);
      // A screen writes the clipboard only while it shows and holds the keyboard, as a page in front would.
      if (!screen || !screen.visible || !screen.view.webContents.isFocused() || typeof text !== "string" || text.length > MAX_SEND_BYTES) return false;
      clipboard.writeText(text);
      return true;
    });
  }

  /** The shell's ask: one machine's screen over `rect`, every other hidden; or none. */
  show(request: ShowRequest): void {
    const window = this.deps.window();
    if (!window || window.isDestroyed()) return;
    const device = request?.device ?? null;
    for (const screen of this.screens.values()) if (screen.device !== device) this.hide(screen);
    if (request === null) {
      if (this.front !== null && window.isFocused()) window.webContents.focus();
      this.front = null;
      return;
    }
    const screen = this.screens.get(request.device) ?? this.create(window, request.device);
    if (!screen) return;
    screen.view.setBounds(toBounds(request.rect, window.webContents.getZoomFactor()));
    screen.shownAt = ++this.clock;
    const shown = screen.visible;
    screen.visible = true;
    // A page that could not load loads again each time the shell asks, which
    // it does when its machine's state moves and on its own retry (B21).
    if (screen.state === "failed") this.load(screen);
    else if (!shown) {
      // The stage asking again was drawn anew, so it is told where the page stands (B63).
      this.report(screen.device, screen.state);
      if (screen.state === "shown") screen.view.setVisible(true);
      screen.view.webContents.send(REMOTE_VISIBLE_CHANNEL, true);
      // The screen takes the keyboard from the shell that asked for it.
      if (window.isFocused()) screen.view.webContents.focus();
    }
    this.front = screen.device;
    this.trim(window);
  }

  /** An app-menu command while a screen shows goes to that screen (B20); true when one took it. */
  sendCommand(id: string): boolean {
    const screen = this.front === null ? null : this.screens.get(this.front);
    if (!screen?.visible) return false;
    screen.view.webContents.send(REMOTE_MENU_CHANNEL, id);
    return true;
  }

  /** Every screen closes: the window went, or this computer's daemon is another one now. */
  closeAll(): void {
    const window = this.deps.window();
    for (const screen of [...this.screens.values()]) this.destroy(screen, window);
    this.front = null;
  }

  private create(window: BrowserWindow, device: string): Screen | null {
    const view = new WebContentsView({
      webPreferences: {
        session: this.sessionFor(device),
        preload: path.join(__dirname, "remote-preload.js"),
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        webSecurity: true,
        webviewTag: false,
        spellcheck: false,
        backgroundThrottling: false,
      },
    });
    // The window's own color until the page paints, so a screen never flashes white.
    view.setBackgroundColor(this.deps.background);
    view.setVisible(false);
    const screen: Screen = { device, view, socket: null, visible: true, state: "opening", shownAt: 0 };
    this.guard(screen);
    window.contentView.addChildView(view);
    this.screens.set(device, screen);
    this.load(screen);
    this.deps.log.event("remote.view_created", { device, live: this.screens.size });
    return screen;
  }

  /** Loads the page, shown once it loaded while the shell still wants it; a failure stands aside for the stage's words. */
  private load(screen: Screen): void {
    this.report(screen.device, "opening");
    screen.view.webContents.loadURL(pageAddress(screen.device)).then(
      () => {
        if (screen.state === "failed" || !this.screens.has(screen.device)) return;
        this.report(screen.device, "shown");
        if (!screen.visible) return;
        screen.view.setVisible(true);
        const window = this.deps.window();
        if (window?.isFocused()) screen.view.webContents.focus();
      },
      () => undefined, // did-fail-load says why
    );
  }

  /** One in-memory session per machine: nothing it stores outlives the app, and no page of another machine or of this one shares it. */
  private sessionFor(device: string): Session {
    const partition = `hide-remote-${device}`;
    const pageSession = session.fromPartition(partition);
    if (this.sessions.has(partition)) return pageSession;
    this.sessions.add(partition);
    pageSession.protocol.handle(REMOTE_SCHEME, (request) => this.fetchPage(request.url, device));
    pageSession.setPermissionRequestHandler((_contents, permission, callback) => {
      this.deps.log.event("remote.permission_refused", { device, permission });
      callback(false);
    });
    pageSession.setPermissionCheckHandler(() => false);
    pageSession.webRequest.onBeforeRequest((details, callback) => {
      const allowed = requestAllowed(details.url, device);
      if (!allowed) this.deps.log.event("remote.request_refused", { device, scheme: details.url.split(":", 1)[0] ?? "" });
      callback({ cancel: !allowed });
    });
    pageSession.on("will-download", (event) => event.preventDefault());
    return pageSession;
  }

  /** A load of the page: its index or an asset, from this computer's daemon, with its token, which the page never holds. */
  private async fetchPage(url: string, device: string): Promise<Response> {
    const where = fetchPath(url, device);
    const daemon = this.deps.daemon();
    if (!where || !daemon) return new Response("", { status: 404 });
    try {
      const answer = await fetch(`${daemon.origin}${where}`, { headers: { authorization: `Bearer ${daemon.token}` } });
      if (!answer.ok && where === `/peer-screen/${device}/`) {
        const reason = await answer.json().then((body: { reason?: unknown }) => (typeof body.reason === "string" ? body.reason : "not_connected"), () => "not_connected");
        this.failed(device, reason);
      }
      return new Response(answer.body, { status: answer.status, headers: { "content-type": answer.headers.get("content-type") ?? "application/octet-stream" } });
    } catch {
      if (where === `/peer-screen/${device}/`) this.failed(device, "not_connected");
      return new Response("", { status: 503 });
    }
  }

  private failed(device: string, reason: string): void {
    const screen = this.screens.get(device);
    if (!screen) return;
    screen.view.setVisible(false);
    this.closeSocket(screen, null);
    this.report(device, "failed", reason);
  }

  private guard(screen: Screen): void {
    const contents = screen.view.webContents;
    const device = screen.device;
    const stay = (event: { preventDefault(): void }, url: string) => {
      if (ofPage(url, device)) return;
      event.preventDefault();
      this.deps.log.event("remote.navigation_refused", { device });
    };
    contents.on("will-navigate", stay);
    contents.on("will-redirect", stay);
    contents.on("will-frame-navigate", (event) => stay(event, event.url));
    contents.on("will-attach-webview", (event) => event.preventDefault());
    contents.setWindowOpenHandler(() => {
      this.deps.log.event("remote.window_refused", { device });
      return { action: "deny" };
    });
    contents.on("did-fail-load", (_event, code, _description, _url, mainFrame) => {
      if (!mainFrame || code === -3) return; // -3: a load another load replaced
      if (screen.state !== "failed") this.failed(device, "not_connected");
    });
    contents.on("render-process-gone", () => this.failed(device, "page_gone"));
    // ⌥⌘n chooses a computer tile, which is this window's rail, not the screen's (B47).
    contents.on("before-input-event", (event, input: Input) => {
      if (input.type !== "keyDown" || input.isComposing) return;
      const command = matchHost({ code: input.code, metaKey: input.meta, altKey: input.alt, shiftKey: input.shift, ctrlKey: input.control }, this.deps.registry(), "electron");
      if (!command?.id.startsWith("select_device_")) return;
      event.preventDefault();
      const window = this.deps.window();
      if (!window || window.isDestroyed()) return;
      window.webContents.focus();
      window.webContents.send(COMMAND_CHANNEL, command.id);
    });
  }

  /** The page's socket, opened only while its screen shows; a second one replaces the first. */
  private openSocket(screen: Screen): void {
    const daemon = this.deps.daemon();
    this.closeSocket(screen, null);
    if (!screen.visible || !daemon) {
      this.toPage(screen, { op: "close", code: HIDDEN_CLOSE, reason: "hidden" });
      return;
    }
    const socket = new WebSocket(socketAddress(daemon.origin, screen.device), { headers: { authorization: `Bearer ${daemon.token}` }, maxPayload: 64 * 1024 * 1024 });
    screen.socket = socket;
    socket.on("open", () => {
      if (screen.socket === socket) this.toPage(screen, { op: "open" });
    });
    socket.on("message", (data, binary) => {
      if (screen.socket !== socket) return;
      const bytes = Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as ArrayBuffer);
      this.toPage(screen, { op: "message", data: binary ? new Uint8Array(bytes) : bytes.toString("utf8") });
    });
    socket.on("close", (code, reason) => {
      if (screen.socket !== socket) return;
      screen.socket = null;
      this.toPage(screen, { op: "close", code: code === 1005 ? 1006 : code, reason: reason.toString("utf8") });
    });
    socket.on("error", () => socket.terminate());
  }

  private closeSocket(screen: Screen, tell: { code: number; reason: string } | null): void {
    const socket = screen.socket;
    if (!socket) return;
    screen.socket = null;
    socket.removeAllListeners();
    socket.on("error", () => undefined);
    if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
    else socket.close(1000);
    if (tell) this.toPage(screen, { op: "close", ...tell });
  }

  /** A hidden screen keeps its page and closes its socket, so the host keeps nothing of it (B63). */
  private hide(screen: Screen): void {
    if (!screen.visible) return;
    screen.visible = false;
    screen.view.setVisible(false);
    screen.view.webContents.send(REMOTE_VISIBLE_CHANNEL, false);
    this.closeSocket(screen, { code: HIDDEN_CLOSE, reason: "hidden" });
  }

  private trim(window: BrowserWindow): void {
    const hidden = [...this.screens.values()].filter((screen) => !screen.visible).sort((a, b) => a.shownAt - b.shownAt);
    while (this.screens.size > KEPT && hidden.length > 0) this.destroy(hidden.shift()!, window);
  }

  private destroy(screen: Screen, window: BrowserWindow | null): void {
    this.closeSocket(screen, null);
    this.screens.delete(screen.device);
    if (window && !window.isDestroyed()) window.contentView.removeChildView(screen.view);
    if (!screen.view.webContents.isDestroyed()) screen.view.webContents.close();
  }

  private toPage(screen: Screen, event: { op: "open" } | { op: "message"; data: string | Uint8Array } | { op: "close"; code: number; reason: string }): void {
    if (!screen.view.webContents.isDestroyed()) screen.view.webContents.send(REMOTE_SOCKET_EVENT_CHANNEL, event);
  }

  private report(device: string, state: RemoteScreenState["state"], reason?: string): void {
    const screen = this.screens.get(device);
    if (screen) screen.state = state;
    const window = this.deps.window();
    if (!window || window.isDestroyed()) return;
    window.webContents.send(REMOTE_STATE_CHANNEL, { device, state, ...(reason ? { reason } : {}) } satisfies RemoteScreenState);
  }

  private refused(screen: Screen, reason: string): void {
    this.deps.log.event("remote.send_refused", { device: screen.device, reason });
  }

  /** The screen whose page sent `event`: its own top frame on its own origin, or none. */
  private screenOf(event: IpcMainEvent | IpcMainInvokeEvent): Screen | null {
    const frame = event.senderFrame;
    if (!frame || frame.parent !== null) return null;
    for (const screen of this.screens.values()) {
      if (event.sender === screen.view.webContents && ofPage(frame.url, screen.device)) return screen;
    }
    return null;
  }
}
