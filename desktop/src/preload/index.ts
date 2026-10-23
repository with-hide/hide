// The renderer's whole view of the host (B11): which host this is, the app
// menu's commands in, the stored macOS pane chords out for the menu to show,
// the host's OS, a file or folder to show in the OS file manager, a folder the operator picks to add as a project,
// the paths a terminal link names and handing one to macOS, and the pages of
// browser displays (issue 155), and the Factory's and agents' notifications with their clicks,
// withdrawals, the system's permission for them and the app icon's count, the window frame the
// top row is drawn inside, a folder handed to VS Code, the app's own updates, and its Quit.
// No Node API, no other channel. Browser pages load in their own session with no preload, so none
// of this reaches them.

import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import type { BrowserCommand, BrowserHostEvent, BrowserSync } from "../../../web/src/host";
import type { HerdrRestart, NotificationPermission, ProbedPath, UpdateStatus, WindowChrome } from "../../../web/src/host";
import { BINDINGS_CHANNEL, BROWSER_CAPTURE_CHANNEL, BROWSER_COMMAND_CHANNEL, BROWSER_CYCLE_END_CHANNEL, BROWSER_EVENT_CHANNEL, BROWSER_SYNC_CHANNEL, COMMAND_CHANNEL, EDITOR_AVAILABLE_CHANNEL, EDITOR_OPEN_CHANNEL, LANGUAGE_CHANNEL, NOTIFY_BADGE_CHANNEL, NOTIFY_CHANNEL, NOTIFY_OPEN_CHANNEL, NOTIFY_PERMISSION_CHANNEL, NOTIFY_SETTINGS_CHANNEL, NOTIFY_WITHDRAW_CHANNEL, OPEN_PATH_CHANNEL, PICK_FOLDER_CHANNEL, PROBE_PATHS_CHANNEL, HERDR_RESTART_CHANNEL, QUIT_CHANNEL, REVEAL_CHANNEL, UPDATE_CHECK_CHANNEL, UPDATE_PRERELEASE_CHANNEL, UPDATE_RESTART_CHANNEL, UPDATE_STATUS_CHANNEL, UPDATE_STATUS_GET_CHANNEL, WINDOW_CHROME_CHANNEL, WINDOW_CHROME_GET_CHANNEL } from "../channel";

