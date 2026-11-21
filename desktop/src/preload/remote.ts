// The remote screen's whole view of the host (PRD device-connect-iroh D-67,
// B20; security spec 8.3, bridge v1): another machine's Hide page, loaded in
// a sandboxed view on its own `hide-remote://` origin, reaches its screen
// socket, the guest machine's clipboard to write, the app menu's commands
// and whether its tile shows, and nothing else of Electron, Node or this
// machine. The page holds no token: the main process opens the socket.

import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import { REMOTE_CLIPBOARD_CHANNEL, REMOTE_MENU_CHANNEL, REMOTE_SOCKET_CLOSE_CHANNEL, REMOTE_SOCKET_EVENT_CHANNEL, REMOTE_SOCKET_OPEN_CHANNEL, REMOTE_SOCKET_SEND_CHANNEL, REMOTE_VISIBLE_CHANNEL } from "../channel";

type SocketEvent = { op: "open" } | { op: "message"; data: string | ArrayBuffer } | { op: "close"; code: number; reason: string };

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
});
