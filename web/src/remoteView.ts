// Another machine's view of its Hide (PRD device-connect-iroh D-67, B17,
// B20, B63): the desktop app shows the host's own page in a sandboxed view
// whose preload exposes `window.hideRemote` and nothing else. Its bridge
// carries the screen socket, clipboard writes, the app menu's commands and
// the work the guest's operator asked of this screen (D-73, D-78); the page
// in it has no token of the host, no `/health` to ask and no rail.
//
// The socket is the bridge's: the desktop opens it only while the tile
// shows, closes it when the tile is hidden (`hidden`) and says when it shows
// again, so the page opens it at once rather than after a reconnect wait.

import type { Clipboard } from "./clipboard";

/** What the socket of the bridge tells the page. */
export type BridgeSocketEvent =
  | { op: "open" }
  | { op: "message"; data: string | ArrayBuffer }
  | { op: "close"; code: number; reason: string };

/**
 * What the operator asked of another computer's screen from their own
 * (PRD device-connect-iroh D-73, D-78): one of its works to show, by the
 * pane its core knows it by, or a start to make there, which is then that
 * computer's own work.
 */
export type RemoteWork =
  | { op: "show"; pane: string }
  | { op: "start"; request: string; place: RemotePlace; provider: string; model: string | null; prompt: string | null; profile: string | null };

/** Where a start goes on that computer: one checkout's folder, or a new worktree of a repository, each in the wire spelling. */
export type RemotePlace = { checkout: string } | { repository: string; branch: string; base: string | null };

/**
 * How a start handed to another computer's screen ended, by its request id:
 * started there, or why not. `not_shown`: that screen did not load in time,
 * failed to load, went, or a later ask took its place, so its page never had
 * the start. `late`: the page held no snapshot, or did not list the folder
 * it added, within `TAKE_WITHIN_MS`. `not_added`: that computer refused to
 * add the folder as its project. The rest are that computer's core's own
 * answer to the start, as the start panel reads one of its own.
 */
export type RemoteWorkResult =
  | { request: string; outcome: "started" }
  /** Not an end: the tab exists and its agent is still coming up; `started` or `agent_failed` follows, or `unconfirmed`. */
  | { request: string; outcome: "starting" }
  /** The start was made, and what became of it can no longer be read: its page went, or lost its core. */
  | { request: string; outcome: "unconfirmed" }
  /** Not made: the page still follows as many starts as it may. */
  | { request: string; outcome: "busy" }
  | { request: string; outcome: "not_shown" | "late" | "no_answer" }
  | { request: string; outcome: "not_added" | "refused"; kind: string }
  | { request: string; outcome: "failed"; code: string | null }
  | { request: string; outcome: "agent_failed"; reason: string | null; line: string | null };

/**
 * How long the desktop holds a start for its screen's page to load: a page
 * loads in a second or two, and a start that went out much later, when the
 * operator no longer looks at it, would surprise them.
 */
export const HAND_WITHIN_MS = 30_000;

/**
 * How long the page that took a start waits for its first snapshot and for
 * a folder it added to be listed, together, from when the start reached it.
 */
export const TAKE_WITHIN_MS = 30_000;

/** The preload's bridge, version 1. */
export type RemoteBridge = {
  bridge: 1;
  socket: {
    /** Delivers the open socket's events; one socket at a time. */
    events(listener: (event: BridgeSocketEvent) => void): void;
    open(): void;
    send(data: string | ArrayBuffer): void;
    close(): void;
  };
  /** Writes the guest machine's clipboard, which the sandbox keeps from the page. */
  clipboardWrite(text: string): Promise<boolean>;
  /** Delivers each app-menu command id while the tile shows. */
  onMenuCommand(listener: (id: string) => void): void;
  /** Says when the tile shows or hides. */
  onVisible(listener: (visible: boolean) => void): void;
  /** Delivers each work the guest's operator asked of this screen; absent from an older desktop app. */
  onWork?(listener: (work: RemoteWork) => void): void;
  /** Tells the guest how a start it handed here ended. */
  workResult?(result: RemoteWorkResult): void;
};

/** The close the desktop gives the socket of a tile it hid: nothing to show, nothing to reconnect. */
export const HIDDEN_CLOSE = 4012;

/** The bridge of another machine's view, or null in a window of this machine. */
export function remoteBridge(): RemoteBridge | null {
  if (typeof window === "undefined") return null;
  const bridge = (window as unknown as { hideRemote?: RemoteBridge }).hideRemote;
  return bridge?.bridge === 1 ? bridge : null;
}

/** Whether this page is another machine's view of its Hide. */
export function isRemoteView(): boolean {
  return remoteBridge() !== null;
}

/**
 * The bridge's socket as the shell reads a WebSocket: the events it listens
 * to and the calls it makes, nothing more.
 */
export class BridgeSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readyState = BridgeSocket.CONNECTING;
  binaryType: BinaryType = "arraybuffer";

  constructor(private readonly bridge: RemoteBridge) {
    super();
    bridge.socket.events((event) => {
      if (event.op === "open") {
        if (this.readyState !== BridgeSocket.CONNECTING) return;
        this.readyState = BridgeSocket.OPEN;
        this.dispatchEvent(new Event("open"));
      } else if (event.op === "message") {
        if (this.readyState !== BridgeSocket.OPEN) return;
        this.dispatchEvent(new MessageEvent("message", { data: event.data }));
      } else {
        this.closed(event.code, event.reason);
      }
    });
    bridge.socket.open();
  }

  send(data: string | ArrayBufferLike | ArrayBufferView): void {
    if (this.readyState !== BridgeSocket.OPEN) throw new DOMException("the screen socket is not open", "InvalidStateError");
    if (typeof data === "string") this.bridge.socket.send(data);
    else if (ArrayBuffer.isView(data)) this.bridge.socket.send(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer);
    else this.bridge.socket.send(data as ArrayBuffer);
  }

  close(): void {
    if (this.readyState >= BridgeSocket.CLOSING) return;
    this.readyState = BridgeSocket.CLOSING;
    this.bridge.socket.close();
    this.closed(1000, "");
  }

  private closed(code: number, reason: string): void {
    if (this.readyState === BridgeSocket.CLOSED) return;
    this.readyState = BridgeSocket.CLOSED;
    this.dispatchEvent(new CloseEvent("close", { code, reason, wasClean: true }));
  }
}

/** The clipboard of another machine's view: writes go through the bridge; nothing reads the guest's clipboard. */
export function remoteClipboard(bridge: RemoteBridge, diagnostic: (message: string) => void): Clipboard {
  return {
    async write(text, what) {
      const written = await bridge.clipboardWrite(text).catch(() => false);
      if (!written) diagnostic(`copy ${what}: the desktop app refused the clipboard`);
      return written;
    },
    async read(what) {
      diagnostic(`paste ${what}: another machine's view reads no clipboard; paste with the keyboard`);
      return null;
    },
  };
}

/** The app menu's commands of another machine's view, in the shape the keyboard reads a host's. */
export function remoteCommands(): { onCommand(listener: (id: string) => void): () => void } | null {
  const bridge = remoteBridge();
  if (!bridge) return null;
  return {
    onCommand(listener) {
      let on = true;
      bridge.onMenuCommand((id) => {
        if (on) listener(id);
      });
      return () => {
        on = false;
      };
    },
  };
}