contextBridge.exposeInMainWorld("hideHost", {
  kind: "electron",
  platform: process.platform,
  onCommand(listener: (id: string) => void): () => void {
    const handler = (_event: IpcRendererEvent, id: unknown) => {
      if (typeof id === "string") listener(id);
    };
    ipcRenderer.on(COMMAND_CHANNEL, handler);
    return () => {
      ipcRenderer.removeListener(COMMAND_CHANNEL, handler);
    };
  },
  reportBindings(bindings: Record<string, string>): void {
    ipcRenderer.send(BINDINGS_CHANNEL, bindings);
  },
  reportLanguage(language: string | null): void {
    ipcRenderer.send(LANGUAGE_CHANNEL, language);
  },
  revealPath(path: string): void {
    ipcRenderer.send(REVEAL_CHANNEL, path);
  },
  pickFolder(): Promise<string | null> {
    return ipcRenderer.invoke(PICK_FOLDER_CHANNEL) as Promise<string | null>;
  },
  probePaths(paths: string[]): Promise<ProbedPath[]> {
    return ipcRenderer.invoke(PROBE_PATHS_CHANNEL, paths) as Promise<ProbedPath[]>;
  },
  openPath(path: string): void {
    ipcRenderer.send(OPEN_PATH_CHANNEL, path);
  },
  notify(id: string, title: string, body: string, options?: { unlessFocused?: boolean }): void {
    ipcRenderer.send(NOTIFY_CHANNEL, { id, title, body, unlessFocused: options?.unlessFocused ?? false });
  },
  withdrawNotifications(ids: string[]): void {
    ipcRenderer.send(NOTIFY_WITHDRAW_CHANNEL, ids);
  },
  notificationPermission(): Promise<NotificationPermission> {
    return ipcRenderer.invoke(NOTIFY_PERMISSION_CHANNEL) as Promise<NotificationPermission>;
  },
  openNotificationSettings(): void {
    ipcRenderer.send(NOTIFY_SETTINGS_CHANNEL);
  },
  reportBadge(count: number): void {
    ipcRenderer.send(NOTIFY_BADGE_CHANNEL, count);
  },
  onNotificationOpen(listener: (id: string) => void): () => void {
    const handler = (_event: IpcRendererEvent, id: unknown) => {
      if (typeof id === "string") listener(id);
    };
    ipcRenderer.on(NOTIFY_OPEN_CHANNEL, handler);
    return () => {
      ipcRenderer.removeListener(NOTIFY_OPEN_CHANNEL, handler);
    };
  },
  onWindowChrome(listener: (chrome: WindowChrome) => void): () => void {
    // The main process is the only sender on this channel; the asked state
    // is dropped if a change has already arrived, which is newer.
    let heard = false;
    const handler = (_event: IpcRendererEvent, chrome: WindowChrome) => {
      heard = true;
      listener(chrome);
    };
    ipcRenderer.on(WINDOW_CHROME_CHANNEL, handler);
    void (ipcRenderer.invoke(WINDOW_CHROME_GET_CHANNEL) as Promise<WindowChrome | null>).then((chrome) => {
      if (chrome !== null && !heard) listener(chrome);
    });
    return () => {
      ipcRenderer.removeListener(WINDOW_CHROME_CHANNEL, handler);
    };
  },
  editorAvailable(): Promise<boolean> {
    return ipcRenderer.invoke(EDITOR_AVAILABLE_CHANNEL) as Promise<boolean>;
  },
  openInEditor(path: string): void {
    ipcRenderer.send(EDITOR_OPEN_CHANNEL, path);
  },
  restartHerdr(): Promise<HerdrRestart> {
    return ipcRenderer.invoke(HERDR_RESTART_CHANNEL) as Promise<HerdrRestart>;
  },
  quit(): void {
    ipcRenderer.send(QUIT_CHANNEL);
  },
  updates: {
    onStatus(listener: (status: UpdateStatus) => void): () => void {
      // As onWindowChrome: the asked status is dropped if a change already arrived.
      let heard = false;
      const handler = (_event: IpcRendererEvent, status: UpdateStatus) => {
        heard = true;
        listener(status);
      };
      ipcRenderer.on(UPDATE_STATUS_CHANNEL, handler);
      void (ipcRenderer.invoke(UPDATE_STATUS_GET_CHANNEL) as Promise<UpdateStatus | null>).then((status) => {
        if (status !== null && !heard) listener(status);
      });
      return () => {
        ipcRenderer.removeListener(UPDATE_STATUS_CHANNEL, handler);
      };
    },
    check(): void {
      ipcRenderer.send(UPDATE_CHECK_CHANNEL);
    },
    restart(): void {
      ipcRenderer.send(UPDATE_RESTART_CHANNEL);
    },
    setPrerelease(on: boolean): void {
      ipcRenderer.send(UPDATE_PRERELEASE_CHANNEL, on);
    },
  },
  browser: {
    endCycle(cycleId: number): void {
      ipcRenderer.send(BROWSER_CYCLE_END_CHANNEL, cycleId);
    },
    sync(state: BrowserSync): void {
      ipcRenderer.send(BROWSER_SYNC_CHANNEL, state);
    },
    capture(workspace: string, id: string): Promise<string | null> {
      return ipcRenderer.invoke(BROWSER_CAPTURE_CHANNEL, { workspace, id }) as Promise<string | null>;
    },
    command(workspace: string, id: string, command: BrowserCommand): void {
      ipcRenderer.send(BROWSER_COMMAND_CHANNEL, { workspace, id }, command);
    },
    onEvent(listener: (event: BrowserHostEvent) => void): () => void {
      // The main process is the only sender on this channel.
      const handler = (_event: IpcRendererEvent, event: BrowserHostEvent) => listener(event);
      ipcRenderer.on(BROWSER_EVENT_CHANNEL, handler);
      return () => {
        ipcRenderer.removeListener(BROWSER_EVENT_CHANNEL, handler);
      };
    },
  },
});
