// The desktop host: finds the daemon through the `hide` CLI, shows the web
// shell it serves in one window, and follows the daemon if it goes away.
//
//   connecting -> attached(url) | failed(reason)
//   attached   -> lost          two missed /health answers in a row
//   lost       -> attached      `hide status --json` names a live daemon
//   failed, lost -> connecting  Retry, a second launch, or a Dock click
//
// Only `connecting` may start a daemon (`hide connect`); `lost` only
// re-attaches. The daemon is never stopped here (desktop PRD D-03).

import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { app, BrowserWindow, clipboard, dialog, ipcMain, nativeImage, net, Notification, screen, session, shell, type IpcMainEvent, type IpcMainInvokeEvent, type NativeImage } from "electron";
import { DebUpdater, MacUpdater, NsisUpdater, RpmUpdater } from "electron-updater";
import { CHECK_UPDATES_COMMAND, SETUP_COMMAND, type HerdrRestart } from "../../../web/src/host";
import type { MenuCommand } from "./menu";
import { BINDINGS_CHANNEL, COMMAND_CHANNEL, EDITOR_AVAILABLE_CHANNEL, EDITOR_OPEN_CHANNEL, LANGUAGE_CHANNEL, NOTIFY_BADGE_CHANNEL, NOTIFY_CHANNEL, NOTIFY_OPEN_CHANNEL, NOTIFY_PERMISSION_CHANNEL, NOTIFY_SETTINGS_CHANNEL, NOTIFY_WITHDRAW_CHANNEL, OPEN_PATH_CHANNEL, PICK_FOLDER_CHANNEL, PROBE_PATHS_CHANNEL, HERDR_RESTART_CHANNEL, REVEAL_CHANNEL, UPDATE_CHECK_CHANNEL, UPDATE_PRERELEASE_CHANNEL, UPDATE_RESTART_CHANNEL, UPDATE_STATUS_CHANNEL, UPDATE_STATUS_GET_CHANNEL, WINDOW_CHROME_CHANNEL, WINDOW_CHROME_GET_CHANNEL } from "../channel";
import { BrowserViews, type ResolvedPage } from "./browser";
import { BrowserCdpGateway, CdpActionUncertain, parseBrowserControlResult, type CdpAction, type CdpActionResult, type CdpScope } from "./browserCdp";
import {
  cliRecords,
  HAS_LOGIN_SHELL,
  loginPathCommand,
  healthFailure,
  parseConnect,
  parseLoginPath,
  parseRememberedCli,
  parseRuntimePrepare,
  parseStatus,
  REMEMBERED_SOURCES,
  rememberedCliPath,
  rememberedCliValue,
  resolveCli,
  systemDirs,
  wellKnownDirs,
  type Attached,
  type CliSource,
  type CoreBuilds,
  type FailureReason,
} from "./cli";
import type { DesktopEnv } from "./env";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import type { HostLanguage } from "./language";
import { chooseHerdr, ensureServer, parseServerStatus, restartServer, serverEnvironment, type HerdrChoice } from "./herdr";
import { loadFailureFields, type HostLog } from "./log";
import { ChildRunner, executableFile, startDetached, type ChildResult } from "./spawn";
import { fallbackScreen, fallbackScript, statusPageFile } from "./statusPage";
import { MIN_SIZE, readWindowState, restoreBounds, windowStatePath, writeWindowState } from "./windowState";
import { openRoute, probe, probeRequest } from "./localPath";
import { revealTarget } from "./reveal";
import { badgeRequest, ClickGate, KeptNotifications, launchedId, notifyRequest, OVERLAY_SIZE, overlayDot, toastXml, withdrawRequest } from "./notify";
import { Editor } from "./editor";
import { frameOptions, hidesTitleBar, windowChrome } from "./windowChrome";
import { NOTIFICATION_SETTINGS_URL, NotificationPermissionReader } from "./notificationPermission";
import { fromPage, toPage, WirePathError } from "./wirePath";
import { AppUpdates, fetchReleases, macosLocation, readPackageType, readPrerelease, savePrerelease, updatesFile, updateSystem, type UpdateSystem, type UpdaterLike } from "./updater";

declare const __HIDE_BACKGROUND__: string;
declare const __HIDE_WARNING__: string;

/**
 * `hide connect` refuses on its own a start that stands still for 10 s, naming the step; this bound is for a CLI that
 * never answers. A first start right after a Windows install measured 1.4 to 1.9 s once it stopped flushing the drive's
 * root (#1515), and 4.3 to 6.7 s, past 10 s on a busy runner, before.
 */
const CONNECT_TIMEOUT_MS = 25_000;
/** The first launch of a Windows build copies its runtime (about 150 MB); later ones find the copy. */
const RUNTIME_PREPARE_TIMEOUT_MS = 120_000;
const STATUS_TIMEOUT_MS = 5_000;
/** VS Code's default for the same question (`application.shellEnvironmentResolutionTimeout`); a warm rc takes seconds. */
const LOGIN_PATH_TIMEOUT_MS = 10_000;
const HEALTH_INTERVAL_MS = 2_000;
const HEALTH_TIMEOUT_MS = 1_500;
const LOST_AFTER_MISSES = 2;
const LOST_POLL_MS = 3_000;
/** A started Herdr answered within 0.1 s in a 2026-09-29 run; restoring many panes can take longer. */
const HERDR_START_WAIT_MS = 5_000;
const HERDR_START_POLL_MS = 200;
const STATUS_PAGE = statusPageFile(__dirname);
const CLIPBOARD_PERMISSIONS = new Set(["clipboard-read", "clipboard-sanitized-write"]);
const EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

export type HostState =
  | { kind: "connecting" }
  | ({ kind: "attached" } & Attached)
  | ({ kind: "lost" } & Attached)
  | { kind: "failed"; reason: FailureReason; file?: string; builds?: CoreBuilds };

