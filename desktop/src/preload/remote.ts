// The remote screen's whole view of the host (PRD device-connect-iroh D-67,
// B20; security spec 8.3, bridge v1): another machine's Hide page, loaded in
// a sandboxed view on its own `hide-remote://` origin, reaches its screen
// socket, the guest machine's clipboard to write, the app menu's commands,
// whether its tile shows, the work the guest's operator asked of it and how
// a start it was handed ended (D-73, D-78), and nothing else of Electron,
// Node or this machine. The page holds no token: the main process opens the socket.

import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import { REMOTE_BROWSER_CAPTURE_CHANNEL, REMOTE_BROWSER_COMMAND_CHANNEL, REMOTE_BROWSER_CYCLE_END_CHANNEL, REMOTE_BROWSER_EVENT_CHANNEL, REMOTE_BROWSER_SYNC_CHANNEL, REMOTE_CLIPBOARD_CHANNEL, REMOTE_MENU_CHANNEL, REMOTE_SOCKET_CLOSE_CHANNEL, REMOTE_SOCKET_EVENT_CHANNEL, REMOTE_SOCKET_OPEN_CHANNEL, REMOTE_SOCKET_SEND_CHANNEL, REMOTE_VIEWER_CHANNEL, REMOTE_VISIBLE_CHANNEL, REMOTE_PAGE_SHOWING_CHANNEL, REMOTE_WORK_CHANNEL, REMOTE_WORK_DONE_CHANNEL } from "../channel";

type SocketEvent = { op: "open" } | { op: "message"; data: string | ArrayBuffer } | { op: "close"; code: number; reason: string };
type Viewer = { language: string; name: string | null; display: unknown };

// What the showing window calls this machine, its language and its display
// choices (the page reads those, as it reads a value of a build it may not
// know), heard from
// the start: the main process says it once the page loaded and again only
// when it changes, which may be before the page first listens, so the last
// one is kept for a listener that comes later.
let viewer: Viewer | null = null;
let viewerListener: ((viewer: Viewer) => void) | null = null;
ipcRenderer.on(REMOTE_VIEWER_CHANNEL, (_event: IpcRendererEvent, told: unknown) => {
  if (typeof told !== "object" || told === null) return;
  const { language, name, display } = told as Record<string, unknown>;
  if (typeof language !== "string" || (name !== null && typeof name !== "string")) return;
  viewer = { language, name, display: display ?? null };
  viewerListener?.(viewer);
});

contextBridge.exposeInMainWorld("hideRemote", {
  bridge: 1,
  socket: {
    events(listener: (event: SocketEvent) => void): void {
      ipcRenderer.removeAllListeners(REMOTE_SOCKET_EVENT_CHANNEL);
      ipcRenderer.on(REMOTE_SOCKET_EVENT_CHANNEL, (_event: IpcRendererEvent, event: SocketEvent) => {
        // A binary frame crosses as a Uint8Array; the page's socket hands out ArrayBuffers.
        if (event.op === "message" && event.data instanceof Uint8Array) {
          const bytes = event.data;
          listener({ op: "message", data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer });
          return;
        }
        listener(event);
      });
    },
    open(): void {
      ipcRenderer.send(REMOTE_SOCKET_OPEN_CHANNEL);
    },
    send(data: string | ArrayBuffer): void {
      ipcRenderer.send(REMOTE_SOCKET_SEND_CHANNEL, typeof data === "string" ? data : new Uint8Array(data));
    },
    close(): void {
      ipcRenderer.send(REMOTE_SOCKET_CLOSE_CHANNEL);
    },
  },
  clipboardWrite(text: string): Promise<boolean> {
    return ipcRenderer.invoke(REMOTE_CLIPBOARD_CHANNEL, String(text)) as Promise<boolean>;
  },
  onMenuCommand(listener: (id: string) => void): void {
    ipcRenderer.removeAllListeners(REMOTE_MENU_CHANNEL);
    ipcRenderer.on(REMOTE_MENU_CHANNEL, (_event: IpcRendererEvent, id: unknown) => {
      if (typeof id === "string") listener(id);
    });
  },
  onVisible(listener: (visible: boolean) => void): void {
    ipcRenderer.removeAllListeners(REMOTE_VISIBLE_CHANNEL);
    ipcRenderer.on(REMOTE_VISIBLE_CHANNEL, (_event: IpcRendererEvent, visible: unknown) => {
      if (typeof visible === "boolean") listener(visible);
    });
  },
  // The main process resolved the language and checked the name the shell
  // gave; a listener hears the last one at once, then each change.
  onViewer(listener: (viewer: Viewer) => void): void {
    viewerListener = listener;
    if (viewer) listener(viewer);
  },
  // The main process checked the ask's shape before it handed it here.
  onWork(listener: (work: unknown) => void): void {
    ipcRenderer.removeAllListeners(REMOTE_WORK_CHANNEL);
    ipcRenderer.on(REMOTE_WORK_CHANNEL, (_event: IpcRendererEvent, work: unknown) => {
      if (typeof work === "object" && work !== null) listener(work);
    });
  },
  // The main process reads it as a result of a start it handed this page, or drops it.
  workResult(result: unknown): void {
    ipcRenderer.send(REMOTE_WORK_DONE_CHANNEL, result);
  },
  // The main process checks it and tells the window that shows this screen.
  showing(shown: unknown): void {
    ipcRenderer.send(REMOTE_PAGE_SHOWING_CHANNEL, shown);
  },
  // The browser displays this screen places, drawn by this window's desktop
  // app with every request leaving through the screen's machine; the main
  // process checks each message as it checks this window's shell's. Added
  // beside `bridge: 1`, which every page checks, so an older page ignores it.
  browser: {
    sync(state: unknown): void {
      ipcRenderer.send(REMOTE_BROWSER_SYNC_CHANNEL, state);
    },
    endCycle(cycleId: number): void {
      ipcRenderer.send(REMOTE_BROWSER_CYCLE_END_CHANNEL, cycleId);
    },
    capture(workspace: string, id: string): Promise<string | null> {
      return ipcRenderer.invoke(REMOTE_BROWSER_CAPTURE_CHANNEL, { workspace, id }) as Promise<string | null>;
    },
    command(workspace: string, id: string, command: string): void {
      ipcRenderer.send(REMOTE_BROWSER_COMMAND_CHANNEL, { workspace, id }, command);
    },
    onEvent(listener: (event: unknown) => void): () => void {
      const handler = (_event: IpcRendererEvent, event: unknown) => {
        if (typeof event === "object" && event !== null) listener(event);
      };
      ipcRenderer.on(REMOTE_BROWSER_EVENT_CHANNEL, handler);
      return () => {
        ipcRenderer.removeListener(REMOTE_BROWSER_EVENT_CHANNEL, handler);
      };
    },
  },
});