function isExecutable(file: string): boolean {
  try {
    fs.accessSync(file, fs.constants.X_OK);
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}

/** Whether this account may create and replace entries in `folder`. */
function isWritable(folder: string): boolean {
  try {
    fs.accessSync(folder, fs.constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

function mtimeMs(file: string): number | null {
  try {
    return fs.statSync(file).mtimeMs;
  } catch {
    return null;
  }
}

export class DesktopHost {
  private state: HostState = { kind: "connecting" };
  private window: BrowserWindow | null = null;
  /**
   * The status page load still in flight. The window takes one navigation at
   * a time, so every render waits for it: a shell load started while the
   * status page is still navigating (a daemon that answers at once, as on a
   * relaunch) lands but is rejected with an empty code and leaves the
   * renderer's navigation tracking pending, and a hash set before the page
   * commits is lost. Electron settles the load when its window is destroyed.
   */
  private statusLoad: Promise<unknown> | null = null;
  /** Whether a render already waits for `statusLoad`; it renders whatever state is current then. */
  private renderWaits = false;
  /**
   * The status screen the window shows now, or null while it shows anything
   * else. Electron answers `getURL()` with the address of a load that failed,
   * so the address cannot say whether the page is there.
   */
  private statusScreen: { lang: string; plain: boolean } | null = null;
  /** Set when the status page file would not load; every later status screen is the plain one (`statusPage.ts`). */
  private statusPageBroken = false;
  private readonly runner = new ChildRunner();
  /** Every CLI child runs through this chain, so the runner's cap of one is never crossed. */
  private cliChain: Promise<unknown> = Promise.resolve();
  private discovering: Promise<void> | null = null;
  /** The one Herdr restart running (`listenHerdrRestart`). */
  private herdrRestart: Promise<HerdrRestart> | null = null;
  private attempts = 0;
  private cli: { path: string; source: CliSource } | null = null;
  /**
   * Where a packaged Windows app runs its daemon, CLI and Herdr from: this
   * build's copy outside the install folder (`hide runtime prepare`; PRD
   * auto-update D-10), learned once per launch. Null until then, and always
   * null elsewhere, where the package's own resources folder is used.
   */
  private runtimeDir: string | null = null;
  /** The login shell's PATH once it answered; a shell that failed or timed out is asked again on the next search. */
  private loginPath: string | null = null;
  private watch: NodeJS.Timeout | null = null;
  private quitting = false;
  /** The pages of browser displays; made once the app is ready, since a session needs it. */
  private browsers: BrowserViews | null = null;
  private browserCdp: BrowserCdpGateway | null = null;
  private browserControlAddress: { endpoint: string; token: string } | null = null;
  private browserControlRegistration: { daemon: Attached; credential: string } | null = null;
  private browserControlEpoch = 0;
  private browserControlAttempted = -1;
  /** One registration transaction at a time, with one attempt per attach transition. */
  private browserControlFlight: Promise<void> | null = null;
  private browserControlShutdown: Promise<void> | null = null;
  private browserControlClosed = false;
  /** Shown agent notifications, kept so their click still reaches the shell. */
  private readonly notifications = new KeptNotifications<Notification>();
  private readonly permission: NotificationPermissionReader;
  /** Notification clicks, held until the shell reports its count, which it does once it holds a live snapshot to open them in. */
  private readonly clicks = new ClickGate();
  /** The app icon's count as last shown. */
  private badge = 0;
  private overlay: NativeImage | null = null;
  private readonly editor: Editor;
  /** This app's own updates; made at start, since electron-updater needs the app ready. */
  private updates: AppUpdates | null = null;

  constructor(
    private readonly env: DesktopEnv,
    private readonly log: HostLog,
    /** `SHOW_INACTIVE_SWITCH` was passed: no window ever activates the app. */
    private readonly showInactive: boolean,
    /** The language the host's own words are drawn in. */
    private readonly language: HostLanguage,
  ) {
    this.permission = new NotificationPermissionReader(process.platform, path.dirname(process.execPath), log, () => Notification.isSupported());
    this.editor = new Editor(
      log,
      () => ({
        platform: process.platform,
        home: app.getPath("home"),
        // The folders a CLI child searches: what was inherited, the install folders, and the login shell's once it answered.
        searchPath: [...(this.childEnvironment().PATH ?? "").split(path.delimiter), ...(this.loginPath ?? "").split(path.delimiter)].filter(Boolean),
        exists: (file) => fs.existsSync(file),
        isExecutable,
      }),
      () => this.childEnvironment(),
    );
  }

  // --- lifecycle ------------------------------------------------------------

  start(): void {
    // A folder whose variable is unset is left out of the search and of every
    // child's PATH (`cli.ts`); the keys are named once here.
    const missing = [...new Set([...wellKnownDirs(this.env).missing, ...(this.env.path ? [] : systemDirs(this.env).missing)])];
    if (missing.length > 0) this.log.event("env.locations_missing", { keys: missing.join(",") });
    this.guardSession();
    this.browsers = new BrowserViews(
      this.log,
      (event) => this.fromShell(event),
      (workspace, id, load) => this.resolveBrowserRoute(workspace, id, load),
      (workspace, id, load) => this.releaseBrowserRoute(workspace, id, load),
      // A popup that may not take the keyboard is ordered in without
      // activating the app, behind the operator's windows under the test switch.
      (window, focus) => (focus || this.showInactive ? this.present(window, focus) : window.showInactive()),
      () => this.language.t,
    );
    this.startBrowserControl(this.browsers);
    this.listenReveal();
    this.listenPickFolder();
    this.listenHerdrRestart();
    this.listenLocalPaths();
    this.listenNotify();
    this.listenWindowChrome();
    this.listenEditor();
    // An optional feature: a failure to set it up is the log's, and the window opens regardless.
    try {
      this.startUpdates();
    } catch (error) {
      this.log.event("update.start_failed", { detail: error instanceof Error ? error.message : String(error) });
    }
    this.openWindow();
    if (process.platform === "win32") this.listenToastActivation();
    void this.discover("launch");
  }

  /** A second launch or a Dock click: bring the window back, and look again when nothing is attached. */
  reopen(trigger: "second-instance" | "activate"): void {
    this.log.event("host.reopen", { trigger, state: this.state.kind, had_window: this.window !== null });
    if (!this.window) this.openWindow();
    else {
      if (this.window.isMinimized()) this.window.restore();
      this.present(this.window, true);
    }
    if (this.state.kind === "failed" || this.state.kind === "lost") void this.discover(trigger);
  }

  quit(): void {
    if (this.quitting) return;
    this.quitting = true;
    this.stopWatch();
    this.runner.stop();
    this.permission.stop();
    this.editor.stop();
    this.updates?.stop();
    // Revoke sockets and debugger leases immediately; hold quit only for
    // the bounded authenticated unregister, never for the operator daemon.
    const close = this.browserCdp?.close() ?? Promise.resolve();
    this.browserControlShutdown = Promise.all([close, this.browserControlFlight]).then(async () => {
      const registration = this.browserControlRegistration;
      this.browserControlRegistration = null;
      if (registration) await this.unregisterBrowserControl(registration);
    });
    this.log.event("host.quit", { state: this.state.kind });
  }

  /**
   * The stored macOS pane chords the shell reports, for the menu. Only this
   * window's page on the daemon origin is heard; anything else is logged and
   * dropped.
   */
  setBrowserRegistry(registry: readonly import("../../../web/src/shortcuts").Command[]): void {
    this.browsers?.setRegistry(registry);
  }

  listenBindings(apply: (reported: unknown) => void): void {
    ipcMain.on(BINDINGS_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("bindings.refused", { reason: "sender" });
        return;
      }
      apply(reported);
    });
  }

  /**
   * The core's explicit interface language, which the shell reports once a
   * snapshot confirms it (null: the system's). Only this window's page on the
   * daemon origin is heard; `apply` runs when the language in effect changed.
   */
  listenLanguage(apply: () => void): void {
    ipcMain.on(LANGUAGE_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("language.refused", { reason: "sender" });
        return;
      }
      const outcome = this.language.report(reported);
      if (outcome === "refused") this.log.event("language.refused", { reason: "value" });
      else if (outcome === "changed") apply();
    });
  }

  /**
   * A menu's `reveal_external` (issue 324) on an Explorer, History, View tab
   * or sidebar row: the OS file manager selects the file or folder in its
   * parent folder and opens nothing, so no program on this computer starts
   * from it. Only this window's page on the daemon origin is heard, and only
   * an absolute path that exists; the log records the outcome, never the path.
   */
  private listenReveal(): void {
    ipcMain.on(REVEAL_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("reveal.refused", { reason: "sender" });
        return;
      }
      void revealTarget(reported).then((target) => {
        if ("refused" in target) {
          this.log.event("reveal.refused", { reason: target.refused });
          return;
        }
        shell.showItemInFolder(target.path);
        this.log.event("reveal.shown", { kind: target.kind });
      }).catch(() => this.log.event("reveal.failed", {}));
    });
  }

  /**
   * System notifications: only this window's page on the
   * daemon origin is heard. A click brings the window forward and hands the
   * item's id to the shell, which opens it; the log records the outcome,
   * never the words.
   */
  private listenNotify(): void {
    ipcMain.on(NOTIFY_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("notify.refused", { reason: "sender" });
        return;
      }
      const request = notifyRequest(reported);
      if ("refused" in request) {
        this.log.event("notify.refused", { reason: request.refused });
        return;
      }
      if (!Notification.isSupported()) {
        this.log.event("notify.unsupported", {});
        return;
      }
      // An agent's notification waits for the operator to look away (D-10);
      // a Hide popup window with the keyboard counts as Hide.
      if (request.unlessFocused && BrowserWindow.getFocusedWindow() !== null) {
        this.log.event("notify.suppressed", { reason: "focused" });
        return;
      }
      // A newer notification for the same id replaces the one shown (B35).
      const replaced = this.notifications.take(request.id);
      if (replaced) {
        replaced.removeAllListeners();
        replaced.close();
      }
      // Windows takes the toast Hide writes, so a click after Hide quit still names it (`toastXml`).
      const shown = new Notification(process.platform === "win32" ? { toastXml: toastXml(request) } : { title: request.title, body: request.body });
      shown.on("click", () => {
        this.notifications.forget(request.id);
        this.log.event("notify.clicked", {});
        this.openNotification(request.id);
      });
      shown.on("close", () => this.notifications.forget(request.id));
      shown.on("show", () => this.permission.showSucceeded());
      // The log records what the system said, never the words shown. Linux
      // fails within `show()`, so a failed notification is not also logged shown.
      let failed = false;
      shown.on("failed", (_event, error) => {
        failed = true;
        this.notifications.forget(request.id);
        this.permission.showFailed();
        this.log.event("notify.failed", { detail: error });
      });
      this.notifications.keep(request.id, shown)?.removeAllListeners("click");
      shown.show();
      if (!failed) this.log.event("notify.shown", { replaced: replaced !== null });
    });
    ipcMain.on(NOTIFY_WITHDRAW_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("notify.withdraw_refused", { reason: "sender" });
        return;
      }
      const ids = withdrawRequest(reported);
      if ("refused" in ids) {
        this.log.event("notify.withdraw_refused", { reason: ids.refused });
        return;
      }
      let closed = 0;
      for (const id of ids) {
        const shown = this.notifications.take(id);
        if (!shown) continue;
        shown.removeAllListeners();
        shown.close();
        closed += 1;
      }
      this.log.event("notify.withdrawn", { asked: ids.length, closed });
    });
    ipcMain.handle(NOTIFY_PERMISSION_CHANNEL, (event: IpcMainInvokeEvent) => {
      if (!this.fromShell(event)) {
        this.log.event("notify.permission_refused", { reason: "sender" });
        return "unknown";
      }
      return this.permission.read();
    });
    ipcMain.on(NOTIFY_BADGE_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("notify.badge_refused", { reason: "sender" });
        return;
      }
      const count = badgeRequest(reported);
      if (typeof count !== "number") {
        this.log.event("notify.badge_refused", { reason: count.refused });
        return;
      }
      if (count !== this.badge) this.showBadge(count);
      const held = this.clicks.shellLive();
      if (held !== null) event.sender.send(NOTIFY_OPEN_CHANNEL, held);
    });
    ipcMain.on(NOTIFY_SETTINGS_CHANNEL, (event: IpcMainEvent) => {
      if (!this.fromShell(event)) {
        this.log.event("notify.settings_refused", { reason: "sender" });
        return;
      }
      if (process.platform !== "darwin") return;
      shell.openExternal(NOTIFICATION_SETTINGS_URL).then(
        () => this.log.event("notify.settings_opened", {}),
        (error: unknown) => this.log.event("notify.settings_failed", { detail: String(error) }),
      );
    });
  }

  /**
   * A notification the operator clicked: the window comes forward and the
   * shell opens what the id names. A click before the shell holds a live
   * snapshot (one that started Hide, or while the status page shows or the
   * daemon is lost) waits for the shell's next count (`ClickGate`).
   */
  private openNotification(id: string): void {
    const window = this.window;
    if (!window || window.isDestroyed()) return;
    if (window.isMinimized()) window.restore();
    this.present(window, true);
    const now = this.clicks.click(id);
    if (now !== null) window.webContents.send(NOTIFY_OPEN_CHANNEL, now);
  }

  /**
   * Windows: a click on a toast in Action Center after Hide quit starts Hide
   * through the toast activator Electron registers, and reaches only this
   * callback, which Electron also calls at once for a click that came before
   * it was set (`Notification.handleActivation`). Electron makes the
   * activator, and the Start Menu shortcut that ties Hide's AppUserModelID to
   * it, when it first builds its notification presenter, on a thread-pool
   * task (`shell/browser/notifications/win/windows_toast_activator.cc`,
   * `RegisterActivator`). Building it here, at launch, starts that task long
   * before the first toast, which could otherwise show before the shortcut
   * exists and vanish, though nothing waits for it, and registers the
   * activator in this process, so a click while Hide runs reaches it rather
   * than starting a second instance that the single-instance lock would end.
   * Not on macOS, where building the presenter asks for permission, which
   * the wizard's button does (D-02).
   */
  private listenToastActivation(): void {
    this.log.event("notify.presenter", { supported: Notification.isSupported() });
    Notification.handleActivation((details) => {
      const id = launchedId(details.arguments);
      this.log.event("notify.activated", { type: details.type, named: id !== null });
      if (id !== null) this.openNotification(id);
    });
  }

  /**
   * The window frame the shell's top row is drawn inside (PRD
   * sidebar-titlebar-refresh D-15, D-17): asked once by the shell and sent on
   * every fullscreen change. Only this window's page on the daemon origin is
   * answered or told; the status page draws its own drag strip.
   */
  private listenWindowChrome(): void {
    ipcMain.handle(WINDOW_CHROME_GET_CHANNEL, (event: IpcMainInvokeEvent) => {
      if (!this.fromShell(event) || !this.window) {
        this.log.event("window_chrome.refused", { reason: "sender" });
        return null;
      }
      return windowChrome(process.platform, this.window.isFullScreen());
    });
  }

  /**
   * This app's own updates (PRD auto-update): the updater, the status the
   * shell draws, and the shell's check, Restart and preview switch. Only this
   * window's page on the daemon origin is heard or told; a browser tab or the
   * phone has no bridge, so it shows none of it (B24).
   */
  private startUpdates(): void {
    const version = app.getVersion();
    const resources = process.resourcesPath;
    const system = updateSystem({
      packaged: app.isPackaged,
      version,
      platform: process.platform,
      packageType: process.platform === "linux" ? readPackageType(resources) : null,
      macosLocation: process.platform === "darwin" ? macosLocation(path.resolve(process.execPath, "..", "..", ".."), isWritable) : "ok",
    });
    const file = updatesFile(app.getPath("userData"));
    const updates = new AppUpdates(
      system,
      version,
      typeof system === "string" ? this.createUpdater(system) : null,
      {
        releases: () => fetchReleases((url, init) => net.fetch(url, init)),
        now: Date.now,
        later: (run, ms) => {
          const timer = setTimeout(run, ms);
          return () => clearTimeout(timer);
        },
        savePrerelease: (on) => savePrerelease(file, on),
      },
      this.log,
      { prerelease: readPrerelease(file), feedOverride: this.env.updateFeedUrl },
    );
    this.updates = updates;
    updates.onChange((status) => {
      const window = this.window;
      if (window && !window.isDestroyed() && this.isDaemonUrl(window.webContents.getURL())) window.webContents.send(UPDATE_STATUS_CHANNEL, status);
    });
    ipcMain.handle(UPDATE_STATUS_GET_CHANNEL, (event: IpcMainInvokeEvent) => (this.fromShell(event) ? updates.status() : null));
    ipcMain.on(UPDATE_CHECK_CHANNEL, (event: IpcMainEvent) => {
      if (this.fromShell(event)) void updates.check("operator");
    });
    ipcMain.on(UPDATE_RESTART_CHANNEL, (event: IpcMainEvent) => {
      if (this.fromShell(event)) updates.restart();
    });
    ipcMain.on(UPDATE_PRERELEASE_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (this.fromShell(event) && typeof reported === "boolean") updates.setPrerelease(reported);
    });
    updates.start();
  }

  /** electron-updater's updater for this system, its own log lines routed to the host log. */
  private createUpdater(system: UpdateSystem): UpdaterLike {
    const updater = system === "macos" ? new MacUpdater() : system === "windows" ? new NsisUpdater() : system === "deb" ? new DebUpdater() : new RpmUpdater();
    const record = (level: string) => (message: unknown) => this.log.event("update.library", { level, detail: String(message).slice(0, 500) });
    updater.logger = { info: () => undefined, debug: () => undefined, warn: record("warn"), error: record("error") };
    return updater;
  }

  /** Tells the shell in `window` that the window entered or left fullscreen. */
  private sendWindowChrome(window: BrowserWindow): void {
    if (window.isDestroyed() || !this.isDaemonUrl(window.webContents.getURL())) return;
    window.webContents.send(WINDOW_CHROME_CHANNEL, windowChrome(process.platform, window.isFullScreen()));
  }

  /**
   * Open in VS Code (PRD sidebar-titlebar-refresh D-13, D-14, B15, B16):
   * whether VS Code is installed, found once per launch, and a folder handed
   * to it. Only this window's page on the daemon origin is heard; a refused
   * or failed open shows nothing, and the log records the outcome, never the
   * path (`editor.ts`).
   */
  private listenEditor(): void {
    ipcMain.handle(EDITOR_AVAILABLE_CHANNEL, (event: IpcMainInvokeEvent) => {
      if (!this.fromShell(event)) {
        this.log.event("editor.refused", { reason: "sender" });
        return false;
      }
      return this.editor.available();
    });
    ipcMain.on(EDITOR_OPEN_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("editor.refused", { reason: "sender" });
        return;
      }
      this.editor.open(reported).catch(() => this.log.event("editor.failed", { reason: "unexpected" }));
    });
  }

  /**
   * The app icon's count of agents that need the operator, zero clearing it.
   * macOS draws it on the Dock once it lets Hide notify and Linux where the
   * desktop takes the LauncherEntry API, both through `app.setBadgeCount`;
   * Windows has no app badge, so the taskbar button wears an amber dot,
   * with the count in words for assistive technology, as VS Code does
   * (`setOverlayIcon`).
   */
  private showBadge(count: number): void {
    this.badge = count;
    if (process.platform === "win32") {
      const window = this.window;
      if (!window || window.isDestroyed()) return;
      this.overlay ??= nativeImage.createFromBitmap(overlayDot(__HIDE_WARNING__), { width: OVERLAY_SIZE, height: OVERLAY_SIZE });
      window.setOverlayIcon(count > 0 ? this.overlay : null, count > 0 ? this.language.t("overview.needsYou", { count }) : "");
      this.log.event("notify.badge", { count });
    } else if (process.platform === "linux") {
      // False where the desktop has no launcher that takes counts.
      this.log.event("notify.badge", { count, shown: app.setBadgeCount(count) });
    } else {
      app.setBadgeCount(count);
      this.log.event("notify.badge", { count });
    }
  }

  /**
   * Add a project's Browse folder: the system's own folder picker, a sheet on
   * this window on macOS, which can also make a new folder. Only this window's
   * page on the daemon origin is answered; a refused sender and a cancelled
   * pick both answer null, the chosen folder is answered in the wire spelling
   * (`wirePath.ts`) or refused with the reason when it has none, and hided
   * judges it like any other path. The log records the outcome, never the path.
   */
  /**
   * Restarting the operator's Herdr server on this app's Herdr (PRD
   * herdr-takeover B12, B13), asked by this window's shell after the operator
   * confirmed what ends; one runs at a time and a second ask answers with it.
   * Only a packaged app has a Herdr of its own to restart onto. A server that
   * does not come back shows the status page's start failure, with Retry and
   * Copy diagnostics, as a launch whose server does not start would.
   */
  private listenHerdrRestart(): void {
    ipcMain.handle(HERDR_RESTART_CHANNEL, (event: IpcMainInvokeEvent) => {
      if (!this.fromShell(event)) {
        this.log.event("herdr.restart_refused", { reason: "sender" });
        return "refused";
      }
      this.herdrRestart ??= this.restartHerdr().finally(() => {
        this.herdrRestart = null;
      });
      return this.herdrRestart;
    });
  }

  private async restartHerdr(): Promise<HerdrRestart> {
    const herdr = this.herdr();
    if (herdr.source !== "bundled" || herdr.path === null) {
      this.log.event("herdr.restart_unavailable", { source: herdr.source });
      return "unavailable";
    }
    const bin = herdr.path;
    const env = serverEnvironment(this.childEnvironment());
    const attempt = this.attempts;
    const result = await restartServer({
      stop: async () => {
        const stopped = await this.runChild(bin, ["server", "stop"], STATUS_TIMEOUT_MS, env);
        if (stopped.spawnError) return { failed: `herdr could not start: ${stopped.spawnError}` };
        if (stopped.timedOut) return { failed: "herdr server stop timed out" };
        return stopped.code === 0 ? { ok: true } : { failed: `herdr server stop exited ${stopped.code ?? stopped.signal ?? "without a code"}` };
      },
      status: (timeoutMs) => this.runChild(bin, ["status", "server", "--json"], timeoutMs, env).then(parseServerStatus),
      start: () => startDetached(bin, ["server"], env),
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      now: Date.now,
      stopped: () => this.quitting,
      statusTimeoutMs: STATUS_TIMEOUT_MS,
      waitMs: HERDR_START_WAIT_MS,
      pollMs: HERDR_START_POLL_MS,
    });
    if (result.outcome === "restarted") {
      this.log.event("herdr.server_restarted", { attempt, pid: result.pid, elapsed_ms: result.elapsedMs });
      return "restarted";
    }
    if (result.outcome === "stop_failed") {
      this.log.event("herdr.server_stop_failed", { attempt, detail: result.detail });
      return "stop_failed";
    }
    this.log.event("herdr.server_restart_failed", { attempt, outcome: result.outcome, detail: result.detail });
    this.fail(attempt, "start_failed", `herdr restart: ${result.detail}`);
    return "failed";
  }

  private listenPickFolder(): void {
    ipcMain.handle(PICK_FOLDER_CHANNEL, async (event: IpcMainInvokeEvent) => {
      if (!this.fromShell(event) || !this.window) {
        this.log.event("pick_folder.refused", { reason: "sender" });
        return null;
      }
      const t = this.language.t;
      const picked = await dialog.showOpenDialog(this.window, { title: t("native.folderPicker.title"), buttonLabel: t("native.folderPicker.button"), properties: ["openDirectory", "createDirectory"] });
      const folder = picked.canceled ? null : (picked.filePaths[0] ?? null);
      if (folder === null) {
        this.log.event("pick_folder.answered", { picked: false });
        return null;
      }
      try {
        const wire = toPage(folder);
        this.log.event("pick_folder.answered", { picked: true });
        return wire;
      } catch (error) {
        if (!(error instanceof WirePathError)) throw error;
        // The page's diagnostic says the picker failed, with the reason and not the path.
        this.log.event("pick_folder.refused", { reason: error.reason });
        throw error;
      }
    });
  }

  /**
   * Terminal links on this computer: a probe answers which of the named paths
   * exist and what they are, and an open hands one to the system, in its
   * default application or as a folder window, or revealed in the file manager
   * when opening would run it (`localPath.ts`). The page names and is
   * answered paths in the wire spelling (`wirePath.ts`). Only this window's
   * page on the daemon origin is heard; a probe is not logged, since it runs
   * on hover, and an open logs its route and never the path.
   */
  private listenLocalPaths(): void {
    ipcMain.handle(PROBE_PATHS_CHANNEL, async (event: IpcMainInvokeEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("probe_paths.refused", { reason: "sender" });
        return [];
      }
      const paths = probeRequest(reported);
      if (paths === null) {
        this.log.event("probe_paths.refused", { reason: "paths" });
        return [];
      }
      return probe(paths);
    });
    ipcMain.on(OPEN_PATH_CHANNEL, (event: IpcMainEvent, reported: unknown) => {
      if (!this.fromShell(event)) {
        this.log.event("open_path.refused", { reason: "sender" });
        return;
      }
      const target = fromPage(reported);
      if (target === null) {
        this.log.event("open_path.refused", { reason: "path" });
        return;
      }
      void openRoute(target).then(async (route) => {
        if (route.action === "refuse") {
          this.log.event("open_path.refused", { reason: route.reason });
          return;
        }
        if (route.action === "reveal") shell.showItemInFolder(target);
        else {
          // Judged a moment ago; a link swapped in since is refused, not followed.
          if ((await fs.promises.realpath(target).catch(() => null)) !== target) {
            this.log.event("open_path.refused", { reason: "not_physical" });
            return;
          }
          const failure = await shell.openPath(target);
          if (failure) {
            this.log.event("open_path.failed", { kind: route.kind });
            return;
          }
        }
        this.log.event(`open_path.${route.action}`, { kind: route.kind, reason: route.reason });
      }).catch(() => this.log.event("open_path.failed", {}));
    });
  }

  /** An app-menu click, delivered to the shell only while it is loaded. */
  sendCommand(id: MenuCommand): void {
    // Check for Updates… looks whatever the window shows; the shell opens About for its outcome.
    if (id === CHECK_UPDATES_COMMAND) void this.updates?.check("operator");
    const command = id === SETUP_COMMAND || id === CHECK_UPDATES_COMMAND ? null : id;
    // A page's popup window holding the keyboard answers the command itself,
    // whatever the shell's state.
    if (command && this.browsers?.popupCommand(command)) return;
    if (!this.window || (this.state.kind !== "attached" && this.state.kind !== "lost")) return;
    // Text size in a page is the page's zoom, and the page keeps the keyboard.
    if (command && this.browsers?.zoomFocused(command)) return;
    // A chord the page of a browser display left unhandled reaches the menu;
    // the command's surface (the palette, a dialog) needs the keyboard.
    if (this.browsers?.hasFocus()) this.window.webContents.focus();
    this.window.webContents.send(COMMAND_CHANNEL, id);
  }

  // --- discovery ------------------------------------------------------------

  discover(trigger: string): Promise<void> {
    this.discovering ??= this.runDiscovery(trigger).finally(() => {
      this.discovering = null;
    });
    return this.discovering;
  }

  private async runDiscovery(trigger: string): Promise<void> {
    const attempt = ++this.attempts;
    this.stopWatch();
    this.setState({ kind: "connecting" });
    const runtime = await this.prepareRuntime(attempt);
    if (runtime !== null) return this.fail(attempt, "start_failed", `runtime: ${runtime}`);
    if (this.quitting) return;
    const herdr = this.herdr();
    this.log.event("discovery.start", { attempt, trigger, herdr: herdr.source, replaced_pane_herdr: herdr.replacedPaneValue ?? undefined });
    const cli = await this.findCli(attempt);
    if (!cli) return this.fail(attempt, "cli_missing", "no executable hide CLI");
    await this.ensureHerdrServer(attempt);
    if (this.quitting) return;
    const connected = await this.runCli(cli.path, ["connect"], CONNECT_TIMEOUT_MS);
    for (const record of cliRecords(connected.stderr)) this.log.event(record.event, { attempt, ...record.fields });
    const answer = parseConnect(connected);
    if (this.quitting) return;
    if (answer.kind === "failed") return this.fail(attempt, answer.reason, answer.detail, answer);
    this.log.event("discovery.attached", { attempt, port: answer.port, pid: answer.pid });
    this.remember(cli);
    this.attach(answer);
  }

  private async findCli(attempt: number): Promise<{ path: string; source: CliSource } | null> {
    const stored = parseRememberedCli(readJsonFile(rememberedCliPath(app.getPath("userData"))));
    if (stored !== null && typeof stored === "object") this.log.event("cli.remembered_unreadable", { attempt, detail: stored.unreadable });
    const resolved = await resolveCli(
      {
        override: this.env.cliPath,
        worktreeRoot: app.isPackaged ? null : path.resolve(app.getAppPath(), ".."),
        bundledDir: this.bundledDir(),
        searchPath: this.env.path,
        remembered: typeof stored === "string" ? stored : null,
        // A packaged app opened from Finder has launchd's PATH, not the operator's.
        loginPath: app.isPackaged && HAS_LOGIN_SHELL ? () => this.readLoginPath(attempt) : null,
        wellKnown: wellKnownDirs(this.env).dirs,
      },
      { isExecutable, mtimeMs },
    );
    this.log.event(resolved.found ? "cli.resolved" : "cli.missing", {
      attempt,
      source: resolved.found?.source,
      path: resolved.found?.path,
      tried: resolved.tried.join(path.delimiter),
    });
    this.cli = resolved.found;
    return this.cli;
  }

  private async readLoginPath(attempt: number): Promise<string | null> {
    if (this.loginPath) return this.loginPath;
    if (this.env.shell === null) {
      this.log.event("cli.login_path", { attempt, ok: false, reason: "no_shell" });
      return null;
    }
    const started = Date.now();
    const command = loginPathCommand(this.env.shell);
    const result = await this.runCli(command.file, command.args, LOGIN_PATH_TIMEOUT_MS);
    this.loginPath = parseLoginPath(result.stdout);
    this.log.event("cli.login_path", {
      attempt,
      ok: this.loginPath !== null,
      code: result.code,
      timed_out: result.timedOut,
      elapsed_ms: Date.now() - started,
    });
    return this.loginPath;
  }

  /** The CLI that just attached, kept for the next launch so it need not ask the login shell again. */
  private remember(cli: { path: string; source: CliSource }): void {
    if (!REMEMBERED_SOURCES.has(cli.source)) return;
    try {
      writeJsonFile(rememberedCliPath(app.getPath("userData")), rememberedCliValue(cli.path));
      this.log.event("cli.remembered", { source: cli.source, path: cli.path });
    } catch (error) {
      this.log.event("cli.remember_failed", { detail: String(error) });
    }
  }

  /**
   * The folder `hide`, `hided` and `herdr` run from in a packaged app: its
   * `Contents/Resources` or `resources`, or on Windows the build's runtime
   * copy once `prepareRuntime` found it.
   */
  private bundledDir(): string | null {
    return app.isPackaged ? (this.runtimeDir ?? process.resourcesPath) : null;
  }

  /**
   * A packaged Windows app's runtime copy (PRD auto-update D-10): the
   * installer stops every program under the install folder and Windows cannot
   * replace a running file, so the daemon and the Herdr server must not run
   * from there. Asked of the install folder's own `hide` once per launch;
   * what it could not remove of older copies goes to the log (D-11). Null
   * when the copy is ready or none is needed, else why it failed.
   */
  private async prepareRuntime(attempt: number): Promise<string | null> {
    if (!app.isPackaged || process.platform !== "win32" || this.runtimeDir !== null) return null;
    const started = Date.now();
    const result = await this.runChild(path.join(process.resourcesPath, executableFile("hide")), ["runtime", "prepare"], RUNTIME_PREPARE_TIMEOUT_MS, this.childEnvironment());
    for (const record of cliRecords(result.stderr)) this.log.event(record.event, { attempt, ...record.fields });
    const prepared = parseRuntimePrepare(result);
    if ("failed" in prepared) {
      this.log.event("runtime.prepare_failed", { attempt, detail: prepared.failed, elapsed_ms: Date.now() - started });
      return prepared.failed;
    }
    this.runtimeDir = prepared.dir;
    this.log.event("runtime.prepared", { attempt, dir: prepared.dir, elapsed_ms: Date.now() - started });
    return null;
  }

  /** The Herdr a daemon started from here attaches pane terminals with; `herdr.ts` owns the rule. */
  private herdr(): HerdrChoice {
    return chooseHerdr({ bundledDir: this.bundledDir(), herdrBinPath: this.env.herdrBinPath, herdrPaneId: this.env.herdrPaneId });
  }

  private childEnvironment(): Record<string, string | undefined> {
    const herdr = this.herdr().path;
    // Finder supplies only launchd's system PATH. The CLI and any daemon it
    // starts need the same install folders we search for `hide`.
    const inherited = this.env.path ? this.env.path.split(path.delimiter) : systemDirs(this.env).dirs;
    const searchPath = [...new Set([...inherited, ...wellKnownDirs(this.env).dirs].filter(Boolean))].join(path.delimiter);
    // Windows names variables without regard to case, so the inherited `Path`
    // goes and this PATH is the child's only one.
    const passed = Object.entries(this.env.inherited).filter(([key]) => process.platform !== "win32" || key.toUpperCase() !== "PATH");
    return { ...Object.fromEntries(passed), PATH: searchPath, ...(herdr ? { HERDR_BIN_PATH: herdr } : {}) };
  }

  /**
   * Starts the Herdr this app bundles when no server answers on the socket
   * its children would use, as `herdr` does when a terminal runs it: after a
   * reboot nothing else starts one, and hided only reads a missing server as
   * absent. A server that answers is never touched, and neither an unpackaged
   * host nor an explicit HERDR_BIN_PATH override starts anything, so a
   * development or e2e run keeps the server it made. Every failure goes to the
   * log and the connect goes ahead, where the shell shows Herdr unreachable.
   */
  private async ensureHerdrServer(attempt: number): Promise<void> {
    const herdr = this.herdr();
    if (herdr.source !== "bundled" || herdr.path === null) return;
    const bin = herdr.path;
    const env = serverEnvironment(this.childEnvironment());
    const result = await ensureServer({
      status: (timeoutMs) => this.runChild(bin, ["status", "server", "--json"], timeoutMs, env).then(parseServerStatus),
      start: () => startDetached(bin, ["server"], env),
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      now: Date.now,
      stopped: () => this.quitting,
      statusTimeoutMs: STATUS_TIMEOUT_MS,
      waitMs: HERDR_START_WAIT_MS,
      pollMs: HERDR_START_POLL_MS,
    });
    if (result.outcome === "status_failed") this.log.event("herdr.status_failed", { attempt, detail: result.detail });
    else if (result.outcome === "started") this.log.event("herdr.server_started", { attempt, pid: result.pid, elapsed_ms: result.elapsedMs });
    else if (result.outcome === "start_failed") this.log.event("herdr.server_start_failed", { attempt, pid: result.pid, detail: result.detail, elapsed_ms: result.elapsedMs });
  }

  private runChild(file: string, args: readonly string[], timeoutMs: number, env: Record<string, string | undefined>): Promise<ChildResult> {
    const run = this.cliChain.then(() =>
      this.quitting
        ? { code: null, signal: null, stdout: "", stderr: "", timedOut: false, spawnError: "host is quitting" }
        : this.runner.run(file, args, timeoutMs, env),
    );
    this.cliChain = run.catch(() => undefined);
    return run;
  }

  private runCli(file: string, args: readonly string[], timeoutMs: number): Promise<ChildResult> {
    // A call queued behind the one quit killed never starts; it answers as a
    // child that could not start, which every caller already reads as a stop.
    return this.runChild(file, args, timeoutMs, this.childEnvironment());
  }

  private fail(attempt: number, reason: FailureReason, detail: string, shown: { file?: string; builds?: CoreBuilds } = {}): void {
    if (this.quitting) return;
    const { file, builds } = shown;
    this.log.event("discovery.failed", { attempt, reason, detail, ...(file === undefined ? {} : { file }), ...(builds === undefined ? {} : builds) });
    this.setState({ kind: "failed", reason, ...(file === undefined ? {} : { file }), ...(builds === undefined ? {} : { builds }) });
  }

  private attach(daemon: Attached): void {
    this.setState({ kind: "attached", ...daemon });
    this.startHealthWatch();
  }

  // --- following the daemon (B4) ---------------------------------------------

  private stopWatch(): void {
    if (this.watch) clearTimeout(this.watch);
    this.watch = null;
  }

  private startHealthWatch(): void {
    let misses = 0;
    // The role the daemon's seat mounts (`/health` instance): a move or an update swaps it on the same port and
    // token, and the browser gateway this window registered went with the role before (PRD core-host-node-move).
    let instance: unknown;
    const tick = async () => {
      const current = this.state;
      if (current.kind !== "attached" || this.quitting) return;
      let healthy = false;
      let failure: ReturnType<typeof healthFailure> = null;
      let mounted: unknown;
      try {
        const answer = await fetch(`${current.origin}/health`, { signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS) });
        healthy = answer.ok;
        if (healthy) {
          const health: unknown = await answer.json().catch(() => null);
          failure = healthFailure(health);
          mounted = (health as { instance?: unknown } | null)?.instance;
        }
      } catch {
        healthy = false;
      }
      if (this.state !== current) return;
      if (mounted !== undefined) {
        if (instance !== undefined && mounted !== instance) {
          this.log.event("daemon.role_changed", { port: current.port });
          this.browserControlChanged();
        }
        instance = mounted;
      }
      // A node whose core is newer than this app, or whose update of its core failed, leaves its shell for the
      // status page; Retry is the next connection, with its one attempt (PRD core-host-node-move B10, B20).
      if (failure) return this.fail(this.attempts, failure.reason, "the node's core link", { builds: failure.builds });
      misses = healthy ? 0 : misses + 1;
      if (misses >= LOST_AFTER_MISSES) {
        this.log.event("daemon.lost", { port: current.port, pid: current.pid });
        // The shell keeps its own disconnected state on screen; nothing is reloaded here.
        this.enter({ ...current, kind: "lost" });
        this.browserControlChanged();
        this.startLostPoll();
        return;
      }
      this.watch = setTimeout(tick, HEALTH_INTERVAL_MS);
    };
    this.stopWatch();
    this.watch = setTimeout(tick, HEALTH_INTERVAL_MS);
  }

  private startLostPoll(): void {
    const tick = async () => {
      const lost = this.state;
      if (lost.kind !== "lost" || this.quitting || !this.cli) return;
      const status = parseStatus(await this.runCli(this.cli.path, ["status", "--json"], STATUS_TIMEOUT_MS));
      if (this.state !== lost) return;
      if (status?.running) {
        this.log.event("daemon.found", { port: status.port, pid: status.pid, same_url: status.url === lost.url });
        if (status.url === lost.url) {
          // The same daemon answered again; the shell reconnects on its own.
          this.enter({ ...lost, kind: "attached" });
          this.browserControlChanged();
          this.startHealthWatch();
        } else {
          this.attach(status);
        }
        return;
      }
      this.watch = setTimeout(tick, LOST_POLL_MS);
    };
    this.stopWatch();
    this.watch = setTimeout(tick, LOST_POLL_MS);
  }

  // --- the window -------------------------------------------------------------

  private setState(next: HostState): void {
    this.enter(next);
    this.browserControlChanged();
    this.render();
  }

  /**
   * Every state change goes through here. Outside `attached` the shell has
   * no live snapshot: a count it can no longer vouch for leaves the icon,
   * and a click waits for its next report, which comes once it is live again.
   */
  private enter(next: HostState): void {
    this.state = next;
    if (next.kind === "attached") return;
    this.clicks.shellGone();
    if (this.badge !== 0) this.showBadge(0);
  }

  // --- scoped browser automation ---------------------------------------------

  private startBrowserControl(browsers: BrowserViews): void {
    const chromeVersion = process.versions.chrome;
    if (!chromeVersion) { this.log.event("browser.control_start_failed", { reason: "chromium_version" }); return; }
    const gateway = new BrowserCdpGateway({
      pages: () => browsers.cdpPages(), changed: (listener) => browsers.observeCdp(listener),
      incarnation: (scope) => browsers.cdpIncarnation(scope),
      attached: (page, attached) => browsers.cdpAttached(page, attached),
      action: (scope, action, signal) => this.browserControlAction(scope, action, signal),
      chromeVersion,
      log: (event, fields) => this.log.event(event, fields),
    });
    this.browserCdp = gateway;
    void gateway.start().then((address) => {
      if (this.quitting) { void gateway.close(); return; }
      this.browserControlAddress = address;
      this.syncBrowserControl();
    }).catch(() => this.log.event("browser.control_start_failed", { reason: "listen" }));
    // index.ts calls quit first. This listener lets its cleanup settle before
    // Electron exits, and then the second before-quit proceeds normally.
    app.on("before-quit", (event) => {
      if (this.browserControlClosed) return;
      event.preventDefault();
      this.quit();
      void this.browserControlShutdown?.finally(() => { this.browserControlClosed = true; app.quit(); });
    });
  }

  private browserControlChanged(): void {
    this.browserControlEpoch++;
    this.browserCdp?.setAvailable(false);
    this.syncBrowserControl();
  }

  private syncBrowserControl(): void {
    const address = this.browserControlAddress;
    if (!address || this.quitting || this.browserControlFlight || this.browserControlAttempted === this.browserControlEpoch) return;
    const epoch = this.browserControlEpoch;
    this.browserControlAttempted = epoch;
    const current = this.state;
    const registration = this.browserControlRegistration;
    this.browserControlRegistration = null;
    this.browserControlFlight = (async () => {
      if (registration) await this.unregisterBrowserControl(registration);
      if (this.quitting || epoch !== this.browserControlEpoch || current.kind !== "attached") return;
      const credential = new URLSearchParams(new URL(current.url).hash.slice(1)).get("token");
      if (!credential) { this.log.event("browser.control_register_failed", { reason: "credential" }); return; }
      try {
        const response = await fetch(`${current.origin}/browser-control`, {
          method: "POST", headers: { Authorization: `Bearer ${credential}`, "Content-Type": "application/json" },
          body: JSON.stringify({ owner_pid: process.pid, ...address }), signal: AbortSignal.timeout(3_000),
        });
        await response.arrayBuffer();
        if (!response.ok) { this.log.event("browser.control_register_failed", { status: response.status }); return; }
        this.browserControlRegistration = { daemon: current, credential };
        if (!this.quitting && epoch === this.browserControlEpoch) this.browserCdp?.setAvailable(true);
        this.log.event("browser.control_registered", { owner_pid: process.pid });
      } catch { this.log.event("browser.control_register_failed", { reason: "transport" }); }
    })().finally(() => {
      this.browserControlFlight = null;
      if (epoch !== this.browserControlEpoch) this.syncBrowserControl();
    });
  }

  private async unregisterBrowserControl(registration: { daemon: Attached; credential: string }): Promise<void> {
    const address = this.browserControlAddress;
    if (!address) return;
    try {
      const response = await fetch(`${registration.daemon.origin}/browser-control`, {
        method: "DELETE", headers: { Authorization: `Bearer ${registration.credential}`, "Content-Type": "application/json" },
        body: JSON.stringify({ owner_pid: process.pid, ...address }), signal: AbortSignal.timeout(3_000),
      });
      await response.arrayBuffer();
      if (!response.ok) this.log.event("browser.control_unregister_failed", { status: response.status });
    } catch { this.log.event("browser.control_unregister_failed", { reason: "transport" }); }
  }

  private async browserControlAction(scope: CdpScope, action: CdpAction, signal: AbortSignal): Promise<CdpActionResult> {
    const current = this.state;
    const registration = this.browserControlRegistration;
    if (current.kind !== "attached" || !registration || registration.daemon.url !== current.url || this.quitting) throw new Error("Desktop browser is disconnected");
    const at = scope.workspace.indexOf("\u0000");
    let response: Response;
    try { response = await fetch(`${current.origin}/browser-control/action`, {
      method: "POST", headers: { Authorization: `Bearer ${registration.credential}`, "Content-Type": "application/json" },
      body: JSON.stringify({ device_id: scope.workspace.slice(0, at), checkout_path: scope.workspace.slice(at + 1), area_id: scope.area_id, ...action, owner_pid: process.pid }),
      signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
    }); } catch {
      this.log.event("browser.control_action_failed", { action: action.action, reason: "transport", request_id: action.request_id });
      throw new CdpActionUncertain();
    }
    let value: unknown;
    try { value = await response.json(); } catch {
      this.log.event("browser.control_action_failed", { action: action.action, reason: "response", request_id: action.request_id });
      throw new CdpActionUncertain();
    }
    const result = parseBrowserControlResult(value);
    if (!response.ok || !result) {
      this.log.event("browser.control_action_failed", { action: action.action, status: response.status, request_id: action.request_id });
      if (response.status >= 500 || response.ok && !result) throw new CdpActionUncertain();
      throw new Error("Workspace browser action failed");
    }
    return result;
  }

  private render(): void {
    const window = this.window;
    if (!window) return;
    if (this.statusLoad) {
      // A local file, so this waits a few milliseconds.
      if (!this.renderWaits) {
        this.renderWaits = true;
        const rerender = () => {
          this.renderWaits = false;
          this.render();
        };
        this.statusLoad.then(rerender, rerender);
      }
      return;
    }
    const state = this.state;
    if (state.kind === "attached" || state.kind === "lost") {
      this.load(window.loadURL(state.url), state.kind);
      return;
    }
    const words = this.statusWords();
    if (this.statusPageBroken) {
      this.showPlainStatus(window, words, state);
      return;
    }
    const hash = state.kind === "failed" ? this.failedHash(state) : "connecting";
    if (this.statusScreen?.lang === words.lang && !this.statusScreen.plain) {
      // Already on the status page in this language: only its hash moves, which it follows itself.
      this.load(window.webContents.executeJavaScript(`location.hash = ${JSON.stringify(hash)}`), state.kind);
      return;
    }
    const loading = window.loadFile(STATUS_PAGE, { hash, query: words.query });
    this.statusLoad = loading;
    // A load settles for the window that started it: a closed window's answer says nothing of the one that replaced it.
    const mine = () => this.window === window && !window.isDestroyed();
    loading.then(
      () => {
        if (!mine()) return;
        this.statusLoad = null;
        this.statusScreen = { lang: words.lang, plain: false };
      },
      (error: unknown) => {
        if (!mine()) return;
        this.statusLoad = null;
        // A load a newer one replaced is not a failure; one that failed will fail again, so the plain screen takes its place.
        if ((error as { code?: string }).code === "ERR_ABORTED") return;
        this.statusPageBroken = true;
        this.statusScreen = null;
        this.log.event("status_page.unavailable", { file_exists: fs.existsSync(STATUS_PAGE), ...loadFailureFields(error) });
        this.render();
      },
    );
    this.load(loading, state.kind);
  }

  /**
   * The status screen when the status page file will not load: the same
   * sentences on a blank page (`statusPage.ts`), so a failure still shows its
   * reason and Retry instead of a blank window. It takes the window's one
   * navigation like the page does, and is loaded afresh each time so its
   * Retry link, a fragment, is a new navigation every time.
   */
  private showPlainStatus(window: BrowserWindow, words: { query: Record<string, string>; lang: string }, state: Exclude<HostState, { kind: "attached" } | { kind: "lost" }>): void {
    const screen = fallbackScreen(words.query, state.kind === "failed" ? { kind: "failed", reason: state.reason, file: state.file, ...this.failedSentences(state) } : state);
    const showing = (async () => {
      await window.loadURL("about:blank");
      await window.webContents.executeJavaScript(fallbackScript(screen, __HIDE_BACKGROUND__));
    })();
    this.statusLoad = showing;
    const mine = () => this.window === window && !window.isDestroyed();
    showing.then(
      () => {
        if (!mine()) return;
        this.statusLoad = null;
        this.statusScreen = { lang: words.lang, plain: true };
      },
      () => {
        if (mine()) this.statusLoad = null;
      },
    );
    this.load(showing, state.kind);
  }

  /**
   * The failure the status page shows, in its hash: the reason, the file a
   * refused start names, and for a core newer than this app or an update of
   * the core that failed its sentences with the core's machine and both
   * builds in them, written here because the catalogs fill them.
   */
  private failedHash(state: Extract<HostState, { kind: "failed" }>): string {
    const shown = new URLSearchParams({ failed: state.reason });
    if (state.file !== undefined) shown.set("file", state.file);
    const { said, builds } = this.failedSentences(state);
    if (said !== undefined) shown.set("said", said);
    if (builds !== undefined) shown.set("builds", builds);
    return shown.toString();
  }

  /** The sentences a failure with both builds adds to its reason; none for the other failures. */
  private failedSentences(state: Extract<HostState, { kind: "failed" }>): { said?: string; builds?: string } {
    if (state.builds === undefined) return {};
    const t = this.language.t;
    const said = state.reason === "update_failed" ? "native.status.updateFailed" : "native.status.coreNewer";
    return { said: t(said, { machine: state.builds.machine }), builds: t("native.status.builds", { core: state.builds.core, app: state.builds.app }) };
  }

  /**
   * The status page's sentences in the language in effect, passed in its
   * address so the catalogs stay the one source of the words. The page gets
   * the window's preload but `fromShell` refuses every message from it, so
   * words cannot reach it any other way and a language change reloads it
   * (`render` compares `lang`).
   */
  private statusWords(): { query: Record<string, string>; lang: string } {
    const t = this.language.t;
    const query = {
      lang: this.language.language,
      connecting: t("native.status.connecting"),
      failed: t("native.status.failed"),
      retry: t("common.retry"),
      copy: t("native.status.copyDiagnostics"),
      copied: t("native.status.copied"),
      cli_missing: t("native.status.cliMissing"),
      start_failed: t("native.status.startFailed"),
      no_response: t("native.status.noResponse"),
      other_build: t("native.status.otherBuild"),
      state_refused: t("native.status.stateRefused"),
      core_newer_title: t("native.status.coreNewerTitle"),
      update_failed_title: t("native.status.updateFailedTitle"),
      // macOS draws its window buttons over the page's top, so the page keeps a drag strip there.
      titlebar: hidesTitleBar(process.platform) ? "hidden" : "framed",
    };
    return { query, lang: query.lang };
  }

  /**
   * The failed status page's Copy diagnostics (PRD first-run-onboarding B2):
   * the app build, the reason code and where the host log is, written to
   * the clipboard here, since the page cannot reach it. The page then says it
   * was copied or, when the clipboard refused it, shows the same text to
   * select by hand.
   */
  private copyDiagnostics(contents: Electron.WebContents, hash: URLSearchParams): void {
    const reason = this.state.kind === "failed" ? this.state.reason : "none";
    const text = [`hide ${app.getVersion()} (${process.platform} ${process.arch}, Electron ${process.versions.electron})`, `reason: ${reason}`, `log: ${path.join(app.getPath("logs"), "desktop.log")}`].join("\n");
    hash.delete("copy");
    const answer = (key: "copied" | "diagnostics", value: string) => {
      hash.set(key, value);
      if (!contents.isDestroyed()) this.load(contents.executeJavaScript(`location.hash = ${JSON.stringify(hash.toString())}`), this.state.kind);
    };
    void (async () => {
      await clipboard.writeText(text);
      if ((await clipboard.readText()) !== text) throw new Error("the clipboard kept other text");
    })().then(
      () => {
        this.log.event("status.diagnostics_copied", { reason });
        answer("copied", "1");
      },
      (error: unknown) => {
        this.log.event("status.diagnostics_copy_failed", { reason, detail: String(error) });
        answer("diagnostics", text);
      },
    );
  }

  /** Draws the status page again in the language now in effect; the daemon's page follows the core itself and is left alone. */
  languageChanged(): void {
    if (this.statusScreen) this.render();
    // The taskbar overlay's words are the host's.
    if (process.platform === "win32" && this.badge > 0) this.showBadge(this.badge);
  }

  private load(pending: Promise<unknown>, state: HostState["kind"]): void {
    pending.catch((error: unknown) => {
      // A load a newer one replaced is not a failure.
      if ((error as { code?: string }).code === "ERR_ABORTED") return;
      this.log.event("window.load_failed", { state, ...loadFailureFields(error) });
    });
  }

  /**
   * Shows the window, and with `focus` gives it the keyboard. `show()` and
   * `focus()` activate the app, which takes the screen and the keyboard from
   * whatever the operator is using. Under `SHOW_INACTIVE_SWITCH` the window
   * is ordered in without activating the app (`showInactive()` alone would
   * still put it above every other window), then sent behind the operator's
   * windows (`blur()` orders it to the back on macOS).
   */
  private present(window: BrowserWindow, focus: boolean): void {
    if (this.showInactive) {
      window.showInactive();
      window.blur();
      return;
    }
    window.show();
    if (focus) window.focus();
  }

  private openWindow(): void {
    const file = windowStatePath(app.getPath("userData"));
    const displays = screen.getAllDisplays().map((display) => display.workArea);
    const restored = restoreBounds(readWindowState(file), displays, screen.getPrimaryDisplay().workArea);
    this.log.event("window.bounds", { source: restored.source, why: restored.why });
    const window = new BrowserWindow({
      ...restored.bounds,
      minWidth: MIN_SIZE.width,
      minHeight: MIN_SIZE.height,
      show: false,
      title: "hide",
      ...frameOptions(process.platform),
      backgroundColor: __HIDE_BACKGROUND__,
      webPreferences: {
        preload: path.join(__dirname, "preload.js"),
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        webSecurity: true,
        spellcheck: false,
      },
    });
    this.window = window;
    // Decoration failure is diagnostic only. Accept a fixed event from our
    // local status document, never arbitrary renderer text or personal data.
    window.webContents.on("console-message", (details) => {
      if (details.message === '{"event":"brand_owl.unavailable","scene":"startup","asset":"owl-roam"}'
        && window.webContents.getURL().startsWith(pathToFileURL(STATUS_PAGE).href + "?")) {
        this.log.event("brand_owl.unavailable", { scene: "startup", asset: "owl-roam" });
      }
    });
    this.browsers?.attach(window);
    window.once("ready-to-show", () => this.present(window, false));
    window.on("close", () => {
      try {
        writeWindowState(file, window.getNormalBounds());
      } catch (error) {
        this.log.event("window.state_write_failed", { detail: String(error) });
      }
    });
    window.on("closed", () => {
      if (this.window !== window) return;
      this.window = null;
      // What this window showed and was loading is gone with it; the next window starts from nothing.
      this.statusScreen = null;
      this.statusLoad = null;
    });
    window.on("enter-full-screen", () => this.sendWindowChrome(window));
    window.on("leave-full-screen", () => this.sendWindowChrome(window));
    this.guardNavigation(window);
    if (this.state.kind === "lost") {
      // A fresh page cannot load a daemon that is gone; show the search instead.
      void this.discover("window");
      return;
    }
    this.render();
  }

  // --- the security boundary (B10, B11) ----------------------------------------

  private daemonOrigin(): string | null {
    return this.state.kind === "attached" || this.state.kind === "lost" ? this.state.origin : null;
  }

  private browserRouteRequest(workspace: string, id: string, load: number): { current: Attached; token: string; body: string } {
    const at = workspace.indexOf("\u0000");
    if (at <= 0 || at === workspace.length - 1 || !id || !Number.isSafeInteger(load)) throw new Error("Invalid Browser Workspace");
    const current = this.state;
    if (current.kind !== "attached") throw new Error("Hide is disconnected");
    const token = new URLSearchParams(new URL(current.url).hash.slice(1)).get("token");
    if (!token) throw new Error("Hide credential is unavailable");
    return { current, token, body: JSON.stringify({ device_id: workspace.slice(0, at), checkout_path: workspace.slice(at + 1), id, load, owner_pid: process.pid }) };
  }

  private async resolveBrowserRoute(workspace: string, id: string, load: number): Promise<ResolvedPage> {
    const { current, token, body } = this.browserRouteRequest(workspace, id, load);
    const answer = await fetch(`${current.origin}/browser-route`, {
      method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body, signal: AbortSignal.timeout(20_000),
    });
    const result: unknown = await answer.json();
    if (!answer.ok) {
      const reason = result && typeof result === "object" && "reason" in result ? String(result.reason) : `HTTP ${answer.status}`;
      throw new Error(reason);
    }
    if (!result || typeof result !== "object" || !("url" in result) || !("source_url" in result) || !("load" in result)
      || typeof result.url !== "string" || typeof result.source_url !== "string" || result.load !== load) throw new Error("Invalid Browser route answer");
    return result as ResolvedPage;
  }

  private releaseBrowserRoute(workspace: string, id: string, load: number): void {
    let request: ReturnType<DesktopHost["browserRouteRequest"]>;
    try { request = this.browserRouteRequest(workspace, id, load); }
    catch { return; }
    const { current, token, body } = request;
    void fetch(`${current.origin}/browser-route`, {
      method: "DELETE", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body, signal: AbortSignal.timeout(3_000),
    }).then((answer) => {
      if (!answer.ok) this.log.event("browser.route_release_failed", { status: answer.status });
    }).catch((error: unknown) => this.log.event("browser.route_release_failed", { detail: String(error) }));
  }

  /** An IPC message from the shell the daemon serves in this window's own frame; the status page and every browser page are refused. */
  private fromShell(event: IpcMainEvent | IpcMainInvokeEvent): boolean {
    const frame = event.senderFrame;
    return this.window !== null && event.sender === this.window.webContents && frame !== null && frame.parent === null && this.isDaemonUrl(frame.url);
  }

  private isDaemonUrl(url: string): boolean {
    try {
      return new URL(url).origin === this.daemonOrigin();
    } catch {
      return false;
    }
  }

  private openExternal(url: string): void {
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      this.log.event("link.refused", { reason: "unparsable" });
      return;
    }
    if (!EXTERNAL_PROTOCOLS.has(parsed.protocol)) {
      this.log.event("link.refused", { protocol: parsed.protocol });
      return;
    }
    this.log.event("link.external", { protocol: parsed.protocol, host: parsed.host });
    shell.openExternal(url).catch((error: unknown) => this.log.event("link.open_failed", { detail: String(error) }));
  }

  private guardNavigation(window: BrowserWindow): void {
    const contents = window.webContents;
    // A new page is not the shell that reported until it reports itself.
    contents.on("did-start-navigation", (details) => {
      if (details.isMainFrame && !details.isSameDocument) {
        this.clicks.shellGone();
        this.statusScreen = null;
      }
    });
    contents.on("will-navigate", (event, url) => {
      if (this.isDaemonUrl(url)) return;
      event.preventDefault();
      this.openExternal(url);
    });
    contents.on("will-redirect", (event, url) => {
      if (this.isDaemonUrl(url)) return;
      event.preventDefault();
      this.log.event("navigation.redirect_refused", {});
    });
    contents.setWindowOpenHandler(({ url }) => {
      if (!this.isDaemonUrl(url)) this.openExternal(url);
      else this.log.event("window_open.refused", { reason: "daemon_origin" });
      return { action: "deny" };
    });
    // The status page asks for Retry and Copy diagnostics by moving its own
    // hash; the host takes no IPC from it (`fromShell`).
    contents.on("did-navigate-in-page", (_event, url) => {
      // The status page is a file; its plain form is about:blank (`statusPage.ts`).
      if (!url.startsWith("file:") && !url.startsWith("about:blank")) return;
      const hash = new URL(url).hash;
      if (hash === "#retry") void this.discover("retry");
      else if (new URLSearchParams(hash.slice(1)).has("copy")) this.copyDiagnostics(contents, new URLSearchParams(hash.slice(1)));
    });
  }

  private guardSession(): void {
    session.defaultSession.setPermissionRequestHandler((_contents, permission, callback, details) => {
      callback(this.isDaemonUrl(details.requestingUrl) && CLIPBOARD_PERMISSIONS.has(permission));
    });
    session.defaultSession.setPermissionCheckHandler((_contents, permission, origin) => {
      return origin === this.daemonOrigin() && CLIPBOARD_PERMISSIONS.has(permission);
    });
  }
}
